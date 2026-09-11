use axum::{
    Router,
    body::{Body, Bytes},
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use clap::Parser;
use ostk_gpt_cache::{
    config::{CompactionBackend, Config, Mode},
    proxy::{App, router},
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::task::JoinHandle;

const SSE: &[u8] = "event: response.output_text.delta\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\r\n\r\nevent: response.completed\r\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":100,\"input_tokens_details\":{\"cached_tokens\":80,\"cache_write_tokens\":20},\"output_tokens\":2}}}\r\n\r\ndata: [DONE]\r\n\r\n".as_bytes();

#[derive(Clone)]
struct Record {
    compact: bool,
    body: Vec<u8>,
    headers: HeaderMap,
}
#[derive(Default)]
struct Mock {
    records: Mutex<Vec<Record>>,
    compact_reply: Mutex<Option<Value>>,
    mode: AtomicUsize,
    compact_fail: AtomicUsize,
    delay: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    inline_mode: AtomicUsize,
    inline_count: AtomicUsize,
}
fn inline_output(number: usize) -> Vec<Value> {
    vec![
        json!({"type":"message","role":"assistant","phase":"commentary","content":"working"}),
        json!({"type":"compaction","id":format!("c{number}"),"encrypted_content":format!("opaque-{number}")}),
        json!({"type":"reasoning","encrypted_content":"reasoning+/=","summary":[]}),
        json!({"type":"message","role":"assistant","phase":"final_answer","content":"READY"}),
    ]
}
fn inline_wire(number: usize, complete: bool, missing_index: bool) -> Vec<u8> {
    let mut wire = Vec::new();
    for (index, item) in inline_output(number).iter().enumerate() {
        if missing_index && index == 0 {
            continue;
        }
        wire.extend(
            format!(
                "data: {}\n\n",
                json!({"type":"response.output_item.done","output_index":index,"item":item})
            )
            .bytes(),
        );
    }
    if complete {
        wire.extend(format!("data: {}\n\n", json!({"type":"response.completed","response":{"status":"completed","output":[],"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":0},"output_tokens":20}}})).bytes());
    }
    wire
}
async fn mock_compact(State(m): State<Arc<Mock>>, headers: HeaderMap, body: Bytes) -> Response {
    m.records.lock().unwrap().push(Record {
        compact: true,
        body: body.to_vec(),
        headers,
    });
    if m.compact_fail.load(Ordering::SeqCst) > 0 {
        return (StatusCode::SERVICE_UNAVAILABLE, "compact down").into_response();
    }
    if let Some(reply) = m.compact_reply.lock().unwrap().clone() {
        return axum::Json(reply).into_response();
    }
    axum::Json(json!({"object":"response.compaction", "output":[
        {"role":"user","content":"retained by provider"},
        {"type":"compaction","id":"cmp","encrypted_content":"opaque-state"}
    ], "usage":{"input_tokens":300,"input_tokens_details":{"cached_tokens":0},"output_tokens":20}}))
    .into_response()
}
async fn mock_response(State(m): State<Arc<Mock>>, headers: HeaderMap, body: Bytes) -> Response {
    m.records.lock().unwrap().push(Record {
        compact: false,
        body: body.to_vec(),
        headers,
    });
    let active = m.active.fetch_add(1, Ordering::SeqCst) + 1;
    m.peak.fetch_max(active, Ordering::SeqCst);
    let delay = m.delay.load(Ordering::SeqCst);
    if delay > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(delay as u64)).await;
    }
    m.active.fetch_sub(1, Ordering::SeqCst);
    let inline_mode = m.inline_mode.load(Ordering::SeqCst);
    if inline_mode > 0
        && serde_json::from_slice::<Value>(&body)
            .unwrap()
            .get("context_management")
            .is_some()
    {
        let number = m.inline_count.fetch_add(1, Ordering::SeqCst) + 1;
        let wire = inline_wire(
            number,
            inline_mode != 2 && inline_mode != 3,
            inline_mode == 4,
        );
        let stream = async_stream::stream! {
            for chunk in wire.chunks(7) { yield Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(chunk)); }
            if inline_mode == 3 { std::future::pending::<()>().await; }
        };
        return Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(stream))
            .unwrap();
    }
    match m.mode.load(Ordering::SeqCst) {
        1 | 2 | 5 => {
            let mode = m.mode.load(Ordering::SeqCst);
            let bytes = if mode == 2 { &SSE[..80] } else { SSE };
            let chunks: Vec<Result<Bytes, std::io::Error>> = bytes.chunks(7).map(Bytes::copy_from_slice).map(Ok).collect();
            let mut builder = Response::builder();
            if mode != 5 { builder = builder.header("content-type", "text/event-stream"); }
            builder.body(Body::from_stream(futures_util::stream::iter(chunks))).unwrap()
        }
        3 => (StatusCode::SERVICE_UNAVAILABLE, "try later").into_response(),
        6 => {
            let stream = async_stream::stream! {
                yield Ok::<Bytes, std::io::Error>(Bytes::from_static(b"data: {\"type\":\"response.created\"}\n\n"));
                std::future::pending::<()>().await;
            };
            Response::builder().header("content-type", "text/event-stream").body(Body::from_stream(stream)).unwrap()
        }
        _ => axum::Json(json!({"id":"resp", "status":"completed", "output":[], "usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":80,"cache_write_tokens":20},"output_tokens":2}})).into_response(),
    }
}

struct Fixture {
    root: tempfile::TempDir,
    url: String,
    mock: Arc<Mock>,
    proxy: JoinHandle<()>,
    upstream: JoinHandle<()>,
    config: Config,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.proxy.abort();
        self.upstream.abort();
    }
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        Self::configured(mode, |_| {}).await
    }
    async fn configured(mode: Mode, configure: impl FnOnce(&mut Config)) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mock = Arc::new(Mock::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let upstream_router = Router::new()
            .route("/v1/responses", post(mock_response))
            .route("/v1/responses/compact", post(mock_compact))
            .route("/v1/compact", post(mock_compact))
            .with_state(mock.clone());
        let upstream = tokio::spawn(async move {
            axum::serve(listener, upstream_router).await.unwrap();
        });
        let mut cfg = Config::parse_from([
            "test",
            "--roll-bytes",
            "400",
            "--min-compact-bytes",
            "10",
            "--keep-recent-turns",
            "1",
            "--min-roll-seconds",
            "0",
        ]);
        cfg.state_dir = root.path().join("state");
        cfg.upstream = upstream_url;
        cfg.allow_compatible_compaction = true;
        cfg.mode = mode;
        configure(&mut cfg);
        let app = App::new(cfg.clone()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
        let proxy = tokio::spawn(async move {
            axum::serve(listener, router(app)).await.unwrap();
        });
        Self {
            root,
            url,
            mock,
            proxy,
            upstream,
            config: cfg,
        }
    }
    async fn restart(&mut self) {
        self.proxy.abort();
        let _ = (&mut self.proxy).await;
        let app = App::new(self.config.clone()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        self.url = format!("http://{}/v1/responses", listener.local_addr().unwrap());
        self.proxy = tokio::spawn(async move {
            axum::serve(listener, router(app)).await.unwrap();
        });
    }
    fn request(&self, session: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .post(&self.url)
            .header("authorization", "Bearer test-only")
            .header("x-ostk-session-id", session)
            .header("accept", "text/event-stream")
    }
    fn records(&self, compact: bool) -> Vec<Record> {
        self.mock
            .records
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.compact == compact)
            .cloned()
            .collect()
    }
    async fn snapshots(&self) -> Vec<Value> {
        let mut values = Vec::new();
        let mut dir = tokio::fs::read_dir(self.root.path().join("state/lanes"))
            .await
            .unwrap();
        while let Some(e) = dir.next_entry().await.unwrap() {
            if e.path().extension().is_some_and(|ext| ext == "json") {
                values.push(
                    serde_json::from_slice(&tokio::fs::read(e.path()).await.unwrap()).unwrap(),
                );
            }
        }
        values
    }
}
fn input() -> Value {
    json!({"model":"gpt-5.5", "instructions":"keep this exact", "tools":[], "store":false, "input":[
        {"role":"user","content":"A".repeat(1000)},
        {"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"done","annotations":[]}]},
        {"role":"user","content":"continue"}
    ]})
}

#[tokio::test]
async fn rolling_reuses_entire_projection_and_preserves_main_response() {
    let f = Fixture::new(Mode::Rolling).await;
    let mut r = input();
    let response = f
        .request("one")
        .header("idempotency-key", "main-call")
        .json(&r)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let reply: Value = response.json().await.unwrap();
    assert_eq!(reply["usage"]["input_tokens"], 100);
    let first: Value = serde_json::from_slice(&f.records(false)[0].body).unwrap();
    assert_eq!(first["input"][0]["content"], "retained by provider");
    assert_eq!(first["input"][1]["encrypted_content"], "opaque-state");
    assert_eq!(first["input"][2], r["input"][2]);
    assert_eq!(first["instructions"], r["instructions"]);
    assert_eq!(first["tools"], r["tools"]);
    assert!(first.get("prompt_cache_retention").is_none()); // Local compatible wire.
    assert!(!f.records(true)[0].headers.contains_key("idempotency-key"));
    assert!(
        !f.records(false)[0]
            .headers
            .contains_key("x-ostk-session-id")
    );
    r["input"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"ok"}),
        json!({"role":"user","content":"again"}),
    ]);
    f.request("one")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let second: Value = serde_json::from_slice(&f.records(false)[1].body).unwrap();
    assert!(
        second["input"]
            .as_array()
            .unwrap()
            .starts_with(first["input"].as_array().unwrap())
    );
    assert_eq!(first["prompt_cache_key"], second["prompt_cache_key"]);
    assert_eq!(f.records(true).len(), 1);
    assert_eq!(f.snapshots().await.len(), 1);
}

#[tokio::test]
async fn failure_does_not_commit_and_retry_rebuilds_from_original() {
    let f = Fixture::new(Mode::Rolling).await;
    f.mock.mode.store(3, Ordering::SeqCst);
    let r = input();
    assert_eq!(
        f.request("one").json(&r).send().await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(f.snapshots().await.is_empty());
    f.mock.mode.store(0, Ordering::SeqCst);
    f.request("one")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(f.records(true).len(), 2);
    assert_eq!(f.snapshots().await[0]["epoch"], 1);
}

#[tokio::test]
async fn truncated_sse_does_not_commit_and_complete_sse_is_byte_identical() {
    let f = Fixture::new(Mode::Rolling).await;
    let mut r = input();
    r["stream"] = json!(true);
    f.mock.mode.store(2, Ordering::SeqCst);
    f.request("one")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert!(f.snapshots().await.is_empty());
    f.mock.mode.store(1, Ordering::SeqCst);
    let bytes = f
        .request("one")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&bytes[..], SSE);
    assert_eq!(f.snapshots().await.len(), 1);
}

#[tokio::test]
async fn missing_content_type_uses_stream_request_but_json_header_wins() {
    let f = Fixture::new(Mode::Rolling).await;
    let mut r = input();
    r["stream"] = json!(true);
    f.mock.mode.store(5, Ordering::SeqCst);
    let b = f
        .request("one")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&b[..], SSE);
    assert_eq!(f.snapshots().await.len(), 1);
    f.mock.mode.store(0, Ordering::SeqCst);
    let v: Value = f
        .request("two")
        .json(&r)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["status"], "completed");
    assert_eq!(f.snapshots().await.len(), 2);
}

#[tokio::test]
async fn true_passthrough_preserves_request_bytes_even_with_force_header() {
    let f = Fixture::new(Mode::Passthrough).await;
    let raw = b"{ \"model\" : \"gpt-5.6\", \"input\": [ {\"role\":\"user\",\"content\":\"<system-reminder>keep</system-reminder>\"} ] }\n";
    f.request("one")
        .header("x-ostk-roll", "1")
        .body(raw.as_slice())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(f.records(false)[0].body, raw);
    assert!(f.records(true).is_empty());
    assert!(f.snapshots().await.is_empty());
}

#[tokio::test]
async fn managed_context_and_missing_identity_are_byte_passthrough() {
    let f = Fixture::new(Mode::Rolling).await;
    for key in ["previous_response_id", "conversation", "context_management"] {
        let mut r = input();
        r[key] = json!("provider-managed");
        let raw = serde_json::to_vec(&r).unwrap();
        f.request("one")
            .body(raw.clone())
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(f.records(false).last().unwrap().body, raw);
    }
    let raw = serde_json::to_vec(&input()).unwrap();
    reqwest::Client::new()
        .post(&f.url)
        .body(raw.clone())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(f.records(false).last().unwrap().body, raw);
    assert!(f.records(true).is_empty());
    assert!(f.snapshots().await.is_empty());
}

#[tokio::test]
async fn compactor_failure_keeps_full_history() {
    let f = Fixture::new(Mode::Rolling).await;
    f.mock.compact_fail.store(1, Ordering::SeqCst);
    let r = input();
    f.request("one")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let sent: Value = serde_json::from_slice(&f.records(false)[0].body).unwrap();
    assert_eq!(sent["input"], r["input"]);
    assert_eq!(f.snapshots().await[0]["cut"], 0);
}

#[tokio::test]
async fn lanes_are_isolated_by_credentials_and_session_not_request_id() {
    let f = Fixture::new(Mode::Rolling).await;
    for (session, credential) in [("one", "key-a"), ("two", "key-a"), ("one", "key-b")] {
        reqwest::Client::new()
            .post(&f.url)
            .header("x-ostk-session-id", session)
            .header("authorization", credential)
            .header("x-client-request-id", "same-request-id")
            .json(&input())
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    }
    assert_eq!(f.records(true).len(), 3);
    assert_eq!(f.snapshots().await.len(), 3);
    let keys: std::collections::HashSet<Value> = f
        .records(false)
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap()["prompt_cache_key"].clone())
        .collect();
    assert_eq!(keys.len(), 3);
}

#[tokio::test]
async fn same_lane_serializes_while_different_lanes_run_concurrently() {
    let f = Fixture::new(Mode::Rolling).await;
    f.mock.delay.store(60, Ordering::SeqCst);
    let a = f.request("one").json(&input());
    let b = f.request("one").json(&input());
    let (a, b) = tokio::join!(
        async { a.send().await.unwrap().bytes().await.unwrap() },
        async { b.send().await.unwrap().bytes().await.unwrap() }
    );
    assert!(!a.is_empty() && !b.is_empty());
    assert_eq!(f.mock.peak.load(Ordering::SeqCst), 1);
    let a = f.request("two").json(&input());
    let b = f.request("three").json(&input());
    tokio::join!(
        async { a.send().await.unwrap().bytes().await.unwrap() },
        async { b.send().await.unwrap().bytes().await.unwrap() }
    );
    assert_eq!(f.mock.peak.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn persistence_survives_restart_and_process_lock_prevents_two_writers() {
    use ostk_gpt_cache::{engine::Lane, store::Store};
    let root = tempfile::tempdir().unwrap();
    let id = "a".repeat(64);
    let lane = Lane {
        version: 1,
        contract: "known".into(),
        cut: 0,
        projection: vec![],
        epoch: 7,
        ..Default::default()
    };
    {
        let store = Store::open(root.path().into(), 2, 1_000_000).await.unwrap();
        assert!(Store::open(root.path().into(), 2, 1_000_000).await.is_err());
        store.commit(&id, &lane).await.unwrap();
    }
    let store = Store::open(root.path().into(), 2, 1_000_000).await.unwrap();
    assert_eq!(store.lane(&id).await.unwrap().as_ref().unwrap().epoch, 7);
}

#[tokio::test]
async fn client_disconnect_releases_lane_without_committing() {
    use futures_util::StreamExt;
    let f = Fixture::new(Mode::Rolling).await;
    f.mock.mode.store(6, Ordering::SeqCst);
    let mut r = input();
    r["stream"] = json!(true);
    let response = f.request("one").json(&r).send().await.unwrap();
    let mut body = response.bytes_stream();
    assert!(body.next().await.unwrap().is_ok());
    drop(body);
    assert!(f.snapshots().await.is_empty());
    f.mock.mode.store(0, Ordering::SeqCst);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        f.request("one")
            .json(&r)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    })
    .await
    .expect("disconnected stream must release the session lock");
    assert_eq!(f.records(true).len(), 2);
    assert_eq!(f.snapshots().await[0]["epoch"], 1);
}

#[tokio::test]
async fn ambiguous_authentication_is_rejected_before_upstream() {
    let f = Fixture::new(Mode::Rolling).await;
    let r = f
        .request("one")
        .header("authorization", "second-token")
        .json(&input())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(f.records(false).is_empty());
    let health: Value = reqwest::Client::new()
        .get(f.url.replace("/v1/responses", "/healthz"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["requests_received"], 1);
}

#[tokio::test]
async fn recursive_projection_survives_a_proxy_restart_with_complete_tool_history() {
    let mut f = Fixture::new(Mode::Rolling).await;
    let mut r = input();
    f.request("recursive")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let first: Value = serde_json::from_slice(&f.records(false)[0].body).unwrap();
    let active = vec![
        json!({"type":"reasoning","id":"r1","encrypted_content":"untouched-reasoning","summary":[]}),
        json!({"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Reading both files","annotations":[]}]}),
        json!({"type":"function_call","call_id":"call-a","name":"read","arguments":"{\"path\":\"a\"}"}),
        json!({"type":"function_call","call_id":"call-b","name":"read","arguments":"{\"path\":\"b\"}"}),
        json!({"type":"function_call_output","call_id":"call-b","output":"B".repeat(1500)}),
        json!({"type":"function_call_output","call_id":"call-a","output":"A".repeat(1500)}),
    ];
    r["input"].as_array_mut().unwrap().extend(active.clone());
    f.request("recursive")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(
        f.records(true).len(),
        1,
        "a tool continuation must not roll"
    );
    let tool_request: Value = serde_json::from_slice(&f.records(false)[1].body).unwrap();
    assert!(tool_request["input"].as_array().unwrap().ends_with(&active));

    r["input"].as_array_mut().unwrap().extend([
        json!({"type":"message","role":"assistant","phase":"final_answer","content":"Both read"}),
        json!({"role":"user","content":"Next turn"}),
    ]);
    f.request("recursive")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(f.records(true).len(), 2);
    let compact: Value = serde_json::from_slice(&f.records(true)[1].body).unwrap();
    assert!(
        compact["input"]
            .as_array()
            .unwrap()
            .starts_with(first["input"].as_array().unwrap())
    );
    assert_eq!(
        &compact["input"].as_array().unwrap()[3..9],
        active.as_slice()
    );
    assert_eq!(f.snapshots().await[0]["epoch"], 2);
    let before_restart: Value = serde_json::from_slice(&f.records(false)[2].body).unwrap();
    f.config.roll_bytes = 1_000_000;
    f.restart().await;
    r["input"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"ok"}),
        json!({"role":"user","content":"After restart"}),
    ]);
    f.request("recursive")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let resumed: Value = serde_json::from_slice(&f.records(false)[3].body).unwrap();
    assert!(
        resumed["input"]
            .as_array()
            .unwrap()
            .starts_with(before_restart["input"].as_array().unwrap())
    );
    assert_eq!(
        resumed["prompt_cache_key"],
        before_restart["prompt_cache_key"]
    );
    assert_eq!(f.records(true).len(), 2);
    assert_eq!(f.snapshots().await[0]["epoch"], 2);
}

#[tokio::test]
async fn history_edit_after_roll_forwards_the_authoritative_branch() {
    let f = Fixture::new(Mode::Rolling).await;
    let mut r = input();
    f.request("branch")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    r["input"][0]["content"] = json!("B".repeat(1000));
    f.request("branch")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let sent: Value = serde_json::from_slice(&f.records(false)[1].body).unwrap();
    assert_eq!(sent["input"], r["input"]);
    assert_eq!(f.records(true).len(), 1, "do not compact a reset request");
    assert_eq!(f.snapshots().await[0]["cut"], 0);
    assert_eq!(f.snapshots().await[0]["projection"], json!([]));
}

#[tokio::test]
async fn unusable_compaction_outputs_never_replace_history() {
    let f = Fixture::new(Mode::Rolling).await;
    let r = input();
    for (i, output) in [
        json!([]),
        json!([{"role":"assistant","content":"a prose summary is not native state"}]),
        json!([
            {"type":"reasoning","encrypted_content":"opaque-reasoning","summary":[]},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"ordinary generation"}]}
        ]),
        json!([{"type":"compaction","encrypted_content":""}]),
        json!([{"type":"compaction","encrypted_content":"X".repeat(5000)}]),
    ]
    .into_iter()
    .enumerate()
    {
        *f.mock.compact_reply.lock().unwrap() = Some(json!({"output":output}));
        f.request(&format!("invalid-{i}"))
            .json(&r)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let sent: Value = serde_json::from_slice(&f.records(false)[i].body).unwrap();
        assert_eq!(sent["input"], r["input"]);
    }
    for state in f.snapshots().await {
        assert_eq!(state["cut"], 0);
        assert_eq!(state["epoch"], 0);
    }
    let ledger = tokio::fs::read_to_string(f.root.path().join("state/ledger.jsonl"))
        .await
        .unwrap();
    let rejections: Vec<Value> = ledger
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|event| event["kind"] == "compact")
        .collect();
    assert_eq!(rejections.len(), 5);
    assert_eq!(
        rejections[2]["rejection_reason"],
        "compact output has no encrypted compaction item"
    );
}

#[tokio::test]
async fn observation_overflow_forwards_bytes_without_committing() {
    let f = Fixture::configured(Mode::Rolling, |cfg| {
        cfg.max_observation_bytes = 64;
        cfg.roll_bytes = 100_000;
    })
    .await;
    f.mock.mode.store(1, Ordering::SeqCst);
    let mut r = input();
    r["stream"] = json!(true);
    let bytes = f
        .request("overflow")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(bytes.as_ref(), SSE);
    assert!(f.snapshots().await.is_empty());
    let ledger = tokio::fs::read_to_string(f.root.path().join("state/ledger.jsonl"))
        .await
        .unwrap();
    let event: Value = serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
    assert_eq!(event["observation_overflow"], true);
    assert_eq!(event["committed"], false);
}

#[tokio::test]
async fn oauth_accounts_and_compatible_api_keys_have_independent_lanes() {
    let f = Fixture::new(Mode::Rolling).await;
    for header in ["chatgpt-account-id", "api-key", "x-api-key"] {
        for value in ["scope-a", "scope-b"] {
            f.request("shared-session")
                .header(header, value)
                .json(&input())
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
        }
    }
    assert_eq!(f.snapshots().await.len(), 6);
    let keys: std::collections::HashSet<Value> = f
        .records(false)
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap()["prompt_cache_key"].clone())
        .collect();
    assert_eq!(keys.len(), 6);
    for header in ["chatgpt-account-id", "api-key", "x-api-key"] {
        let response = f
            .request("ambiguous")
            .header(header, "scope-a")
            .header(header, "scope-b")
            .json(&input())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(f.records(false).len(), 6);
}

#[tokio::test]
async fn invalid_configured_ca_bundle_fails_startup_instead_of_disabling_tls_validation() {
    let root = tempfile::tempdir().unwrap();
    for (index, contents) in [
        None,
        Some(""),
        Some("not a certificate"),
        Some("-----BEGIN CERTIFICATE-----\ninvalid-base64\n-----END CERTIFICATE-----\n"),
    ]
    .into_iter()
    .enumerate()
    {
        let path = root.path().join(format!("ca-{index}.pem"));
        if let Some(contents) = contents {
            tokio::fs::write(&path, contents).await.unwrap();
        }
        let mut cfg = Config::parse_from(["test"]);
        cfg.state_dir = root.path().join(format!("state-{index}"));
        cfg.upstream_ca_bundle = Some(path);
        assert!(App::new(cfg).await.is_err());
    }
}

#[tokio::test]
async fn custom_compact_path_handles_autonomous_and_caller_compaction() {
    let f = Fixture::configured(Mode::Rolling, |cfg| {
        cfg.compact_path = "/compact".into();
    })
    .await;
    f.request("custom-route")
        .json(&input())
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(f.records(true).len(), 1);
    assert!(f.snapshots().await[0]["cut"].as_u64().unwrap() > 0);
    let raw = b"{ \"model\": \"gpt-6-astra\", \"input\": [] }";
    let result = reqwest::Client::new()
        .post(f.url.replace("/v1/responses", "/responses/compact"))
        .header("content-type", "application/json")
        .body(raw.as_slice())
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    result.bytes().await.unwrap();
    assert_eq!(f.records(true)[1].body, raw);
    for path in [
        "https://other.example/compact",
        "//other.example/compact",
        "/compact?secret=value",
    ] {
        let mut cfg = f.config.clone();
        cfg.compact_path = path.into();
        assert!(cfg.validate().is_err());
    }
}

#[tokio::test]
async fn scheduled_inline_maps_original_history_across_two_checkpoints_and_restart() {
    let mut f = Fixture::configured(Mode::Rolling, |cfg| {
        cfg.compaction_backend = CompactionBackend::Inline;
        cfg.roll_bytes = 100_000;
    })
    .await;
    f.mock.inline_mode.store(1, Ordering::SeqCst);
    let mut r = input();
    r["prompt_cache_key"] = json!("stable-caller-key");
    r["prompt_cache_options"] = json!({"mode":"explicit","ttl":"30m"});
    let bytes = f
        .request("inline")
        .header("x-ostk-roll", "1")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(bytes.as_ref(), inline_wire(1, true, false));
    assert_eq!(f.snapshots().await[0]["cut"], 5);
    r["input"].as_array_mut().unwrap().extend(inline_output(1));
    let active = vec![
        json!({"role":"user","content":"Read a file"}),
        json!({"type":"reasoning","encrypted_content":"active-opaque","summary":[]}),
        json!({"type":"function_call","call_id":"tool-1","name":"read","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"tool-1","output":"X".repeat(1000)}),
    ];
    r["input"].as_array_mut().unwrap().extend(active.clone());
    f.request("inline")
        .header("x-ostk-roll", "1")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let sent: Value = serde_json::from_slice(&f.records(false)[1].body).unwrap();
    assert!(sent.get("context_management").is_none());
    assert_eq!(sent["input"][0], inline_output(1)[1]);
    assert!(sent["input"].as_array().unwrap().ends_with(&active));
    r["input"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"done"}),
        json!({"role":"user","content":"Next"}),
    ]);
    f.request("inline")
        .header("x-ostk-roll", "1")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let sent: Value = serde_json::from_slice(&f.records(false)[2].body).unwrap();
    assert_eq!(sent["input"][0], inline_output(1)[1]);
    assert_eq!(sent["context_management"][0]["compact_threshold"], 8192);
    assert_eq!(f.snapshots().await[0]["epoch"], 2);
    let before_restart = f.snapshots().await;
    r["input"].as_array_mut().unwrap().extend(inline_output(2));
    let next = json!({"role":"user","content":"After restart"});
    r["input"].as_array_mut().unwrap().push(next.clone());
    f.restart().await;
    f.request("inline")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let sent: Value = serde_json::from_slice(&f.records(false)[3].body).unwrap();
    let mut expected = inline_output(2)[1..].to_vec();
    expected.push(next);
    assert_eq!(sent["input"], json!(expected));
    assert_eq!(
        f.snapshots().await[0]["projection"],
        before_restart[0]["projection"]
    );
    assert_eq!(f.snapshots().await[0]["epoch"], 2);
    for record in f.records(false) {
        let sent: Value = serde_json::from_slice(&record.body).unwrap();
        assert_eq!(sent["prompt_cache_options"], r["prompt_cache_options"]);
        assert_eq!(sent["prompt_cache_key"], r["prompt_cache_key"]);
    }
    assert!(f.records(true).is_empty());
}

#[tokio::test]
async fn inline_truncation_or_missing_output_indices_cannot_advance_checkpoint() {
    let f = Fixture::configured(Mode::Rolling, |cfg| {
        cfg.compaction_backend = CompactionBackend::Inline
    })
    .await;
    f.mock.inline_mode.store(1, Ordering::SeqCst);
    let mut r = input();
    f.request("inline-fail")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let saved = f.snapshots().await;
    r["input"].as_array_mut().unwrap().extend(inline_output(1));
    r["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"user","content":"next"}));
    for mode in [2, 4] {
        f.mock.inline_mode.store(mode, Ordering::SeqCst);
        let returned = f
            .request("inline-fail")
            .header("x-ostk-roll", "1")
            .json(&r)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert!(!returned.is_empty());
        assert_eq!(f.snapshots().await, saved);
    }
    f.mock.inline_mode.store(1, Ordering::SeqCst);
    f.request("inline-fail")
        .header("x-ostk-roll", "1")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(f.snapshots().await[0]["epoch"], 2);
}

#[tokio::test]
async fn disconnect_after_inline_checkpoint_releases_lane_without_commit() {
    let f = Fixture::configured(Mode::Rolling, |cfg| {
        cfg.compaction_backend = CompactionBackend::Inline
    })
    .await;
    f.mock.inline_mode.store(3, Ordering::SeqCst);
    let r = input();
    let mut response = f.request("inline-drop").json(&r).send().await.unwrap();
    let mut seen = Vec::new();
    while !String::from_utf8_lossy(&seen).contains("opaque-1") {
        seen.extend(response.chunk().await.unwrap().unwrap());
    }
    drop(response);
    assert!(f.snapshots().await.is_empty());
    f.mock.inline_mode.store(1, Ordering::SeqCst);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        f.request("inline-drop")
            .json(&r)
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    })
    .await
    .unwrap();
    assert_eq!(f.snapshots().await[0]["epoch"], 1);
}

#[tokio::test]
async fn edited_replayed_inline_checkpoint_resets_to_authoritative_client_history() {
    let f = Fixture::configured(Mode::Rolling, |cfg| {
        cfg.compaction_backend = CompactionBackend::Inline
    })
    .await;
    f.mock.inline_mode.store(1, Ordering::SeqCst);
    let mut r = input();
    f.request("inline-edit")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let mut output = inline_output(1);
    output[1]["encrypted_content"] = json!("changed-by-client");
    r["input"].as_array_mut().unwrap().extend(output);
    r["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"user","content":"next"}));
    f.request("inline-edit")
        .header("x-ostk-roll", "1")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let sent: Value = serde_json::from_slice(&f.records(false)[1].body).unwrap();
    assert_eq!(sent["input"], r["input"]);
    assert!(sent.get("context_management").is_none());
    assert_eq!(f.snapshots().await[0]["cut"], 0);
}

#[tokio::test]
async fn inline_tool_boundary_requires_all_results_and_preserves_native_items() {
    let f = Fixture::configured(Mode::Rolling, |cfg| {
        cfg.compaction_backend = CompactionBackend::Inline;
        cfg.inline_tool_boundaries = true;
    })
    .await;
    f.mock.inline_mode.store(1, Ordering::SeqCst);
    let mut r = json!({"model":"gpt-6-astra","input":[
        {"role":"user","content":"inspect"},
        {"type":"reasoning","encrypted_content":"unchanged+/="},
        {"type":"function_call","call_id":"a","name":"read","arguments":"{}"},
        {"type":"function_call","call_id":"b","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"a","output":"first"}]});
    f.request("tool-boundary")
        .header("x-ostk-roll", "1")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let sent: Value = serde_json::from_slice(&f.records(false)[0].body).unwrap();
    assert!(sent.get("context_management").is_none());
    r["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"function_call_output","call_id":"b","output":"X".repeat(5000)}));
    f.request("tool-boundary")
        .header("x-ostk-roll", "1")
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let sent: Value = serde_json::from_slice(&f.records(false)[1].body).unwrap();
    assert!(sent.get("context_management").is_some());
    assert_eq!(sent["input"], r["input"]);
    assert_eq!(f.snapshots().await[0]["epoch"], 1);
}

#[tokio::test]
async fn economic_gate_defers_unprofitable_roll_and_accepts_valid_positive_estimate() {
    let f = Fixture::configured(Mode::Rolling, |cfg| {
        cfg.compaction_backend = CompactionBackend::Inline;
        cfg.economic_roll_policy = true;
    })
    .await;
    f.mock.inline_mode.store(1, Ordering::SeqCst);
    let r = input();
    let mut estimate = json!({"remaining_calls":1,"saved_input_per_call":1.0,"checkpoint_cost":10.0,"lost_cache_cost":1.0,"recovery_cost":1.0});
    f.request("economics")
        .header("x-ostk-roll", "1")
        .header("x-ostk-roll-estimate", estimate.to_string())
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert!(
        serde_json::from_slice::<Value>(&f.records(false)[0].body)
            .unwrap()
            .get("context_management")
            .is_none()
    );
    estimate["remaining_calls"] = json!(100);
    f.request("economics")
        .header("x-ostk-roll", "1")
        .header("x-ostk-roll-estimate", estimate.to_string())
        .json(&r)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert!(
        serde_json::from_slice::<Value>(&f.records(false)[1].body)
            .unwrap()
            .get("context_management")
            .is_some()
    );
    assert!(
        !f.records(false)[1]
            .headers
            .contains_key("x-ostk-roll-estimate")
    );
    let response = f
        .request("economics")
        .header("x-ostk-roll-estimate", "invalid")
        .json(&r)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
