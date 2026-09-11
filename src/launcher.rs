//! Argument plumbing for a future native launcher, independent of session staging.
//!
//! Astral owns `--root`, `--work`, `--inspect`, and `--proxy` before the first
//! literal `--`. Unknown Codex options have unknown arity: if an option's value
//! equals an Astral option, put the whole Codex argument sequence after `--`.
//! The separator itself is consumed; subsequent tokens, including another `--`,
//! remain untouched. No model, permission, sandbox, or approval flags are added.

use crate::project::{Error, Result};
use serde::Serialize;
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::{Child, Command};

pub const MAX_ARGUMENTS: usize = 1_024;
pub const MAX_ARGUMENT_BYTES: usize = 65_536;
pub const MAX_TOTAL_ARGUMENT_BYTES: usize = 262_144;

fn error(code: &'static str, message: &str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

fn check_arguments(args: &[OsString]) -> Result<()> {
    if args.len() > MAX_ARGUMENTS {
        return Err(error("ARGUMENT_LIMIT_EXCEEDED", "too many arguments"));
    }
    let mut bytes = 0usize;
    for arg in args {
        let encoded = arg.as_encoded_bytes();
        if encoded.contains(&0) {
            return Err(error("INVALID_ARGUMENT", "arguments cannot contain NUL"));
        }
        bytes = bytes.saturating_add(encoded.len());
        if encoded.len() > MAX_ARGUMENT_BYTES || bytes > MAX_TOTAL_ARGUMENT_BYTES {
            return Err(error(
                "ARGUMENT_LIMIT_EXCEEDED",
                "argument byte limit exceeded",
            ));
        }
    }
    Ok(())
}

fn utf8(value: OsString, field: &str) -> Result<String> {
    value
        .into_string()
        .map_err(|_| error("NON_UTF8_ARGUMENT", field))
}

/// Splitting immediately after an ASCII option prefix preserves arbitrary OS bytes.
fn option_value(argument: &OsStr, prefix: &str) -> Option<OsString> {
    let remaining = argument
        .as_encoded_bytes()
        .strip_prefix(prefix.as_bytes())?;
    // SAFETY: prefix is supplied by this module's callers as complete ASCII text;
    // the split follows ASCII '=' at a valid boundary in OsStr's self-synchronizing
    // encoding. The remainder retains exactly the source platform's encoding.
    Some(unsafe { OsStr::from_encoded_bytes_unchecked(remaining) }.to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    Direct,
    Proxy,
}

#[derive(Debug, Clone)]
pub struct ProjectArguments {
    pub root: PathBuf,
    pub name: String,
    pub work: Option<String>,
    pub inspect: bool,
    pub route: Route,
    /// Raw user arguments. Never round-trip these through a shell or JSON preview.
    pub codex_args: Vec<OsString>,
}

impl ProjectArguments {
    pub fn parse(root: PathBuf, args: Vec<OsString>) -> Result<Self> {
        check_arguments(&args)?;
        let mut args = args.into_iter();
        let name = utf8(
            args.next()
                .ok_or_else(|| error("CLI_USAGE", "project requires NAME"))?,
            "project name must be UTF-8",
        )?;
        if name.is_empty() || name.starts_with('-') {
            return Err(error("CLI_USAGE", "project requires NAME before options"));
        }
        let mut parsed = Self {
            root,
            name,
            work: None,
            inspect: false,
            route: Route::Direct,
            codex_args: Vec::new(),
        };
        let mut seen_root = false;
        let mut after_separator = false;
        while let Some(arg) = args.next() {
            if after_separator {
                parsed.codex_args.push(arg);
                continue;
            }
            if arg == "--" {
                after_separator = true;
                continue;
            }
            if arg == "--inspect" {
                if parsed.inspect {
                    return Err(error("CLI_USAGE", "duplicate --inspect"));
                }
                parsed.inspect = true;
            } else if arg == "--proxy" {
                if parsed.route == Route::Proxy {
                    return Err(error("CLI_USAGE", "duplicate --proxy"));
                }
                parsed.route = Route::Proxy;
            } else if arg == "--root" || option_value(&arg, "--root=").is_some() {
                if seen_root {
                    return Err(error("CLI_USAGE", "duplicate --root after project"));
                }
                seen_root = true;
                let value = option_value(&arg, "--root=")
                    .or_else(|| args.next().filter(|value| value != "--"))
                    .ok_or_else(|| error("CLI_USAGE", "--root requires a path"))?;
                if value.is_empty() {
                    return Err(error("CLI_USAGE", "--root requires a path"));
                }
                parsed.root = value.into();
            } else if arg == "--work" || option_value(&arg, "--work=").is_some() {
                if parsed.work.is_some() {
                    return Err(error("CLI_USAGE", "duplicate --work"));
                }
                let value = option_value(&arg, "--work=")
                    .or_else(|| args.next())
                    .ok_or_else(|| error("CLI_USAGE", "--work requires an ID"))?;
                let value = utf8(value, "work ID must be UTF-8")?;
                if value.is_empty() || value.starts_with('-') {
                    return Err(error("CLI_USAGE", "--work requires an ID"));
                }
                parsed.work = Some(value);
            } else {
                parsed.codex_args.push(arg);
            }
        }
        Ok(parsed)
    }

    pub fn preview(&self) -> Result<Value> {
        check_arguments(&self.codex_args)?;
        let args: Vec<_> = self.codex_args.iter().map(|arg| arg.to_str().ok_or_else(|| error("NON_UTF8_ARGUMENT", "JSON inspection requires UTF-8 arguments; execution plumbing retains original OsString values"))).collect::<Result<_>>()?;
        Ok(json!({
            "requested_route": self.route,
            "codex_args": args,
            "argument_boundary": "Astral options are recognized before the first --; place all Codex arguments after -- to protect option values that resemble Astral options",
            "executed": false
        }))
    }

    /// Prepare a process only; this does not import, bind, or launch a projection.
    /// The executable comes from trusted caller code, never repository text.
    /// Explicit caller arguments remain a contiguous suffix, including permission
    /// flags and config overrides; later overrides may change the effective route.
    pub fn command(
        &self,
        executable: impl AsRef<OsStr>,
        proxy: Option<&ProxyBinding>,
    ) -> Result<Command> {
        check_arguments(&self.codex_args)?;
        let mut argv = Vec::new();
        match (self.route, proxy) {
            (Route::Direct, None) => {}
            (Route::Direct, Some(_)) => {
                return Err(error(
                    "UNREQUESTED_PROXY",
                    "a proxy binding requires explicit --proxy",
                ));
            }
            (Route::Proxy, None) => {
                return Err(error(
                    "PROXY_BINDING_REQUIRED",
                    "--proxy requires a caller-supplied proxy binding",
                ));
            }
            (Route::Proxy, Some(binding)) => {
                argv.extend([
                    OsString::from("-c"),
                    OsString::from(format!(
                        "openai_base_url={}",
                        serde_json::to_string(binding.url.as_str()).expect("URL serializes")
                    )),
                    OsString::from("-c"),
                    OsString::from("features.enable_request_compression=false"),
                ]);
            }
        }
        argv.extend(self.codex_args.iter().cloned());
        check_arguments(&argv)?;
        let mut command = Command::new(executable);
        command.current_dir(&self.root).args(argv);
        Ok(command)
    }

    /// Low-level process plumbing for a launcher that has already staged native
    /// state. The `astral project` CLI deliberately does not call this yet.
    pub fn spawn(
        &self,
        executable: impl AsRef<OsStr>,
        proxy: Option<&ProxyBinding>,
    ) -> Result<Child> {
        self.command(executable, proxy)?
            .spawn()
            .map_err(|_| error("SPAWN_FAILED", "could not start the requested executable"))
    }
}

#[derive(Debug, Clone)]
pub struct ProxyBinding {
    url: reqwest::Url,
}

impl ProxyBinding {
    /// A local routing value only. Construction performs no health check, DNS,
    /// connection, authentication, TLS override, or managed-proxy startup.
    pub fn loopback(base_url: &str) -> Result<Self> {
        if base_url.len() > MAX_ARGUMENT_BYTES {
            return Err(error(
                "ARGUMENT_LIMIT_EXCEEDED",
                "proxy URL byte limit exceeded",
            ));
        }
        let url = reqwest::Url::parse(base_url).map_err(|_| {
            error(
                "INVALID_PROXY_BINDING",
                "expected a loopback HTTP(S) base URL",
            )
        })?;
        let loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|host| host.is_loopback());
        if !matches!(url.scheme(), "http" | "https")
            || !loopback
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error(
                "INVALID_PROXY_BINDING",
                "expected a loopback HTTP(S) URL without credentials, query, or fragment",
            ));
        }
        Ok(Self { url })
    }
}

/// Recognize global root arguments until the subcommand. Returning `None` leaves
/// context/help/error parsing to Clap. Project arguments are never parsed by Clap,
/// which would need advance knowledge of every upstream Codex flag's arity.
pub fn project_request(args: &[OsString]) -> Result<Option<ProjectArguments>> {
    check_arguments(args)?;
    let mut root = PathBuf::from(".");
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg == "project" {
            if args
                .get(index + 1)
                .is_some_and(|arg| arg == "--help" || arg == "-h")
            {
                return Ok(None);
            }
            return ProjectArguments::parse(root, args[index + 1..].to_vec()).map(Some);
        }
        if arg == "--root" {
            index += 1;
            root = args
                .get(index)
                .filter(|value| !value.is_empty() && *value != "--")
                .ok_or_else(|| error("CLI_USAGE", "--root requires a path"))?
                .into();
        } else if let Some(value) = option_value(arg, "--root=") {
            if value.is_empty() {
                return Err(error("CLI_USAGE", "--root requires a path"));
            }
            root = value.into();
        } else {
            return Ok(None);
        }
        index += 1;
    }
    Ok(None)
}
