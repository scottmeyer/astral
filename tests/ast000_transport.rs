use axum::{
    Router,
    body::Bytes,
    extract::{State, WebSocketUpgrade, ws::Message},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use ostk_gpt_cache::{
    config::{Ast000, Config, Mode},
    proxy::{App, router},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{Message as Wire, client::IntoClientRequest};

type Records = Arc<Mutex<Vec<(HeaderMap, String)>>>;
const COMPLETION: &str = r#"{ "type":"response.completed", "response":{"id":"synthetic-response", "output":[{"type":"custom_tool_call","call_id":"unaltered","input":"pwd"}],"usage":{"input_tokens":3,"attribution":{"private":"MUST_NOT_LOG"}}} }"#;
const NATIVE_OUTPUT: &str = r#"{ "type":"response.output_item.done", "output_index":0, "item":{"type":"compaction","id":"generated-native","encrypted_content":"synthetic+/=opaque","unknown_native":{"keep":[true,null,3]}} }"#;
const NATIVE_COMPLETION: &str =
    r#"{ "type":"response.completed", "response":{"id":"compacted-response","output":[]} }"#;

async fn ws_mock(
    State(records): State<Records>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Some(status) = headers
        .get("x-mock-handshake-status")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u16>().ok())
    {
        let mut response = (
            StatusCode::from_u16(status).unwrap(),
            "PRIVATE_UPSTREAM_BODY",
        )
            .into_response();
        for (name, value) in [
            ("retry-after", "7"),
            ("www-authenticate", "Bearer error=\"invalid_token\""),
            ("x-openai-authorization-error", "synthetic_rejected"),
            ("x-codex-active-limit", "primary"),
            ("x-codex-rate-limit-reached-type", "primary"),
            ("set-cookie", "private_cookie=do_not_forward"),
            ("authorization", "Bearer do_not_forward"),
            ("x-private-header", "do_not_forward"),
        ] {
            response.headers_mut().insert(name, value.parse().unwrap());
        }
        if headers.contains_key("x-mock-hop-retry") {
            response
                .headers_mut()
                .insert("connection", "retry-after".parse().unwrap());
        }
        return response;
    }
    let mut response = ws
        .on_upgrade(move |mut socket| async move {
            while let Some(Ok(message)) = socket.next().await {
                let text = match message {
                    Message::Text(text) => text,
                    Message::Binary(bytes) => {
                        if socket.send(Message::Binary(bytes)).await.is_err() {
                            break;
                        }
                        continue;
                    }
                    _ => break,
                };
                records
                    .lock()
                    .unwrap()
                    .push((headers.clone(), text.to_string()));
                let body: Value = serde_json::from_str(&text).unwrap();
                if body["input"].as_array().is_some_and(|input| {
                    input
                        .last()
                        .is_some_and(|item| item["type"] == "compaction_trigger")
                }) {
                    for event in [NATIVE_OUTPUT, NATIVE_COMPLETION] {
                        if socket.send(Message::Text(event.into())).await.is_err() {
                            return;
                        }
                    }
                    continue;
                }
                if socket.send(Message::Text(COMPLETION.into())).await.is_err() {
                    break;
                }
            }
        })
        .into_response();
    for (name, value) in [
        ("x-codex-turn-state", "synthetic-turn-state"),
        ("x-reasoning-included", "true"),
        ("openai-model", "synthetic-upstream-model"),
        ("set-cookie", "private_cookie=do_not_forward"),
        ("authorization", "Bearer do_not_forward"),
        ("x-private-header", "do_not_forward"),
    ] {
        response.headers_mut().insert(name, value.parse().unwrap());
    }
    response
}
async fn http_mock(
    State(records): State<Records>,
    headers: HeaderMap,
    body: Bytes,
) -> &'static str {
    records
        .lock()
        .unwrap()
        .push((headers, String::from_utf8(body.to_vec()).unwrap()));
    COMPLETION
}
struct Fixture {
    root: tempfile::TempDir,
    url: String,
    records: Records,
    tasks: Vec<JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Fixture {
    async fn new(mode: Ast000) -> Self {
        let root = tempfile::tempdir().unwrap();
        let records = Records::default();
        let up = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = format!("http://{}/backend-api/codex", up.local_addr().unwrap());
        let routes = Router::new()
            .route("/backend-api/codex/responses", get(ws_mock).post(http_mock))
            .with_state(records.clone());
        let up_task = tokio::spawn(async move {
            axum::serve(up, routes).await.unwrap();
        });
        let mut config = Config::parse_from(["test"]);
        config.upstream = upstream;
        config.state_dir = root.path().join("state");
        config.mode = Mode::Passthrough;
        config.ast000_compat = mode;
        let app = App::new(config).await.unwrap();
        let down = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/backend-api/codex/responses",
            down.local_addr().unwrap()
        );
        let proxy_task = tokio::spawn(async move {
            axum::serve(down, router(app)).await.unwrap();
        });
        Self {
            root,
            url,
            records,
            tasks: vec![up_task, proxy_task],
        }
    }
    async fn socket(
        &self,
        identity: &str,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let mut request = self
            .url
            .replacen("http://", "ws://", 1)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer synthetic-{identity}").parse().unwrap(),
        );
        request
            .headers_mut()
            .insert("chatgpt-account-id", identity.parse().unwrap());
        tokio_tungstenite::connect_async(request).await.unwrap().0
    }
}
fn full(name: &str) -> Value {
    json!({"type":"response.create","model":"synthetic","input":[
        {"type":"additional_tools","id":"at_runtime","role":"developer","tools":[{"name":name,"type":"function","parameters":{}}]},
        {"type":"message","role":"developer","content":"synthetic runtime"}
    ]})
}
fn checkpoint() -> Value {
    json!({"type":"compaction","id":"native","encrypted_content":"synthetic+/=","unknown_native":{"nested":true}})
}

#[tokio::test]
async fn websocket_handshake_metadata_reaches_client_and_is_reused_on_reconnect() {
    let fixture = Fixture::new(Ast000::Rebind).await;
    let request = fixture.url.replacen("http://", "ws://", 1);
    let (mut socket, handshake) = tokio_tungstenite::connect_async(&request).await.unwrap();
    assert_eq!(
        handshake.headers()["x-codex-turn-state"],
        "synthetic-turn-state"
    );
    assert_eq!(handshake.headers()["x-reasoning-included"], "true");
    assert_eq!(
        handshake.headers()["openai-model"],
        "synthetic-upstream-model"
    );
    for name in ["set-cookie", "authorization", "x-private-header"] {
        assert!(!handshake.headers().contains_key(name));
    }
    socket.close(None).await.unwrap();

    // Mimic Codex retaining the handshake's sticky token for reconnect within a
    // turn. A new connection still needs its own complete runtime prefix.
    let mut reconnect = request.into_client_request().unwrap();
    reconnect.headers_mut().insert(
        "x-codex-turn-state",
        handshake.headers()["x-codex-turn-state"].clone(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(reconnect).await.unwrap();
    socket
        .send(Wire::Text(full("current").to_string().into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap().into_text().unwrap(),
        COMPLETION
    );
    assert_eq!(
        fixture.records.lock().unwrap()[0].0["x-codex-turn-state"],
        "synthetic-turn-state"
    );
}

#[tokio::test]
async fn websocket_rejection_does_not_forward_allowlisted_hop_headers() {
    let fixture = Fixture::new(Ast000::Rebind).await;
    let mut request = fixture
        .url
        .replacen("http://", "ws://", 1)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("x-mock-hop-retry", "true".parse().unwrap());
    request
        .headers_mut()
        .insert("x-mock-handshake-status", "429".parse().unwrap());
    let error = tokio_tungstenite::connect_async(request).await.unwrap_err();
    let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
        panic!("expected HTTP rejection, got {error}");
    };
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(!response.headers().contains_key("retry-after"));
    assert!(response.headers().contains_key("www-authenticate"));
}

#[tokio::test]
async fn websocket_handshake_rejections_preserve_classification_without_private_data() {
    let fixture = Fixture::new(Ast000::Rebind).await;
    for status in [401, 429, 426] {
        let mut request = fixture
            .url
            .replacen("http://", "ws://", 1)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "x-mock-handshake-status",
            status.to_string().parse().unwrap(),
        );
        let error = tokio_tungstenite::connect_async(request).await.unwrap_err();
        let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
            panic!("expected HTTP rejection, got {error}");
        };
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers()["retry-after"], "7");
        assert_eq!(
            response.headers()["www-authenticate"],
            "Bearer error=\"invalid_token\""
        );
        assert_eq!(
            response.headers()["x-openai-authorization-error"],
            "synthetic_rejected"
        );
        assert_eq!(response.headers()["x-codex-active-limit"], "primary");
        assert_eq!(
            response.headers()["x-codex-rate-limit-reached-type"],
            "primary"
        );
        for name in ["set-cookie", "authorization", "x-private-header"] {
            assert!(!response.headers().contains_key(name));
        }
        let body = String::from_utf8(response.body().clone().unwrap_or_default()).unwrap();
        assert!(body.contains("AST000_UPSTREAM_HANDSHAKE_REJECTED"));
        assert!(!body.contains("PRIVATE_UPSTREAM_BODY"));
    }
    let ledger = std::fs::read_to_string(fixture.root.path().join("state/ledger.jsonl")).unwrap();
    assert!(!ledger.contains("do_not_forward"));
    assert!(!ledger.contains("PRIVATE_UPSTREAM_BODY"));
    assert!(!ledger.contains("invalid_token"));
    assert!(fixture.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn websocket_observe_forwards_binary_requests_but_correction_rejects_them() {
    for mode in [Ast000::Observe, Ast000::Rebind] {
        let fixture = Fixture::new(mode).await;
        let mut socket = fixture.socket("binary-fixture").await;
        let binary = Wire::Binary(vec![0, 255, 1, 128].into());
        socket.send(binary.clone()).await.unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if mode == Ast000::Observe {
            assert_eq!(reply, binary);
        } else {
            assert!(
                reply
                    .into_text()
                    .unwrap()
                    .contains("AST000_BINARY_REQUEST_UNSUPPORTED")
            );
        }
    }
}

#[tokio::test]
async fn websocket_warmup_delta_output_and_identity_are_preserved() {
    let fixture = Fixture::new(Ast000::Rebind).await;
    for (identity, name) in [("account-alpha", "alpha"), ("account-beta", "beta")] {
        let mut socket = fixture.socket(identity).await;
        let warmup = serde_json::to_string_pretty(&full(name)).unwrap();
        socket
            .send(Wire::Text(warmup.clone().into()))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap().into_text().unwrap(),
            COMPLETION
        );
        let delta = json!({"type":"response.create","model":"synthetic","previous_response_id":"synthetic-response","input":[checkpoint(),{"type":"custom_tool_call_output","call_id":"preserved-id","output":"unchanged"}]});
        socket
            .send(Wire::Text(delta.to_string().into()))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap().into_text().unwrap(),
            COMPLETION
        );
        {
            let records = fixture.records.lock().unwrap();
            let last = records.len() - 1;
            assert_eq!(records[last - 1].1, warmup); // No rewrite when unnecessary.
            assert_eq!(
                records[last].0["authorization"],
                format!("Bearer synthetic-{identity}")
            );
            assert_eq!(records[last].0["chatgpt-account-id"], identity);
            let mut transformed: Value = serde_json::from_str(&records[last].1).unwrap();
            assert_eq!(transformed["input"][1]["tools"][0]["name"], name);
            transformed["input"].as_array_mut().unwrap().remove(1);
            assert_eq!(transformed, delta);
        }
        socket.close(None).await.unwrap();
    }
    // An unrelated/new connection cannot use another account's previous ID.
    let mut reconnect = fixture.socket("account-alpha").await;
    reconnect.send(Wire::Text(json!({"type":"response.create","model":"synthetic","previous_response_id":"synthetic-response","input":[checkpoint()]}).to_string().into())).await.unwrap();
    let rejection = reconnect
        .next()
        .await
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert!(rejection.contains("AST000_UNKNOWN_PREVIOUS_RESPONSE"));
    assert_eq!(fixture.records.lock().unwrap().len(), 4);
    // The private ledger must still exclude provider per-item attribution.
    let ledger = std::fs::read_to_string(fixture.root.path().join("state/ledger.jsonl")).unwrap();
    assert!(!ledger.contains("MUST_NOT_LOG"));
    assert!(!ledger.contains("synthetic-account"));
}

#[tokio::test]
async fn http_full_history_corrects_but_compression_and_references_fail_explicitly() {
    let fixture = Fixture::new(Ast000::Rebind).await;
    let client = reqwest::Client::new();
    let mut body = full("current");
    body["input"].as_array_mut().unwrap().push(checkpoint());
    let response = client.post(&fixture.url).json(&body).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), COMPLETION);
    let captured: Value = serde_json::from_str(&fixture.records.lock().unwrap()[0].1).unwrap();
    assert_eq!(captured["input"][2], checkpoint());
    assert_eq!(captured["input"][3]["tools"][0]["name"], "current");
    for request in [
        client
            .post(&fixture.url)
            .header("content-encoding", "zstd")
            .body("synthetic compressed"),
        client.post(&fixture.url).json(
            &json!({"model":"synthetic","previous_response_id":"unknown","input":[checkpoint()]}),
        ),
    ] {
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 400);
        assert!(response.text().await.unwrap().contains("AST000_"));
    }
    assert_eq!(fixture.records.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn websocket_observation_and_before_checkpoint_are_real_transport_controls() {
    for mode in [Ast000::Observe, Ast000::RepeatBefore] {
        let fixture = Fixture::new(mode).await;
        let mut socket = fixture.socket("synthetic").await;
        let mut body = full("runtime");
        body["input"].as_array_mut().unwrap().push(checkpoint());
        let wire = serde_json::to_string_pretty(&body).unwrap();
        socket.send(Wire::Text(wire.clone().into())).await.unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap().into_text().unwrap(),
            COMPLETION
        );
        let records = fixture.records.lock().unwrap();
        if mode == Ast000::Observe {
            assert_eq!(records[0].1, wire);
        } else {
            let value: Value = serde_json::from_str(&records[0].1).unwrap();
            assert_eq!(value["input"][2]["type"], "additional_tools");
            assert_eq!(value["input"][3], checkpoint());
        }
    }
}

#[tokio::test]
async fn websocket_native_compaction_preserves_output_then_rebinds_delta() {
    let fixture = Fixture::new(Ast000::Rebind).await;
    let mut socket = fixture.socket("lifecycle-account").await;
    let mut request = full("current");
    request["client_metadata"] = json!({"x-codex-turn-metadata":"{\"thread_id\":\"lifecycle\",\"request_kind\":\"compaction\"}"});
    request["input"]
        .as_array_mut()
        .unwrap()
        .extend([checkpoint(), json!({"type":"compaction_trigger"})]);
    socket
        .send(Wire::Text(request.to_string().into()))
        .await
        .unwrap();
    for expected in [NATIVE_OUTPUT, NATIVE_COMPLETION] {
        assert_eq!(
            socket.next().await.unwrap().unwrap().into_text().unwrap(),
            expected
        );
    }
    let delta = json!({"type":"response.create","model":"synthetic","previous_response_id":"compacted-response","client_metadata":request["client_metadata"],"input":[{"type":"message","role":"user","content":"continue"}]});
    socket
        .send(Wire::Text(delta.to_string().into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap().into_text().unwrap(),
        COMPLETION
    );
    let records = fixture.records.lock().unwrap();
    let compact: Value = serde_json::from_str(&records[0].1).unwrap();
    assert_eq!(compact["input"][2], checkpoint());
    assert_eq!(compact["input"][3]["tools"][0]["name"], "current");
    assert_eq!(compact["input"][4]["type"], "compaction_trigger");
    let mut continued: Value = serde_json::from_str(&records[1].1).unwrap();
    assert_eq!(continued["input"][0]["tools"][0]["name"], "current");
    continued["input"].as_array_mut().unwrap().remove(0);
    assert_eq!(continued, delta);
}

#[tokio::test]
async fn http_compaction_requires_verified_control_metadata() {
    let fixture = Fixture::new(Ast000::Rebind).await;
    let client = reqwest::Client::new();
    let mut request = full("current");
    request["input"]
        .as_array_mut()
        .unwrap()
        .extend([checkpoint(), json!({"type":"compaction_trigger"})]);
    assert_eq!(
        client
            .post(&fixture.url)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    request["client_metadata"] = json!({"x-codex-turn-metadata":"{\"thread_id\":\"fixture\",\"request_kind\":\"compaction\"}"});
    assert_eq!(
        client
            .post(&fixture.url)
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(fixture.records.lock().unwrap().len(), 1);
}
