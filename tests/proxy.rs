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
    config::{Config, Mode},
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
    mode: AtomicUsize,
    compact_fail: AtomicUsize,
    delay: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
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
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.proxy.abort();
        self.upstream.abort();
    }
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mock = Arc::new(Mock::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let upstream_router = Router::new()
            .route("/v1/responses", post(mock_response))
            .route("/v1/responses/compact", post(mock_compact))
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
        let app = App::new(cfg).await.unwrap();
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
        }
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
}
