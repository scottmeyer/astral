//! Owned, bounded Codex app-server client for staging fresh document context.
//!
//! No inference or tool request is sent here. The interactive Codex process owns
//! execution, authentication, approvals and sandbox enforcement.

use crate::project::{Error, Result};
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const RPC_BYTES: usize = 4 * 1024 * 1024;
const RPC_MESSAGES: usize = 4096;
const RPC_TIMEOUT: Duration = Duration::from_secs(45);

pub(crate) fn error(code: &'static str, message: impl Into<String>) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

/// Settings needed by both staging and the final interactive process. Raw user
/// arguments are retained separately; this is not a replacement argv parser.
#[derive(Debug)]
pub struct StagingOptions {
    pub server_args: Vec<OsString>,
    pub thread_params: Value,
}

impl StagingOptions {
    pub fn from_args(root: &Path, args: &[OsString]) -> Result<Self> {
        Self::parse(root, args, true)
    }

    /// Direct initialization does not import saved runtime settings, so profile,
    /// provider selection and additional write roots can go straight to Codex.
    pub fn validate_initialization(root: &Path, args: &[OsString]) -> Result<()> {
        Self::parse(root, args, false).map(|_| ())
    }

    fn parse(root: &Path, args: &[OsString], staging: bool) -> Result<Self> {
        let cwd = root.to_str().ok_or_else(|| {
            error(
                "NON_UTF8_ROOT",
                "Codex app-server requires a UTF-8 workspace path",
            )
        })?;
        let mut options = Self {
            server_args: vec!["app-server".into(), "--listen".into(), "stdio://".into()],
            thread_params: json!({"cwd": cwd}),
        };
        let mut bypass = false;
        let mut auto_review = false;
        let mut search = false;
        let mut bypass_hook_trust = false;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--" {
                if !staging && i + 1 < args.len() {
                    return Err(error(
                        "CLI_USAGE",
                        "init supplies its own prompt; pass only Codex options",
                    ));
                }
                break;
            }
            let bytes = arg.as_encoded_bytes();
            if arg == "-i"
                || arg == "--image"
                || bytes.starts_with(b"--image=")
                || (bytes.starts_with(b"-i") && !bytes.starts_with(b"--"))
            {
                // Codex images have num_args=1.. and are PathBuf values. Retain
                // arbitrary OS bytes and consume their full option arity.
                let mut count = usize::from(arg != "-i" && arg != "--image");
                while args
                    .get(i + 1)
                    .is_some_and(|next| !next.as_encoded_bytes().starts_with(b"-"))
                {
                    i += 1;
                    count += 1;
                }
                if count == 0 {
                    return Err(error("CLI_USAGE", "Codex image option requires a path"));
                }
                i += 1;
                continue;
            }
            let text = arg.to_str().ok_or_else(|| {
                error(
                    "NON_UTF8_ARGUMENT",
                    "Codex option names and prompts must be UTF-8",
                )
            })?;
            let (key, inline) = text
                .split_once('=')
                .map_or((text, None), |(k, v)| (k, Some(v)));
            if !staging && !text.starts_with('-') {
                return Err(error(
                    "CLI_USAGE",
                    "init supplies its own prompt; subcommands and extra prompts are not accepted",
                ));
            }
            if inline.is_some()
                && matches!(
                    key,
                    "--strict-config"
                        | "--dangerously-bypass-approvals-and-sandbox"
                        | "--yolo"
                        | "--approve-for-me"
                        | "--not-so-yolo"
                        | "--search"
                        | "--dangerously-bypass-hook-trust"
                )
            {
                return Err(error("CLI_USAGE", "Codex boolean flags do not take values"));
            }
            if matches!(
                key,
                "--last" | "--remote" | "--remote-auth-token-env" | "--worktree"
            ) || (staging
                && (matches!(
                    key,
                    "--last"
                        | "--remote"
                        | "--remote-auth-token-env"
                        | "--worktree"
                        | "--oss"
                        | "--local-provider"
                        | "--profile"
                        | "-p"
                        | "--add-dir"
                ) || (text.starts_with("-p") && !text.starts_with("--") && text.len() > 2)))
            {
                return Err(error(
                    "UNSUPPORTED_LAUNCH_ARGUMENT",
                    "session selectors, remote runtimes, worktrees, extra write roots and provider/profile selection are not supported by fresh staging; select the checkout with --root and configure the default Codex runtime",
                ));
            }
            let value_key = match key {
                "-c"
                | "--config"
                | "--enable"
                | "--disable"
                | "-m"
                | "--model"
                | "-s"
                | "--sandbox"
                | "-a"
                | "--ask-for-approval"
                | "-C"
                | "--cd"
                | "--add-dir"
                | "-i"
                | "--image"
                | "-p"
                | "--profile"
                | "--local-provider"
                | "-o"
                | "--output-last-message"
                | "--output-schema"
                | "--color"
                | "--thread-source" => Some(key),
                _ => None,
            };
            // Accept attached short values as Codex does, including -mMODEL and -cKEY=VALUE.
            let attached = ["-c", "-m", "-s", "-a", "-C", "-i", "-p", "-o"]
                .into_iter()
                .find(|prefix| {
                    text.starts_with(prefix) && !text.starts_with("--") && text.len() > prefix.len()
                });
            if let Some(option) = attached.or(value_key) {
                let value: &OsStr = if let Some(prefix) = attached {
                    OsStr::new(
                        text[prefix.len()..]
                            .strip_prefix('=')
                            .unwrap_or(&text[prefix.len()..]),
                    )
                } else if let Some(value) = inline {
                    OsStr::new(value)
                } else {
                    i += 1;
                    args.get(i)
                        .filter(|v| *v != "--")
                        .ok_or_else(|| error("CLI_USAGE", "Codex option requires a value"))?
                };
                match option {
                    "-c" | "--config" => {
                        let value_text = value.to_str().ok_or_else(|| {
                            error("NON_UTF8_ARGUMENT", "Codex configuration must be UTF-8")
                        })?;
                        let config_key = value_text
                            .split_once('=')
                            .map(|(k, _)| k.trim())
                            .unwrap_or("");
                        // These settings redirect state or workspace outside the thread being staged.
                        if config_key == "cwd"
                            || config_key.starts_with("environments")
                            || (staging
                                && matches!(
                                    config_key,
                                    "profile" | "config_profile" | "sqlite_home" | "codex_home"
                                ))
                        {
                            return Err(error(
                                "UNSUPPORTED_LAUNCH_ARGUMENT",
                                "workspace, profile, environment and session-store config overrides are unsupported during fresh staging",
                            ));
                        }
                        options.server_args.extend(["-c".into(), value.to_owned()]);
                    }
                    "--enable" | "--disable" => options
                        .server_args
                        .extend([option.into(), value.to_owned()]),
                    "-C" | "--cd" => {
                        let destination = root.join(value).canonicalize().map_err(|_| {
                            error(
                                "WORKSPACE_CONFLICT",
                                "Codex --cd must resolve to the selected --root",
                            )
                        })?;
                        if destination != root {
                            return Err(error(
                                "WORKSPACE_CONFLICT",
                                "Codex --cd differs from selected context; choose the checkout with --root",
                            ));
                        }
                    }
                    "-m" | "--model" | "-s" | "--sandbox" | "-a" | "--ask-for-approval" => {
                        let field = match option {
                            "-m" | "--model" => "model",
                            "-s" | "--sandbox" => "sandbox",
                            _ => "approvalPolicy",
                        };
                        let value = value.to_str().ok_or_else(|| {
                            error("NON_UTF8_ARGUMENT", "Codex runtime settings must be UTF-8")
                        })?;
                        if value.is_empty()
                            || (field == "sandbox"
                                && !matches!(
                                    value,
                                    "read-only" | "workspace-write" | "danger-full-access"
                                ))
                            || (field == "approvalPolicy"
                                && !matches!(value, "on-request" | "never"))
                            || options.thread_params.get(field).is_some()
                        {
                            return Err(error(
                                "CLI_USAGE",
                                "invalid or repeated Codex runtime setting",
                            ));
                        }
                        options.thread_params[field] = Value::String(value.to_owned());
                    }
                    _ => {}
                }
            } else if key == "--strict-config" {
                options.server_args.push(arg.clone());
            } else if matches!(key, "--dangerously-bypass-approvals-and-sandbox" | "--yolo") {
                bypass = true;
            } else if matches!(key, "--approve-for-me" | "--not-so-yolo") {
                auto_review = true;
            } else if key == "--search" {
                search = true;
            } else if key == "--dangerously-bypass-hook-trust" {
                bypass_hook_trust = true;
            }
            i += 1;
        }
        if (bypass && options.thread_params.get("approvalPolicy").is_some())
            || (auto_review
                && (bypass
                    || options.thread_params.get("approvalPolicy").is_some()
                    || options.thread_params.get("sandbox").is_some()))
        {
            return Err(error(
                "CLI_USAGE",
                "conflicting Codex approval or sandbox flags",
            ));
        }
        if bypass {
            // Only an explicit current CLI request can select this policy.
            options.thread_params["sandbox"] = json!("danger-full-access");
            options.thread_params["approvalPolicy"] = json!("never");
        }
        if auto_review {
            options.server_args.extend([
                "-c".into(),
                "approvals_reviewer=\"auto_review\"".into(),
                "-c".into(),
                "approval_policy=\"on-request\"".into(),
                "-c".into(),
                "sandbox_mode=\"workspace-write\"".into(),
            ]);
        }
        if search {
            options
                .server_args
                .extend(["-c".into(), "web_search=\"live\"".into()]);
        }
        if bypass_hook_trust {
            options.thread_params["config"] = json!({"bypass_hook_trust": true});
        }
        Ok(options)
    }
}

struct Rpc {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    id: u64,
}

impl Rpc {
    fn spawn(executable: &OsStr, root: &Path, args: &[OsString]) -> Result<Self> {
        let mut child = Command::new(executable)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| error("CODEX_UNAVAILABLE", "could not start Codex app-server"))?;
        let input = child.stdin.take();
        let output = child
            .stdout
            .take()
            .ok_or_else(|| error("CODEX_PROTOCOL", "app-server output unavailable"))?;
        Ok(Self {
            child,
            input,
            output: BufReader::new(output),
            id: 0,
        })
    }

    async fn send(&mut self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)
            .map_err(|_| error("CODEX_PROTOCOL", "could not encode app-server request"))?;
        if bytes.len() >= RPC_BYTES {
            return Err(error("LIMIT_EXCEEDED", "app-server request byte limit"));
        }
        bytes.push(b'\n');
        self.input
            .as_mut()
            .ok_or_else(|| error("CODEX_PROTOCOL", "app-server input closed"))?
            .write_all(&bytes)
            .await
            .map_err(|_| error("CODEX_PROTOCOL", "could not write app-server request"))
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        tokio::time::timeout(RPC_TIMEOUT, self.request_inner(method, params))
            .await
            .map_err(|_| error("CODEX_TIMEOUT", format!("Codex {method} timed out")))?
    }

    async fn request_inner(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id += 1;
        let id = self.id;
        self.send(json!({"id": id, "method": method, "params": params}))
            .await?;
        let mut total = 0usize;
        for _ in 0..RPC_MESSAGES {
            let mut line = Vec::new();
            (&mut self.output)
                .take((RPC_BYTES + 1) as u64)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|_| error("CODEX_PROTOCOL", "could not read app-server response"))?;
            total = total.saturating_add(line.len());
            if line.len() > RPC_BYTES || total > 16 * RPC_BYTES {
                return Err(error("LIMIT_EXCEEDED", "app-server response byte limit"));
            }
            if line.last() != Some(&b'\n') {
                return Err(error(
                    "CODEX_PROTOCOL",
                    "app-server closed or sent an incomplete response",
                ));
            }
            let message: Value = serde_json::from_slice(&line)
                .map_err(|_| error("CODEX_PROTOCOL", "invalid app-server JSON"))?;
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    return Err(error(
                        "CODEX_UNEXPECTED_REQUEST",
                        "staging requested a client action; no approval or tool execution was supplied",
                    ));
                }
                continue;
            }
            if message.get("id") != Some(&json!(id)) {
                return Err(error("CODEX_PROTOCOL", "unexpected app-server response ID"));
            }
            if message.get("error").is_some() {
                return Err(error(
                    "CODEX_REQUEST_FAILED",
                    format!("Codex {method} failed; inspect Codex diagnostics"),
                ));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| error("CODEX_PROTOCOL", "missing app-server result"));
        }
        Err(error("LIMIT_EXCEEDED", "app-server message count limit"))
    }

    async fn close(&mut self) -> Result<()> {
        self.input.take();
        match tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await {
            Ok(Ok(status)) if status.success() => Ok(()),
            Ok(_) => Err(error(
                "CODEX_SHUTDOWN_FAILED",
                "staging app-server did not exit successfully",
            )),
            Err(_) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                Err(error(
                    "CODEX_SHUTDOWN_FAILED",
                    "staging app-server failed to release its thread in time",
                ))
            }
        }
    }
}

fn valid_thread_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

/// Create a new thread and persist exactly one user-context message. No session
/// file, credential or native artifact is read or patched by this adapter.
pub async fn stage_fresh(
    executable: &OsStr,
    root: &Path,
    options: StagingOptions,
    context: &str,
) -> Result<String> {
    let mut rpc = Rpc::spawn(executable, root, &options.server_args)?;
    let result = async {
        rpc.request("initialize", json!({"clientInfo": {"name": "astral", "version": env!("CARGO_PKG_VERSION")}, "capabilities": {"experimentalApi": true}})).await?;
        rpc.send(json!({"method": "initialized", "params": {}})).await?;
        let started = rpc.request("thread/start", options.thread_params).await?;
        let id = started.pointer("/thread/id").and_then(Value::as_str).filter(|id| valid_thread_id(id)).ok_or_else(|| error("CODEX_PROTOCOL", "invalid staged thread ID"))?.to_owned();
        if started.pointer("/thread/ephemeral").and_then(Value::as_bool) != Some(false) { return Err(error("CODEX_PROTOCOL", "staged thread must report durable storage")); }
        let returned_cwd = started.get("cwd").and_then(Value::as_str).ok_or_else(|| error("CODEX_PROTOCOL", "staged thread did not report its workspace"))?;
        if Path::new(returned_cwd).canonicalize().ok().as_deref() != Some(root) { return Err(error("WORKSPACE_CONFLICT", "staged thread workspace differs from selected context")); }
        rpc.request("thread/inject_items", json!({"threadId": id, "items": [{"type":"message", "role":"user", "content":[{"type":"input_text", "text":context}]}]})).await?;
        Ok(id)
    }.await;
    let closed = rpc.close().await;
    let id = result?;
    closed?;
    Ok(id)
}

pub async fn wait_interactive(mut command: Command) -> Result<i32> {
    let status = command
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .map_err(|_| {
            error(
                "CODEX_UNAVAILABLE",
                "could not run Codex; check its installation and arguments",
            )
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        Ok(status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)))
    }
    #[cfg(not(unix))]
    Ok(status.code().unwrap_or(1))
}
