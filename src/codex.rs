//! Owned, bounded Codex app-server client for staging fresh document context.
//!
//! No inference or tool request is sent here. The interactive Codex process owns
//! execution, authentication, approvals and sandbox enforcement.

use crate::native_bundle::NativeBundle;
use crate::project::{Error, Result};
use serde::Serialize;
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

pub(crate) struct Rpc {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    id: u64,
    request_limit: usize,
    notifications: Option<std::collections::VecDeque<Value>>,
}

impl Rpc {
    pub(crate) fn spawn(executable: &OsStr, root: &Path, args: &[OsString]) -> Result<Self> {
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
            request_limit: RPC_BYTES,
            notifications: None,
        })
    }

    pub(crate) async fn send(&mut self, value: impl Serialize) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)
            .map_err(|_| error("CODEX_PROTOCOL", "could not encode app-server request"))?;
        if bytes.len() >= self.request_limit {
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

    pub(crate) async fn request(&mut self, method: &str, params: impl Serialize) -> Result<Value> {
        tokio::time::timeout(RPC_TIMEOUT, self.request_inner(method, params))
            .await
            .map_err(|_| error("CODEX_TIMEOUT", format!("Codex {method} timed out")))?
    }

    async fn request_inner(&mut self, method: &str, params: impl Serialize) -> Result<Value> {
        self.id += 1;
        let id = self.id;
        #[derive(Serialize)]
        struct Request<'a, T> {
            id: u64,
            method: &'a str,
            params: T,
        }
        self.send(Request { id, method, params }).await?;
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
                if let Some(queue) = &mut self.notifications {
                    queue.push_back(message);
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

    /// Explicit compaction only. Rejects every client action request; supplies
    /// neither tool execution nor an approval response.
    pub(crate) async fn compact(&mut self, thread: &str) -> Result<(String, String)> {
        self.notifications = Some(std::collections::VecDeque::new());
        let result = tokio::time::timeout(Duration::from_secs(300), async {
            self.request("thread/compact/start", json!({"threadId":thread}))
                .await?;
            let mut turn: Option<String> = None;
            let mut item: Option<String> = None;
            let mut item_completed = false;
            let mut total = 0usize;
            for _ in 0..RPC_MESSAGES {
                let message = if let Some(value) =
                    self.notifications.as_mut().and_then(|q| q.pop_front())
                {
                    value
                } else {
                    let mut line = Vec::new();
                    (&mut self.output)
                        .take((RPC_BYTES + 1) as u64)
                        .read_until(b'\n', &mut line)
                        .await
                        .map_err(|_| error("CODEX_PROTOCOL", "could not read compaction event"))?;
                    total = total.saturating_add(line.len());
                    if line.len() > RPC_BYTES || total > 16 * RPC_BYTES {
                        return Err(error("LIMIT_EXCEEDED", "compaction event byte limit"));
                    }
                    if line.last() != Some(&b'\n') {
                        return Err(error("CODEX_PROTOCOL", "incomplete compaction event"));
                    }
                    serde_json::from_slice(&line)
                        .map_err(|_| error("CODEX_PROTOCOL", "invalid compaction event"))?
                };
                if message.get("id").is_some() {
                    return Err(error(
                        "CODEX_UNEXPECTED_REQUEST",
                        "compaction requested a client action or returned an unexpected response",
                    ));
                }
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .ok_or_else(|| error("CODEX_PROTOCOL", "missing event method"))?;
                let params = &message["params"];
                if matches!(
                    method,
                    "turn/started" | "turn/completed" | "item/started" | "item/completed"
                ) && params.get("threadId").and_then(Value::as_str) != Some(thread)
                {
                    return Err(error(
                        "CAPTURE_LIFECYCLE",
                        "missing or mismatched compaction thread ID",
                    ));
                }
                if params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id != thread)
                {
                    return Err(error(
                        "CAPTURE_LIFECYCLE",
                        "compaction event belongs to another thread",
                    ));
                }
                match method {
                    "turn/started" => {
                        let id = params
                            .pointer("/turn/id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty() && id.len() <= 128)
                            .ok_or_else(|| {
                                error("CAPTURE_LIFECYCLE", "missing compaction turn ID")
                            })?;
                        if turn.replace(id.to_owned()).is_some() {
                            return Err(error("CAPTURE_LIFECYCLE", "multiple compaction turns"));
                        }
                    }
                    "item/started" | "item/completed" => {
                        if params.get("turnId").and_then(Value::as_str) != turn.as_deref()
                            || params.pointer("/item/type").and_then(Value::as_str)
                                != Some("contextCompaction")
                        {
                            return Err(error(
                                "CAPTURE_LIFECYCLE",
                                "unexpected compaction item or turn",
                            ));
                        }
                        let id = params
                            .pointer("/item/id")
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty() && id.len() <= 128)
                            .ok_or_else(|| {
                                error("CAPTURE_LIFECYCLE", "missing compaction item ID")
                            })?;
                        if method == "item/started" {
                            if item.replace(id.to_owned()).is_some() {
                                return Err(error(
                                    "CAPTURE_LIFECYCLE",
                                    "multiple compaction items",
                                ));
                            }
                        } else {
                            if item.as_deref() != Some(id) || item_completed {
                                return Err(error(
                                    "CAPTURE_LIFECYCLE",
                                    "unmatched compaction item completion",
                                ));
                            }
                            item_completed = true;
                        }
                    }
                    "turn/completed" => {
                        if !item_completed
                            || turn.is_none()
                            || params.pointer("/turn/id").and_then(Value::as_str) != turn.as_deref()
                            || params.pointer("/turn/status").and_then(Value::as_str)
                                != Some("completed")
                            || params.pointer("/turn/error").is_some_and(|e| !e.is_null())
                        {
                            return Err(error(
                                "CAPTURE_LIFECYCLE",
                                "compaction did not complete successfully",
                            ));
                        }
                        return Ok((turn.expect("checked turn"), item.expect("checked item")));
                    }
                    "error" => {
                        return Err(error("CAPTURE_FAILED", "Codex reported a compaction error"));
                    }
                    _ => {}
                }
            }
            Err(error("LIMIT_EXCEEDED", "compaction event count limit"))
        })
        .await
        .map_err(|_| error("CODEX_TIMEOUT", "native compaction timed out"))?;
        self.notifications = None;
        result
    }

    pub(crate) async fn close(&mut self) -> Result<()> {
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

#[derive(Debug)]
pub struct StagedThread {
    pub thread_id: String,
    pub model: String,
    pub model_provider: String,
}

pub(crate) async fn verify_native_version(
    executable: &OsStr,
    root: &Path,
    expected: &str,
) -> Result<()> {
    let mut child = Command::new(executable)
        .arg("--version")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| error("CODEX_UNAVAILABLE", "could not check Codex version"))?;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or_else(|| error("CODEX_PROTOCOL", "version output unavailable"))?
            .take(4097)
            .read_to_end(&mut output)
            .await
            .map_err(|_| error("CODEX_PROTOCOL", "could not read Codex version"))?;
        if output.len() > 4096 {
            return Err(error("LIMIT_EXCEEDED", "Codex version output limit"));
        }
        let status = child
            .wait()
            .await
            .map_err(|_| error("CODEX_UNAVAILABLE", "could not check Codex version"))?;
        let version = String::from_utf8_lossy(&output);
        if !status.success() || version.trim() != format!("codex-cli {expected}") {
            return Err(error(
                "NATIVE_RUNTIME_MISMATCH",
                format!("native launch requires Codex {expected}"),
            ));
        }
        Ok(())
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(error("CODEX_TIMEOUT", "Codex version check timed out"))
        }
    }
}

/// Import one complete native window, or reopen a receipt-bound destination.
/// Only current runtime options supply capabilities and permissions. No turn
/// is started here. Raw items are serialized directly, without a Value roundtrip.
pub async fn stage_native(
    executable: &OsStr,
    root: &Path,
    options: StagingOptions,
    bundle: &NativeBundle,
    context: &str,
    proxy_url: &str,
    resume_thread: Option<&str>,
) -> Result<StagedThread> {
    stage_native_before_start(
        executable,
        root,
        options,
        bundle,
        context,
        proxy_url,
        resume_thread,
        || Ok(()),
    )
    .await
}

/// Install destination ownership only after no-thread preflights pass. The hook
/// precedes the first thread/start request; after it succeeds, any interrupted
/// request may have created a thread and must retain uncertain staging state.
#[allow(clippy::too_many_arguments)] // The hook supplements the compatible public staging API.
pub(crate) async fn stage_native_before_start<F: FnOnce() -> Result<()>>(
    executable: &OsStr,
    root: &Path,
    mut options: StagingOptions,
    bundle: &NativeBundle,
    context: &str,
    proxy_url: &str,
    resume_thread: Option<&str>,
    before_start: F,
) -> Result<StagedThread> {
    crate::native_import::validate(bundle)?;
    let compatibility = &bundle.manifest().compatibility;
    verify_native_version(executable, root, &compatibility.runtime_version).await?;
    if resume_thread.is_some_and(|id| !valid_thread_id(id)) {
        return Err(error("CODEX_PROTOCOL", "invalid receipt thread ID"));
    }
    // Same order as the child command: owned routing precedes current caller
    // overrides. Effective configuration is checked before native items leave
    // this process, so an override cannot silently route a checkpoint elsewhere.
    crate::launcher::ProxyBinding::loopback(proxy_url)?;
    let mut args: Vec<OsString> = vec!["app-server".into(), "--listen".into(), "stdio://".into()];
    args.extend([
        "-c".into(),
        format!("openai_base_url={proxy_url:?}").into(),
        "-c".into(),
        "features.enable_request_compression=false".into(),
    ]);
    args.extend(options.server_args.into_iter().skip(3));
    let mut rpc = Rpc::spawn(executable, root, &args)?;
    // Native payload is at most 8 MiB; the current document prompt is at most
    // 2 MiB before JSON escaping. Responses retain their independent 4 MiB cap.
    rpc.request_limit = 24 * 1024 * 1024;
    let result = async {
        rpc.request("initialize", json!({"clientInfo":{"name":"astral","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        rpc.send(json!({"method":"initialized","params":{}})).await?;
        let config = rpc.request("config/read", json!({"cwd":root,"includeLayers":false})).await?;
        let config = config.get("config").ok_or_else(|| error("CODEX_PROTOCOL", "missing effective configuration"))?;
        if config.get("openai_base_url").and_then(Value::as_str) != Some(proxy_url)
            || config.pointer("/features/enable_request_compression").and_then(Value::as_bool) != Some(false)
            || config.pointer("/model_providers/openai").is_some_and(|v| !v.is_null())
            || config.get("model_provider").and_then(Value::as_str).is_some_and(|p| p != compatibility.provider)
        {
            return Err(error("NATIVE_ROUTE_CONFLICT", "effective Codex configuration differs from the owned native proxy route"));
        }
        let method = if let Some(id) = resume_thread {
            options.thread_params["threadId"] = json!(id);
            options.thread_params["excludeTurns"] = json!(true);
            "thread/resume"
        } else { before_start()?; "thread/start" };
        let started = rpc.request(method, options.thread_params).await?;
        let id = started.pointer("/thread/id").and_then(Value::as_str).filter(|id| valid_thread_id(id))
            .ok_or_else(|| error("CODEX_PROTOCOL", "invalid staged thread ID"))?.to_owned();
        if resume_thread.is_some_and(|expected| expected != id)
            || started.pointer("/thread/ephemeral").and_then(Value::as_bool) != Some(false)
        { return Err(error("CODEX_PROTOCOL", "native thread identity or durable storage differs from requested binding")); }
        let cwd = started.get("cwd").and_then(Value::as_str).ok_or_else(|| error("CODEX_PROTOCOL", "native thread did not report its workspace"))?;
        if Path::new(cwd).canonicalize().ok().as_deref() != Some(root) {
            return Err(error("WORKSPACE_CONFLICT", "native thread workspace differs from selected context"));
        }
        let model = started.get("model").and_then(Value::as_str);
        let provider = started.get("modelProvider").and_then(Value::as_str);
        if model != Some(compatibility.model.as_str()) || provider != Some(compatibility.provider.as_str()) {
            return Err(error("NATIVE_RUNTIME_MISMATCH", "effective model/provider differs from the native bundle"));
        }
        if resume_thread.is_none() {
            #[derive(Serialize)]
            #[serde(rename_all="camelCase")]
            struct Injection<'a> { thread_id: &'a str, items: Vec<&'a serde_json::value::RawValue> }
            let current = serde_json::value::to_raw_value(&json!({"type":"message","role":"user","content":[{"type":"input_text","text":context}]}))
                .map_err(|_| error("CODEX_PROTOCOL", "could not encode current project context"))?;
            let mut items: Vec<_> = bundle.raw_items().iter().map(AsRef::as_ref).collect();
            items.push(current.as_ref());
            rpc.request("thread/inject_items", Injection { thread_id:&id, items }).await?;
        }
        Ok(StagedThread { thread_id:id, model:compatibility.model.clone(), model_provider:compatibility.provider.clone() })
    }.await;
    let closed = rpc.close().await;
    let staged = result?;
    closed?;
    Ok(staged)
}

pub(crate) async fn initialize_rpc(rpc: &mut Rpc) -> Result<()> {
    rpc.request("initialize", json!({"clientInfo":{"name":"astral","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
    rpc.send(json!({"method":"initialized","params":{}})).await
}

pub(crate) fn proxy_args(options: &StagingOptions, proxy: Option<&str>) -> Result<Vec<OsString>> {
    let Some(url) = proxy else {
        return Ok(options.server_args.clone());
    };
    crate::launcher::ProxyBinding::loopback(url)?;
    let mut args: Vec<OsString> = vec![
        "app-server".into(),
        "--listen".into(),
        "stdio://".into(),
        "-c".into(),
        format!("openai_base_url={url:?}").into(),
        "-c".into(),
        "features.enable_request_compression=false".into(),
    ];
    args.extend(options.server_args.iter().skip(3).cloned());
    Ok(args)
}

pub(crate) async fn verify_proxy(rpc: &mut Rpc, root: &Path, proxy: Option<&str>) -> Result<()> {
    if let Some(url) = proxy {
        let value = rpc
            .request("config/read", json!({"cwd":root,"includeLayers":false}))
            .await?;
        let config = value
            .get("config")
            .ok_or_else(|| error("CODEX_PROTOCOL", "missing effective configuration"))?;
        if config.get("openai_base_url").and_then(Value::as_str) != Some(url)
            || config
                .pointer("/features/enable_request_compression")
                .and_then(Value::as_bool)
                != Some(false)
            || config
                .pointer("/model_providers/openai")
                .is_some_and(|v| !v.is_null())
            || config
                .get("model_provider")
                .and_then(Value::as_str)
                .is_some_and(|v| v != "openai")
        {
            return Err(error(
                "NATIVE_ROUTE_CONFLICT",
                "effective configuration differs from the owned proxy route",
            ));
        }
    }
    Ok(())
}

pub(crate) fn staged_identity(
    value: &Value,
    root: &Path,
    expected_id: Option<&str>,
) -> Result<StagedThread> {
    let id = value
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .filter(|id| valid_thread_id(id))
        .ok_or_else(|| error("CODEX_PROTOCOL", "invalid worker thread ID"))?;
    if expected_id.is_some_and(|expected| expected != id)
        || value.pointer("/thread/ephemeral").and_then(Value::as_bool) != Some(false)
    {
        return Err(error(
            "CODEX_PROTOCOL",
            "worker identity or durable storage mismatch",
        ));
    }
    let cwd = value
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| error("CODEX_PROTOCOL", "missing worker workspace"))?;
    if Path::new(cwd).canonicalize().ok().as_deref() != Some(root) {
        return Err(error(
            "WORKSPACE_CONFLICT",
            "worker workspace differs from selected worktree",
        ));
    }
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or_else(|| error("CODEX_PROTOCOL", "missing worker model"))?;
    let provider = value
        .get("modelProvider")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or_else(|| error("CODEX_PROTOCOL", "missing worker provider"))?;
    Ok(StagedThread {
        thread_id: id.into(),
        model: model.into(),
        model_provider: provider.into(),
    })
}

/// Stage a fresh bound worker, or reopen its exact local thread. A context
/// update appends current selected documents, never the original native window.
pub async fn stage_worker(
    executable: &OsStr,
    root: &Path,
    options: StagingOptions,
    proxy: Option<&str>,
    resume: Option<&str>,
    context_update: Option<&str>,
    native_runtime: Option<(&str, &str)>,
) -> Result<StagedThread> {
    stage_worker_before_start(
        executable,
        root,
        options,
        proxy,
        resume,
        context_update,
        native_runtime,
        || Ok(()),
    )
    .await
}

#[allow(clippy::too_many_arguments)] // The hook supplements the compatible public staging API.
pub(crate) async fn stage_worker_before_start<F: FnOnce() -> Result<()>>(
    executable: &OsStr,
    root: &Path,
    mut options: StagingOptions,
    proxy: Option<&str>,
    resume: Option<&str>,
    context_update: Option<&str>,
    native_runtime: Option<(&str, &str)>,
    before_start: F,
) -> Result<StagedThread> {
    verify_native_version(executable, root, "0.154.0").await?;
    if resume.is_some_and(|id| !valid_thread_id(id)) {
        return Err(error("CODEX_PROTOCOL", "invalid bound worker thread ID"));
    }
    if native_runtime.is_some() && proxy.is_none() {
        return Err(error(
            "NATIVE_PROXY_REQUIRED",
            "this worker contains saved native context; pass --proxy",
        ));
    }
    let args = proxy_args(&options, proxy)?;
    let mut rpc = Rpc::spawn(executable, root, &args)?;
    let result = async {
        initialize_rpc(&mut rpc).await?;
        verify_proxy(&mut rpc, root, proxy).await?;
        let method = if let Some(id) = resume {
            options.thread_params["threadId"] = json!(id);
            options.thread_params["excludeTurns"] = json!(true);
            "thread/resume"
        } else { before_start()?; "thread/start" };
        let started = rpc.request(method, options.thread_params).await?;
        let staged = staged_identity(&started, root, resume)?;
        if native_runtime.is_some_and(|(model,provider)| staged.model != model || staged.model_provider != provider) {
            return Err(error("NATIVE_RUNTIME_MISMATCH", "effective worker model/provider differs from its native state"));
        }
        if let Some(context) = context_update {
            rpc.request("thread/inject_items", json!({"threadId":staged.thread_id,"items":[{"type":"message","role":"user","content":[{"type":"input_text","text":context}]}]})).await?;
        }
        Ok(staged)
    }.await;
    let closed = rpc.close().await;
    let staged = result?;
    closed?;
    Ok(staged)
}

pub async fn wait_interactive(mut command: Command) -> Result<i32> {
    let status = command
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
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

#[cfg(all(test, unix))]
mod compaction_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const FIXTURE: &str = r#"#!/usr/bin/env python3
import json,sys
mode=sys.argv[1]
def emit(value): print(json.dumps(value),flush=True)
for line in sys.stdin:
    request=json.loads(line)
    assert request['method']=='thread/compact/start'
    thread=request['params']['threadId']
    def event(method, **params):
        if mode!='missing-thread': params['threadId']=thread
        emit(dict(method=method,params=params))
    if mode=='action':
        emit(dict(id=99,method='item/commandExecution/requestApproval',params={}))
        continue
    if mode=='early-result': emit(dict(id=request['id'],result={}))
    event('turn/started',turn=dict(id='turn1'))
    event('item/started',turnId='turn1',item=dict(type='contextCompaction',id='compact1'))
    if mode!='incomplete': event('item/completed',turnId='turn1',item=dict(type='contextCompaction',id='compact1'))
    event('turn/completed',turn=dict(id='turn1',status='completed',error=None if mode!='error' else dict(message='private error body')))
    if mode!='early-result': emit(dict(id=request['id'],result={}))
"#;

    #[tokio::test]
    async fn compaction_correlates_events_before_or_after_ack_and_rejects_unsafe_completion() {
        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("fixture");
        std::fs::write(&executable, FIXTURE).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        for (mode, expected) in [
            ("ok", None),
            ("early-result", None),
            ("missing-thread", Some("CAPTURE_LIFECYCLE")),
            ("incomplete", Some("CAPTURE_LIFECYCLE")),
            ("error", Some("CAPTURE_LIFECYCLE")),
            ("action", Some("CODEX_UNEXPECTED_REQUEST")),
        ] {
            let mut rpc = Rpc::spawn(executable.as_os_str(), temp.path(), &[mode.into()]).unwrap();
            let result = rpc.compact("01a10000-1234-7000-8000-000000000001").await;
            rpc.close().await.unwrap();
            if let Some(code) = expected {
                assert_eq!(result.unwrap_err().code, code, "{mode}");
            } else {
                assert_eq!(
                    result.unwrap(),
                    ("turn1".into(), "compact1".into()),
                    "{mode}"
                );
            }
        }
    }
}
