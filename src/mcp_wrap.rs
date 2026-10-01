//! A stdio MCP relay that changes eligible tool results only at their first delivery.
use anyhow::{Context, ensure};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};

const MAX_PENDING: usize = 64;
const MAX_JOBS: usize = 8;
pub const MAX_MESSAGE: usize = 32 * 1024 * 1024;

#[derive(Clone)]
enum Pending {
    Initialize,
    List,
    Call { tool: String, eligible: bool },
    Other,
}

fn local_url(proxy: &str) -> anyhow::Result<reqwest::Url> {
    crate::providers::validate_upstream(proxy)?;
    let mut url = reqwest::Url::parse(proxy)?;
    ensure!(
        url.path() == "/" || url.path().is_empty(),
        "proxy URL must name the listener root"
    );
    let host = url.host_str().unwrap_or("");
    ensure!(
        host == "localhost"
            || host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()),
        "MCP intake requires a loopback proxy URL"
    );
    url.set_path("/_astral/intake");
    Ok(url)
}

fn reader<R: AsyncRead + Unpin + Send + 'static>(
    input: R,
) -> (mpsc::Receiver<std::io::Result<Vec<u8>>>, JoinHandle<()>) {
    let (send, receive) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        let mut input = BufReader::new(input);
        loop {
            let mut line = Vec::new();
            let read = (&mut input)
                .take(MAX_MESSAGE as u64 + 1)
                .read_until(b'\n', &mut line)
                .await;
            let value = match read {
                Ok(0) => break,
                Ok(_) if line.len() <= MAX_MESSAGE => Ok(line),
                Ok(_) => Err(std::io::Error::other("MCP message exceeds 32 MiB")),
                Err(error) => Err(error),
            };
            let failed = value.is_err();
            if send.send(value).await.is_err() || failed {
                break;
            }
        }
    });
    (receive, task)
}

async fn write(output: &mut (impl AsyncWrite + Unpin), bytes: &[u8]) -> anyhow::Result<()> {
    output.write_all(bytes).await?;
    if !bytes.ends_with(b"\n") {
        output.write_all(b"\n").await?;
    }
    output.flush().await?;
    Ok(())
}

fn id(value: &Value) -> Option<String> {
    let id = value.get("id")?;
    (id.is_string() || id.is_number()).then(|| id.to_string())
}

fn own_tool(request: &Value) -> bool {
    request["method"] == "tools/call"
        && matches!(
            request["params"]["name"].as_str(),
            Some("astral_recall" | "astral_search")
        )
}

fn add_tools(response: &mut Value, eligible: &mut HashSet<String>) {
    let Some(tools) = response["result"]["tools"].as_array_mut() else {
        return;
    };
    if tools
        .iter()
        .any(|t| matches!(t["name"].as_str(), Some("astral_recall" | "astral_search")))
    {
        response.as_object_mut().unwrap().remove("result");
        response["error"] = json!({"code":-32603,"message":"Wrapped server uses reserved tool name astral_recall or astral_search"});
        return;
    }
    for tool in tools {
        if tool.get("outputSchema").is_none() {
            if let Some(name) = tool["name"].as_str() {
                eligible.insert(name.into());
            }
        }
    }
    // Append our tools once, on the final page, leaving upstream cursors intact.
    if response["result"]["nextCursor"].is_null() {
        response["result"]["tools"].as_array_mut().unwrap().extend(
            crate::recall::definitions()["tools"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
    }
}

async fn compact(
    client: &reqwest::Client,
    url: reqwest::Url,
    scope: String,
    tool: String,
    policy: crate::tool_intake::Policy,
    result: Value,
) -> anyhow::Result<Value> {
    let response = client
        .post(url)
        .json(&crate::tool_intake::Request {
            scope,
            tool,
            policy,
            result,
        })
        .send()
        .await?;
    ensure!(response.status().is_success(), "intake request failed");
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_MESSAGE,
            "intake response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let mut response: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        response["result"]["content"].is_array(),
        "invalid intake response"
    );
    Ok(response["result"].take())
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(pid) = self.0.id() {
            #[cfg(unix)]
            // SAFETY: the child created its own process group with this live PID.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = self.0.start_kill();
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! { _ = term.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

pub async fn serve(
    proxy: &str,
    session: Option<&str>,
    passthrough: bool,
    policy: crate::tool_intake::Policy,
    command: &[OsString],
) -> anyhow::Result<()> {
    let url = local_url(proxy)?;
    ensure!(
        !command.is_empty(),
        "an MCP server command is required after --"
    );
    let scope = if let Some(session) = session {
        ensure!(
            !session.is_empty() && session.len() <= 1024,
            "invalid intake session ID"
        );
        crate::fingerprint(&json!([
            "mcp-intake-v1",
            std::env::current_dir()?.as_os_str().as_encoded_bytes(),
            command
                .iter()
                .map(|s| s.as_encoded_bytes())
                .collect::<Vec<_>>(),
            session
        ]))
    } else {
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| anyhow::anyhow!("intake entropy unavailable"))?;
        crate::hash(&nonce)
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()?;
    let mut child = Command::new(&command[0]);
    child
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        child.as_std_mut().process_group(0);
    }
    let mut child = Process(child.spawn().context("cannot start wrapped MCP server")?);
    let mut child_input = child.0.stdin.take().unwrap();
    let (mut from_child, child_reader) = reader(child.0.stdout.take().unwrap());
    let (mut from_client, client_reader) = reader(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    let mut pending = HashMap::new();
    let mut active = HashSet::new();
    let mut eligible = HashSet::new();
    let mut initialized = false;
    let mut child_open = true;
    let mut jobs = JoinSet::new();
    let signal = shutdown_signal();
    tokio::pin!(signal);
    let result: anyhow::Result<()> = async {
        loop {
            if !child_open && jobs.is_empty() {
                ensure!(pending.is_empty(), "wrapped MCP server closed with pending requests");
                break;
            }
            tokio::select! {
                _ = &mut signal => break,
                message = from_client.recv() => {
                    let Some(bytes) = message else { break; };
                    let bytes = bytes?;
                    let request: Value = serde_json::from_slice(&bytes).context("invalid client MCP JSON")?;
                    if request.get("method").is_some() {
                        if let Some(key) = id(&request) {
                            ensure!(active.len() < MAX_PENDING, "too many pending MCP requests");
                            ensure!(active.insert(key.clone()), "duplicate active MCP request ID");
                            if own_tool(&request) {
                                let proxy = proxy.to_owned();
                                let mut initialized = initialized;
                                jobs.spawn(async move {
                                    let response = crate::recall::dispatch(&proxy, request, &mut initialized).await.unwrap();
                                    (key, response)
                                });
                                continue;
                            }
                            let kind = match request["method"].as_str() {
                                Some("initialize") => Pending::Initialize,
                                Some("tools/list") => {
                                    if request["params"]["cursor"].is_null() { eligible.clear(); }
                                    Pending::List
                                },
                                Some("tools/call") => {
                                    let tool = request["params"]["name"].as_str().unwrap_or("").to_owned();
                                    Pending::Call { eligible: eligible.contains(&tool), tool }
                                },
                                _ => Pending::Other,
                            };
                            pending.insert(key, kind);
                        }
                    }
                    // Responses to server requests, notifications and cancellation pass through.
                    write(&mut child_input, &bytes).await?;
                },
                message = from_child.recv(), if child_open && jobs.len() < MAX_JOBS => {
                    let Some(bytes) = message else { child_open = false; continue; };
                    let bytes = bytes?;
                    let mut response: Value = serde_json::from_slice(&bytes).context("invalid server MCP JSON")?;
                    if response["method"] == "notifications/tools/list_changed" { eligible.clear(); }
                    if response.get("method").is_none() {
                        if let Some(key) = id(&response) {
                            match pending.remove(&key) {
                                Some(Pending::Initialize) => {
                                    initialized = response["result"]["protocolVersion"].is_string();
                                    active.remove(&key);
                                },
                                Some(Pending::List) => {
                                    add_tools(&mut response, &mut eligible);
                                    active.remove(&key);
                                    write(&mut output, &serde_json::to_vec(&response)?).await?;
                                    continue;
                                },
                                Some(Pending::Call { tool, eligible: true }) if !passthrough && crate::tool_intake::text(&response["result"]).is_some() => {
                                    let (client, url, scope) = (client.clone(), url.clone(), scope.clone());
                                    jobs.spawn(async move {
                                        match compact(&client, url, scope, tool, policy, response["result"].clone()).await {
                                            Ok(result) => response["result"] = result,
                                            Err(_) => eprintln!("astral: intake unavailable; preserving original tool result"),
                                        }
                                        (key, response)
                                    });
                                    continue;
                                },
                                _ => { active.remove(&key); },
                            }
                        }
                    }
                    write(&mut output, &bytes).await?;
                },
                job = jobs.join_next(), if !jobs.is_empty() => {
                    let (key, response) = job.unwrap().context("MCP result task failed")?;
                    active.remove(&key);
                    write(&mut output, &serde_json::to_vec(&response)?).await?;
                },
            }
        }
        Ok(())
    }.await;
    jobs.abort_all();
    child_reader.abort();
    client_reader.abort();
    drop(child_input);
    // Kill the owned process group before reaping the parent, including descendants.
    if let Some(pid) = child.0.id() {
        #[cfg(unix)]
        // SAFETY: this is the owned child's process group, before waiting/reaping it.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.0.kill().await;
    result
}
