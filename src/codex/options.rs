//! Translate current caller options needed by staging without changing its argv.

use super::error;
use crate::project::Result;
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::path::Path;

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
                        | "--ephemeral"
                        | "--ignore-user-config"
                        | "--ignore-rules"
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
