use astral::{
    config::Config,
    proxy::{App, router},
};
use clap::Parser;
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
    task::JoinHandle,
};

struct Proxy {
    root: tempfile::TempDir,
    url: String,
    task: JoinHandle<()>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Proxy {
    async fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut config = Config::parse_from(["astral", "--mode", "passthrough"]);
        config.state_dir = root.path().into();
        let app = App::new(config).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router(app)).await.unwrap();
        });
        Self { root, url, task }
    }
}

struct Mcp {
    child: Child,
    input: Option<ChildStdin>,
    output: Lines<BufReader<ChildStdout>>,
}
impl Mcp {
    async fn start(url: &str, mode: &str, pass: bool) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_astral"));
        cmd.args([
            "mcp-wrap",
            "--proxy-url",
            url,
            "--session-id",
            "test",
            "--policy",
            "preview",
        ]);
        if pass {
            cmd.arg("--passthrough");
        }
        cmd.args([
            "--",
            "python3",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/intake_mcp.py"),
            mode,
        ]);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut mcp = Self {
            child,
            input,
            output,
        };
        mcp.send(json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).await;
        assert_eq!(mcp.next().await["result"]["protocolVersion"], "2025-06-18");
        mcp
    }
    async fn send(&mut self, value: Value) {
        let input = self.input.as_mut().unwrap();
        input
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
        input.flush().await.unwrap();
    }
    async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(10), self.output.next_line())
            .await
            .unwrap()
            .unwrap()
            .expect("MCP response");
        serde_json::from_str(&line).unwrap()
    }
    async fn list(&mut self) {
        self.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .await;
        let first = self.next().await;
        assert_eq!(first["result"]["nextCursor"], "second-page");
        assert!(
            !first["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == "astral_recall")
        );
        self.send(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"cursor":"second-page"}}),
        )
        .await;
        let second = self.next().await;
        let names: Vec<_> = second["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["schema", "astral_recall", "astral_search"]);
        assert!(second["result"]["tools"][0]["outputSchema"].is_object());
    }
    async fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":arguments}})).await;
        self.next().await
    }
    async fn stop(mut self) {
        self.input.take();
        let status = tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success());
    }
}

fn large() -> Value {
    json!({"content":[{"type":"text","text":format!("HEAD\n{}\nneedle-hidden-世界\n{}\nTAIL", "routine line\n".repeat(1600), "routine line\n".repeat(1600))}],"isError":false})
}
fn handle(response: &Value) -> String {
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .split("handle=")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn wrapper_compacts_first_read_and_exposes_exact_targeted_retrieval() {
    let proxy = Proxy::start().await;
    let mut mcp = Mcp::start(&proxy.url, "normal", false).await;
    mcp.list().await;
    let source = large();
    let first = mcp.call("plain", json!({"result":source})).await;
    assert!(!first.to_string().contains("needle-hidden"));
    let handle = handle(&first);
    let again = mcp.call("plain", json!({"result":source})).await;
    assert_eq!(again, first);
    let search = mcp
        .call(
            "astral_search",
            json!({"handle":handle,"query":"needle-hidden"}),
        )
        .await;
    let found: Value =
        serde_json::from_str(search["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let offset = found["matches"][0]["offset"].as_u64().unwrap();
    let recall = mcp
        .call(
            "astral_recall",
            json!({"handle":handle,"offset":offset,"limit":200}),
        )
        .await;
    let page: Value =
        serde_json::from_str(recall["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        page["data"]
            .as_str()
            .unwrap()
            .starts_with("needle-hidden-世界")
    );
    let artifact: Value = serde_json::from_slice(
        &std::fs::read(proxy.root.path().join(format!("artifacts/{handle}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(artifact["data"].as_str().unwrap()).unwrap(),
        source
    );
    let ledger = std::fs::read_to_string(proxy.root.path().join("ledger.jsonl")).unwrap();
    assert!(!ledger.contains("needle-hidden") && !ledger.contains(&handle));
    assert_eq!(
        ledger.lines().filter(|l| l.contains("tool_intake")).count(),
        2
    );
    mcp.stop().await;
}

#[tokio::test]
async fn wrapper_preserves_passthrough_schema_contracts_and_unavailable_archive() {
    let proxy = Proxy::start().await;
    let source = large();
    for (url, pass, tool) in [
        (&proxy.url, true, "plain"),
        (&proxy.url, false, "schema"),
        (&"http://127.0.0.1:1".to_owned(), false, "plain"),
    ] {
        let mut mcp = Mcp::start(url, "normal", pass).await;
        mcp.list().await;
        assert_eq!(
            mcp.call(tool, json!({"result":source})).await["result"],
            source
        );
        mcp.stop().await;
    }
}

#[tokio::test]
async fn wrapper_relays_parallel_results_progress_cancellation_and_server_requests() {
    let proxy = Proxy::start().await;
    let mut mcp = Mcp::start(&proxy.url, "normal", false).await;
    mcp.list().await;
    mcp.send(json!({"jsonrpc":"2.0","id":"first","method":"tools/call","params":{"name":"reverse","arguments":{"result":large()}}})).await;
    assert_eq!(mcp.next().await["method"], "notifications/progress");
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"unrelated","reason":"test"}})).await;
    assert_eq!(mcp.next().await["params"]["requestId"], "unrelated");
    mcp.send(json!({"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"reverse","arguments":{"result":large()}}})).await;
    let a = mcp.next().await;
    let b = mcp.next().await;
    assert_eq!(
        std::collections::HashSet::from([a["id"].to_string(), b["id"].to_string()]),
        std::collections::HashSet::from([json!("first").to_string(), "99".into()])
    );
    assert_eq!(handle(&a), handle(&b));
    mcp.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"roots","arguments":{}}})).await;
    let server_request = mcp.next().await;
    assert_eq!(server_request["method"], "roots/list");
    mcp.send(json!({"jsonrpc":"2.0","id":4,"result":{"roots":[]}}))
        .await;
    let returned = mcp.next().await;
    assert_eq!(returned["id"], 4);
    let echoed: Value =
        serde_json::from_str(returned["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(echoed["result"]["roots"], json!([]));
    mcp.stop().await;
}

#[tokio::test]
async fn wrapper_drains_last_result_when_server_exits_and_rejects_name_collision() {
    let proxy = Proxy::start().await;
    let mut mcp = Mcp::start(&proxy.url, "exit", false).await;
    mcp.list().await;
    assert!(!handle(&mcp.call("plain", json!({"result":large()})).await).is_empty());
    let status = tokio::time::timeout(Duration::from_secs(5), mcp.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    let mut mcp = Mcp::start(&proxy.url, "collision", false).await;
    mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .await;
    assert_eq!(mcp.next().await["error"]["code"], -32603);
    mcp.stop().await;
}

#[tokio::test]
async fn intake_http_rejects_browser_access_and_non_loopback_wrapper_urls() {
    let proxy = Proxy::start().await;
    let response = reqwest::Client::new()
        .post(format!("{}/_astral/intake", proxy.url))
        .header("origin", "https://example.com")
        .json(&json!({"scope":astral::hash(b"one"),"tool":"plain","result":large()}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let result = Command::new(env!("CARGO_BIN_EXE_astral"))
        .args([
            "mcp-wrap",
            "--proxy-url",
            "https://example.com",
            "--",
            "false",
        ])
        .output()
        .await
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}

#[tokio::test]
async fn wrapper_reports_server_exit_with_an_unanswered_call() {
    let proxy = Proxy::start().await;
    let mut mcp = Mcp::start(&proxy.url, "crash", false).await;
    mcp.list().await;
    mcp.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"plain","arguments":{}}})).await;
    let status = tokio::time::timeout(Duration::from_secs(5), mcp.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
}

#[cfg(unix)]
#[tokio::test]
async fn wrapper_termination_stops_its_child_group() {
    let proxy = Proxy::start().await;
    let mut mcp = Mcp::start(&proxy.url, "normal", false).await;
    mcp.list().await;
    let result = mcp.call("spawn", json!({})).await;
    let descendant: i32 = result["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    // SAFETY: signal only our newly spawned wrapper, which owns the fixture group.
    unsafe {
        libc::kill(mcp.child.id().unwrap() as i32, libc::SIGTERM);
    }
    let status = tokio::time::timeout(Duration::from_secs(5), mcp.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    for _ in 0..100 {
        let output = Command::new("ps")
            .args(["-o", "stat=", "-p", &descendant.to_string()])
            .output()
            .await
            .unwrap();
        let state = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || state.trim().is_empty() || state.trim().starts_with('Z') {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("wrapped descendant survived termination");
}
