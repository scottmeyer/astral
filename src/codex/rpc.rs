//! Owned app-server process, bounded RPC transport and lifecycle checks.

use super::error;
use crate::project::Result;
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

pub(crate) struct Rpc {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    id: u64,
    pub(super) request_limit: usize,
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

    /// Always release the staging process, preserving the primary failure if
    /// both staging and shutdown fail. A successful result requires clean EOF.
    pub(super) async fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        let closed = self.close().await;
        let staged = result?;
        closed?;
        Ok(staged)
    }
}

pub(crate) async fn initialize_rpc(rpc: &mut Rpc) -> Result<()> {
    rpc.request("initialize", json!({"clientInfo":{"name":"astral","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
    rpc.send(json!({"method":"initialized","params":{}})).await
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
