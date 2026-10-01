use astral::{
    config::{Config, Mode},
    providers::Wire,
    proxy::{App, router},
    usage::Observer,
};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{OriginalUri, State, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    process::Stdio,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    task::JoinHandle,
};

const ANTHROPIC_SSE: &str = "event: message_start\r\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":20,\"cache_read_input_tokens\":70,\"output_tokens\":1}}}\r\n\r\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"héllo 世界\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":9}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
const WS_RESPONSE: &str = "{  \"type\":\"response.completed\", \"response\":{\"id\":\"r1\",\"status\":\"completed\",\"output\":[]} }";

#[derive(Clone)]
struct Record {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}
type Records = Arc<Mutex<Vec<Record>>>;

async fn mock(
    State(records): State<Records>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    records.lock().unwrap().push(Record {
        path: uri.to_string(),
        headers: headers.clone(),
        body: body.to_vec(),
    });
    if headers.contains_key("x-test-fail") {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "3")],
            "exact upstream failure",
        )
            .into_response();
    }
    if uri.path().ends_with("count_tokens") {
        return axum::Json(json!({"input_tokens":111})).into_response();
    }
    if uri.path().ends_with("models") {
        return axum::Json(json!({"data":[{"id":"arbitrary-model"}]})).into_response();
    }
    if uri.path().ends_with("messages") {
        let chunks: Vec<Result<Bytes, std::io::Error>> = ANTHROPIC_SSE
            .as_bytes()
            .chunks(3)
            .map(|b| Ok(Bytes::copy_from_slice(b)))
            .collect();
        return Response::builder()
            .header("content-type", "text/event-stream")
            .header("request-id", "anthropic-fixture")
            .body(Body::from_stream(futures_util::stream::iter(chunks)))
            .unwrap();
    }
    axum::Json(json!({"id":"r","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":1}})).into_response()
}
async fn ws_mock(
    State(records): State<Records>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        if headers.contains_key("x-test-close") {
            socket
                .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                    code: 1013,
                    reason: "retry later".into(),
                })))
                .await
                .unwrap();
            return;
        }
        while let Some(Ok(message)) = socket.next().await {
            match message {
                Message::Text(text) => {
                    records.lock().unwrap().push(Record {
                        path: "ws".into(),
                        headers: headers.clone(),
                        body: text.as_bytes().to_vec(),
                    });
                    if socket
                        .send(Message::Text(WS_RESPONSE.into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Message::Binary(bytes) => {
                    if socket.send(Message::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
                Message::Close(frame) => {
                    records.lock().unwrap().push(Record {
                        path: "ws-close".into(),
                        headers: headers.clone(),
                        body: serde_json::to_vec(&frame.map(
                            |frame| json!({"code":frame.code,"reason":frame.reason.as_str()}),
                        ))
                        .unwrap(),
                    });
                    break;
                }
                _ => break,
            }
        }
    })
    .into_response()
}

struct Fixture {
    root: tempfile::TempDir,
    url: String,
    records: Records,
    tasks: Vec<JoinHandle<()>>,
    config: Config,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        let root = tempfile::tempdir().unwrap();
        let records = Records::default();
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", upstream.local_addr().unwrap());
        let routes = Router::new()
            .route("/oa/responses", get(ws_mock).post(mock))
            .fallback(any(mock))
            .with_state(records.clone());
        let upstream_task = tokio::spawn(async move {
            axum::serve(upstream, routes).await.unwrap();
        });
        let mut config = Config::parse_from([
            "test",
            "--keep-recent-turns",
            "1",
            "--tool-result-bytes",
            "2048",
            "--tool-preview-bytes",
            "128",
        ]);
        config.upstream = format!("{base}/oa");
        config.anthropic_upstream = format!("{base}/anthropic");
        config.state_dir = root.path().join("state");
        config.mode = mode;
        let provider_file = root.path().join("providers.toml");
        std::fs::write(
            &provider_file,
            format!(
                "[[providers]]\nname = 'local'\nprotocol = 'openai'\nupstream = '{base}/local'\n"
            ),
        )
        .unwrap();
        config.providers = Some(provider_file);
        let app = App::new(config.clone()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let proxy_task = tokio::spawn(async move {
            axum::serve(listener, router(app)).await.unwrap();
        });
        Self {
            root,
            url,
            records,
            tasks: vec![upstream_task, proxy_task],
            config,
        }
    }
    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .post(format!("{}{path}", self.url))
            .header("authorization", "Bearer fixture")
            .header("x-astral-workspace-id", "repo-a")
    }
    async fn restart(&mut self) {
        let task = self.tasks.pop().unwrap();
        task.abort();
        let _ = task.await;
        let app = App::new(self.config.clone()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        self.url = format!("http://{}", listener.local_addr().unwrap());
        self.tasks.push(tokio::spawn(async move {
            axum::serve(listener, router(app)).await.unwrap();
        }));
    }
}

fn payload() -> String {
    format!(
        "{}middle-needle{}",
        "prefix ".repeat(800),
        " suffix".repeat(800)
    )
}
fn responses() -> Value {
    json!({"model":"unrecognized-local-model","input":[
        {"role":"user","content":"start"},{"type":"function_call","call_id":"one","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"one","output":payload()},{"role":"assistant","content":"done"},{"role":"user","content":"next"}
    ]})
}
fn messages() -> Value {
    json!({"model":"claude-any","max_tokens":40,"stream":true,"messages":[
        {"role":"user","content":"start"},{"role":"assistant","content":[{"type":"tool_use","id":"one","name":"read","input":{}}]},
        {"role":"user","content":[{"type":"tool_result","tool_use_id":"one","content":payload()}]},{"role":"assistant","content":"done"},{"role":"user","content":"next"}
    ]})
}
fn token(text: &str) -> String {
    text.split("handle=")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn one_listener_routes_both_providers_preserves_streams_and_can_recall_after_restart() {
    let mut f = Fixture::new(Mode::Tools).await;
    let client = reqwest::Client::new();
    let response = f
        .post("/v1/responses?trace=yes")
        .json(&responses())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.bytes().await.unwrap();
    let response = client
        .post(format!("{}/v1/messages?beta=true", f.url))
        .header("x-api-key", "anthropic-only")
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "unchanged-beta")
        .json(&messages())
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["request-id"], "anthropic-fixture");
    assert_eq!(response.bytes().await.unwrap(), ANTHROPIC_SSE.as_bytes());
    let records = f.records.lock().unwrap().clone();
    assert_eq!(records[0].path, "/oa/responses?trace=yes");
    assert_eq!(records[1].path, "/anthropic/messages?beta=true");
    assert_eq!(records[1].headers["x-api-key"], "anthropic-only");
    assert!(!records[1].headers.contains_key("authorization"));
    assert_eq!(records[1].headers["anthropic-beta"], "unchanged-beta");
    assert!(!records[0].headers.contains_key("x-astral-workspace-id"));
    let oa: Value = serde_json::from_slice(&records[0].body).unwrap();
    let anth: Value = serde_json::from_slice(&records[1].body).unwrap();
    let handle = token(oa["input"][2]["output"].as_str().unwrap());
    assert!(
        anth["messages"][2]["content"][0]["content"]
            .as_str()
            .unwrap()
            .contains("astral archived")
    );
    f.restart().await;
    let recalled = astral::recall::fetch(&f.url, &handle, 0, 16384, None)
        .await
        .unwrap();
    assert_eq!(recalled["data"], payload());
    let ledger = std::fs::read_to_string(f.root.path().join("state/ledger.jsonl")).unwrap();
    assert!(!ledger.contains("anthropic-only"));
    assert!(!ledger.contains("middle-needle"));
    let row = ledger
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["provider"] == "anthropic" && v["kind"] == "response")
        .unwrap();
    assert_eq!(row["usage"]["input_tokens"], 100);
    assert_eq!(row["usage"]["output_tokens"], 9);
    assert_eq!(row["outcome"], "completed");
}

#[tokio::test]
async fn named_routes_chat_counting_errors_compressed_and_passthrough_preserve_contracts() {
    let f = Fixture::new(Mode::Tools).await;
    let chat = json!({"model":"arbitrary","messages":[{"role":"user","content":"start"},{"role":"assistant","tool_calls":[{"id":"a","type":"function","function":{"name":"read","arguments":"{}"}}]},{"role":"tool","tool_call_id":"a","content":payload()},{"role":"assistant","content":"done"},{"role":"user","content":"next"}]});
    f.post("/providers/local/v1/chat/completions")
        .json(&chat)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let raw = serde_json::to_vec(&messages()).unwrap();
    f.post("/providers/anthropic/v1/messages/count_tokens?q=1")
        .body(raw.clone())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let failed = f
        .post("/providers/anthropic/v1/messages")
        .header("x-test-fail", "1")
        .json(&messages())
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status(), 429);
    assert_eq!(failed.headers()["retry-after"], "3");
    assert_eq!(failed.text().await.unwrap(), "exact upstream failure");
    f.post("/v1/responses")
        .header("content-encoding", "gzip")
        .body("opaque-compressed-fixture")
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let records = f.records.lock().unwrap().clone();
    assert_eq!(records[0].path, "/local/chat/completions");
    assert!(String::from_utf8_lossy(&records[0].body).contains("astral archived"));
    assert_eq!(records[1].path, "/anthropic/messages/count_tokens?q=1");
    assert_eq!(records[1].body, raw);
    assert_eq!(records[3].body, b"opaque-compressed-fixture");
    assert_eq!(records[3].headers["content-encoding"], "gzip");
    let p = Fixture::new(Mode::Passthrough).await;
    p.post("/v1/messages")
        .body(raw.clone())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(p.records.lock().unwrap()[0].body, raw);
}

#[tokio::test]
async fn isolation_duplicates_and_browser_retrieval_are_enforced() {
    let f = Fixture::new(Mode::Tools).await;
    for workspace in ["one", "two"] {
        reqwest::Client::new()
            .post(format!("{}/v1/responses", f.url))
            .header("authorization", "Bearer same")
            .header("x-astral-workspace-id", workspace)
            .header("x-astral-session-id", "same")
            .json(&responses())
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    }
    let rows = f.records.lock().unwrap().clone();
    let handles: Vec<_> = rows
        .iter()
        .map(|r| {
            let v: Value = serde_json::from_slice(&r.body).unwrap();
            token(v["input"][2]["output"].as_str().unwrap())
        })
        .collect();
    assert_ne!(handles[0], handles[1]);
    let origin = reqwest::Client::new()
        .get(format!("{}/_astral/artifacts/{}", f.url, handles[0]))
        .header("origin", "https://example.com")
        .send()
        .await
        .unwrap();
    assert_eq!(origin.status(), 403);
    let duplicate = f
        .post("/v1/messages")
        .header("authorization", "Bearer second")
        .json(&messages())
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), 400);
    let looped = f
        .post("/v1/messages")
        .header("x-astral-hop", "1")
        .json(&messages())
        .send()
        .await
        .unwrap();
    assert_eq!(looped.status(), 508);
    assert_eq!(f.records.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn ordinary_websocket_reduces_full_history_but_forwards_incremental_and_binary() {
    let f = Fixture::new(Mode::Tools).await;
    let url = f.url.replacen("http://", "ws://", 1) + "/v1/responses";
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let mut request = responses();
    request["type"] = json!("response.create");
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            request.to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap().into_text().unwrap(),
        WS_RESPONSE
    );
    request["previous_response_id"] = json!("r1");
    let raw = request.to_string();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            raw.clone().into(),
        ))
        .await
        .unwrap();
    socket.next().await.unwrap().unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Binary(
            Bytes::from_static(b"binary"),
        ))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap().into_data(),
        b"binary"[..]
    );
    let rows = f.records.lock().unwrap().clone();
    assert!(String::from_utf8_lossy(&rows[0].body).contains("astral archived"));
    assert_eq!(rows[1].body, raw.as_bytes());
}

#[tokio::test]
async fn ordinary_websocket_preserves_close_metadata_in_both_directions() {
    use tokio_tungstenite::tungstenite::{
        Message as ClientMessage, client::IntoClientRequest, protocol::CloseFrame,
    };
    let f = Fixture::new(Mode::Tools).await;
    let url = f.url.replacen("http://", "ws://", 1) + "/v1/responses";
    let mut request = url.clone().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("x-test-close", "1".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let received = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ClientMessage::Close(Some(frame)) = received else {
        panic!("upstream close metadata was lost: {received:?}");
    };
    assert_eq!(u16::from(frame.code), 1013);
    assert_eq!(frame.reason, "retry later");

    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    socket
        .send(ClientMessage::Close(Some(CloseFrame {
            code: 1001.into(),
            reason: "client done".into(),
        })))
        .await
        .unwrap();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(record) = f
                .records
                .lock()
                .unwrap()
                .iter()
                .find(|record| record.path == "ws-close")
            {
                break serde_json::from_slice::<Value>(&record.body).unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(frame, json!({"code":1001,"reason":"client done"}));
}

#[tokio::test]
async fn mcp_client_can_search_and_retrieve_a_hidden_detail() {
    let f = Fixture::new(Mode::Tools).await;
    f.post("/v1/responses")
        .json(&responses())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let record = f.records.lock().unwrap()[0].clone();
    let request: Value = serde_json::from_slice(&record.body).unwrap();
    let handle = token(request["input"][2]["output"].as_str().unwrap());
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_astral"))
        .args(["mcp", "--proxy-url", &f.url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = tokio::io::BufReader::new(child.stdout.take().unwrap());
    async fn rpc(
        input: &mut tokio::process::ChildStdin,
        output: &mut tokio::io::BufReader<tokio::process::ChildStdout>,
        message: Value,
    ) -> Value {
        input
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        let mut line = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            output.read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    assert!(rpc(&mut input,&mut output,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}})).await["result"]["capabilities"]["tools"].is_object());
    let tools = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 2);
    let searched=rpc(&mut input,&mut output,json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"astral_search","arguments":{"handle":handle,"query":"middle-needle"}}})).await;
    let found: Value =
        serde_json::from_str(searched["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let recalled=rpc(&mut input,&mut output,json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"astral_recall","arguments":{"handle":handle,"offset":found["matches"][0]["offset"],"limit":13}}})).await;
    let content: Value =
        serde_json::from_str(recalled["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(content["data"], "middle-needle");
    drop(input);
    assert!(child.wait().await.unwrap().success());
}

#[test]
fn observers_distinguish_anthropic_usage_and_incomplete_or_error_streams() {
    for size in 1..20 {
        let mut observer = Observer::with_wire(true, 65536, Wire::Messages);
        for chunk in ANTHROPIC_SSE.as_bytes().chunks(size) {
            observer.feed(chunk);
        }
        observer.finish();
        assert!(observer.completed());
        let usage = observer.usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.cached_tokens, 70);
        assert_eq!(usage.output_tokens, 9);
    }
    let mut error = Observer::with_wire(true, 65536, Wire::Messages);
    error.feed(b"data: {\"type\":\"error\"}\n\ndata: {\"type\":\"message_stop\"}\n\n");
    error.finish();
    assert!(!error.completed());
    let mut partial = Observer::with_wire(true, 65536, Wire::Messages);
    partial.feed(b"data: {\"type\":\"message_stop\"}");
    partial.finish();
    assert!(!partial.completed());
    let mut chat = Observer::with_wire(true, 65536, Wire::Chat);
    chat.feed(b"data: {\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":90}}}\n\ndata: [DONE]\n\n");
    chat.finish();
    assert!(chat.completed());
    assert_eq!(chat.usage.unwrap().input_tokens, 100);
}

#[tokio::test]
async fn environment_credentials_are_provider_scoped_and_never_inherited_by_custom_routes() {
    let f = Fixture::new(Mode::Tools).await;
    let child_state = f.root.path().join("child-state");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_astral"))
        .args([
            "proxy",
            "--listen",
            "127.0.0.1:0",
            "--state-dir",
            child_state.to_str().unwrap(),
            "--providers",
            f.config.providers.as_ref().unwrap().to_str().unwrap(),
        ])
        .env("ASTRAL_UPSTREAM", &f.config.upstream)
        .env("ASTRAL_ANTHROPIC_UPSTREAM", &f.config.anthropic_upstream)
        .env("OPENAI_API_KEY", "openai-fallback-fixture")
        .env("ANTHROPIC_API_KEY", "anthropic-fallback-fixture")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stderr = tokio::io::BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stderr.read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    let address = line
        .trim()
        .split("listening on ")
        .nth(1)
        .expect("proxy startup address");
    let client = reqwest::Client::new();
    for path in [
        "/v1/responses",
        "/v1/messages",
        "/providers/local/v1/responses",
    ] {
        client
            .post(format!("http://{address}{path}"))
            .json(&json!({"model":"any","input":[],"messages":[]}))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    }
    let rows = f.records.lock().unwrap().clone();
    assert_eq!(
        rows[0].headers["authorization"],
        "Bearer openai-fallback-fixture"
    );
    assert!(!rows[0].headers.contains_key("x-api-key"));
    assert_eq!(rows[1].headers["x-api-key"], "anthropic-fallback-fixture");
    assert!(!rows[1].headers.contains_key("authorization"));
    assert!(!rows[2].headers.contains_key("authorization"));
    assert!(!rows[2].headers.contains_key("x-api-key"));
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}

#[test]
fn provider_configuration_rejects_ambiguous_routes_and_credential_urls() {
    for input in [
        "[[providers]]\nname='bad/path'\nprotocol='openai'\nupstream='http://localhost/v1'",
        "[[providers]]\nname='local'\nprotocol='openai'\nupstream='https://secret@example.com/v1'",
        "[[providers]]\nname='anthropic'\nprotocol='openai'\nupstream='https://example.com/v1'",
        "[[providers]]\nname='local'\nprotocol='openai'\nupstream='https://example.com/v1?key=secret'",
        "[[providers]]\nname='local'\nprotocol='openai'\nupstream='http://localhost/v1'\nunknown=true",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("providers.toml");
        std::fs::write(&path, input).unwrap();
        let mut config = Config::parse_from(["test"]);
        config.providers = Some(path);
        assert!(astral::providers::load(&config).is_err());
    }
}
