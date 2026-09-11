//! Opt-in feasibility transport. One Codex websocket, one upstream websocket,
//! one inventory state; no process-global response/inventory cache.
use crate::{
    config::NativeToolBindingMode,
    native_binding::Connection,
    now_ms,
    proxy::{App, effective_headers, error, strip_header},
};
use axum::{
    extract::{
        OriginalUri, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    io::BufReader,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio_tungstenite::{
    Connector,
    tungstenite::{self, client::IntoClientRequest, protocol::WebSocketConfig},
};

// Codex 0.154 consumes these from the successful websocket handshake. The turn
// state is routing metadata, not an inventory cache; Codex owns its turn lifetime.
const HANDSHAKE_METADATA: &[&str] = &["x-codex-turn-state", "x-reasoning-included", "openai-model"];
const REJECTION_METADATA: &[&str] = &[
    "retry-after",
    "www-authenticate",
    "x-openai-authorization-error",
    "x-codex-active-limit",
    "x-codex-rate-limit-reached-type",
];

pub(crate) async fn handle(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    app.requests_received.fetch_add(1, Ordering::Relaxed);
    let headers = match effective_headers(headers) {
        Ok(h) => h,
        Err((s, m)) => return error(s, m),
    };
    if headers.contains_key("sec-websocket-protocol") {
        return error(
            StatusCode::BAD_REQUEST,
            "NATIVE_BINDING_UNSUPPORTED_WEBSOCKET_SUBPROTOCOL",
        );
    }
    let mut url = match reqwest::Url::parse(&format!(
        "{}/responses",
        app.config.upstream.trim_end_matches('/')
    )) {
        Ok(u) => u,
        Err(_) => return error(StatusCode::BAD_GATEWAY, "NATIVE_BINDING_INVALID_UPSTREAM"),
    };
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    let _ = url.set_scheme(scheme);
    url.set_query(uri.query());
    let mut request = match url.as_str().into_client_request() {
        Ok(r) => r,
        Err(_) => return error(StatusCode::BAD_GATEWAY, "NATIVE_BINDING_INVALID_UPSTREAM"),
    };
    for (name, value) in &headers {
        if !strip_header(name.as_str(), &headers) && !name.as_str().starts_with("sec-websocket-") {
            request.headers_mut().insert(name.clone(), value.clone());
        }
    }
    request
        .headers_mut()
        .insert("x-ostk-gpt-hop", "1".parse().unwrap());
    let connector = match tls_connector(&app).await {
        Ok(c) => c,
        Err(_) => return error(StatusCode::BAD_GATEWAY, "NATIVE_BINDING_TLS_CONFIG_ERROR"),
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(app.config.max_body_bytes))
        .max_frame_size(Some(app.config.max_body_bytes));
    let upstream = tokio::time::timeout(
        Duration::from_secs(20),
        tokio_tungstenite::connect_async_tls_with_config(
            request,
            Some(config),
            false,
            Some(connector),
        ),
    )
    .await;
    let (upstream, handshake) = match upstream {
        Ok(Ok(pair)) => pair,
        Ok(Err(tungstenite::Error::Http(response))) => {
            let status = response.status();
            app.store.record(&json!({"kind":"native_binding_transport","transport":"websocket","outcome":"upstream_handshake_rejected","status":status.as_u16(),"ts_ms":now_ms()})).await;
            let mut rejection = error(status, "NATIVE_BINDING_UPSTREAM_HANDSHAKE_REJECTED");
            // Preserve error classification/retry advice without exposing the
            // upstream body, arbitrary headers, cookies, or authentication values.
            copy_metadata(response.headers(), &mut rejection, REJECTION_METADATA);
            return rejection;
        }
        _ => {
            app.store.record(&json!({"kind":"native_binding_transport","transport":"websocket","outcome":"upstream_handshake_failed","ts_ms":now_ms()})).await;
            return error(
                StatusCode::BAD_GATEWAY,
                "NATIVE_BINDING_UPSTREAM_HANDSHAKE_FAILED",
            );
        }
    };
    let limit = app.config.max_body_bytes;
    let mut response = ws
        .max_message_size(limit)
        .max_frame_size(limit)
        .on_upgrade(move |socket| bridge(app, socket, upstream))
        .into_response();
    copy_metadata(handshake.headers(), &mut response, HANDSHAKE_METADATA);
    response
}

fn copy_metadata(source: &HeaderMap, response: &mut Response, names: &[&'static str]) {
    for &name in names {
        // An upstream Connection token can make even an allowlisted name hop-by-hop.
        if !strip_header(name, source) {
            for value in source.get_all(name) {
                response.headers_mut().append(name, value.clone());
            }
        }
    }
}

async fn tls_connector(app: &App) -> anyhow::Result<Connector> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = &app.config.upstream_ca_bundle {
        let pem = tokio::fs::read(path).await?;
        for cert in rustls_pemfile::certs(&mut BufReader::new(pem.as_slice())) {
            roots.add(cert?)?;
        }
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Connector::Rustls(Arc::new(config)))
}

async fn bridge(
    app: App,
    mut downstream: WebSocket,
    mut upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    let mut state = Connection::default();
    let connection_id =
        crate::hash(format!("{}-{:?}", now_ms(), std::time::Instant::now()).as_bytes());
    let timeout = Duration::from_secs(app.config.request_timeout_seconds);
    loop {
        tokio::select! {
            message = tokio::time::timeout(timeout, downstream.next()) => {
                let Ok(Some(Ok(message))) = message else { break; };
                match message {
                    Message::Text(text) => {
                        let prepared = serde_json::from_str::<Value>(&text).map_err(|_| anyhow::anyhow!("NATIVE_BINDING_INVALID_JSON")).and_then(|body| state.prepare(&body, app.config.native_tool_binding));
                        let outgoing = match prepared {
                            Ok(p) => {
                                let out = if p.changed { p.body.to_string() } else { text.to_string() };
                                app.store.record(&json!({"kind":"native_binding_request","transport":"websocket","connection":connection_id,"bytes_in":text.len(),"bytes_out":out.len(),"ts_ms":now_ms(),"evidence":p.evidence})).await;
                                out
                            }
                            Err(e) => {
                                app.store.record(&json!({"kind":"native_binding_rejected","transport":"websocket","connection":connection_id,"code":e.to_string(),"mode":app.config.native_tool_binding,"ts_ms":now_ms()})).await;
                                if app.config.native_tool_binding == NativeToolBindingMode::Observe { text.to_string() }
                                else {
                                    let _ = downstream.send(Message::Text(json!({"type":"error","error":{"type":"invalid_request_error","code":"native_binding_unsupported","message":e.to_string()}}).to_string().into())).await;
                                    break;
                                }
                            }
                        };
                        if upstream.send(tungstenite::Message::Text(outgoing.into())).await.is_err() { break; }
                    }
                    Message::Ping(p) => { if upstream.send(tungstenite::Message::Ping(p)).await.is_err() { break; } }
                    Message::Pong(p) => { if upstream.send(tungstenite::Message::Pong(p)).await.is_err() { break; } }
                    Message::Close(_) => break,
                    Message::Binary(bytes) => {
                        if app.config.native_tool_binding == NativeToolBindingMode::Observe {
                            app.store.record(&json!({"kind":"native_binding_rejected","transport":"websocket","connection":connection_id,"code":"NATIVE_BINDING_BINARY_REQUEST_UNSUPPORTED","mode":app.config.native_tool_binding,"ts_ms":now_ms()})).await;
                            if upstream.send(tungstenite::Message::Binary(bytes)).await.is_err() { break; }
                            continue;
                        }
                        let _ = downstream.send(Message::Text(json!({"type":"error","error":{"code":"native_binding_unsupported","message":"NATIVE_BINDING_BINARY_REQUEST_UNSUPPORTED"}}).to_string().into())).await;
                        break;
                    }
                }
            }
            message = tokio::time::timeout(timeout, upstream.next()) => {
                let Ok(Some(Ok(message))) = message else { break; };
                let outgoing = match message {
                    tungstenite::Message::Text(text) => {
                        if let Ok(event) = serde_json::from_str::<Value>(&text) {
                            if let Err(error) = state.observe_response(&event) {
                                app.store.record(&json!({"kind":"native_binding_rejected_response","transport":"websocket","connection":connection_id,"code":error.to_string(),"mode":app.config.native_tool_binding,"ts_ms":now_ms()})).await;
                                if app.config.native_tool_binding != NativeToolBindingMode::Observe {
                                    let _ = downstream.send(Message::Text(json!({"type":"error","error":{"code":"native_binding_unsupported","message":error.to_string()}}).to_string().into())).await;
                                    break;
                                }
                            }
                            let native_items = if event["type"] == "response.output_item.done" {
                                vec![&event["item"]]
                            } else {
                                event["response"]["output"].as_array().map(|items| items.iter().collect()).unwrap_or_default()
                            };
                            let checkpoints: Vec<Value> = native_items.into_iter().filter(|item| item["type"] == "compaction").map(|item| json!({
                                "item_sha256":crate::fingerprint(item),
                                "encrypted_sha256":item["encrypted_content"].as_str().map(|s| crate::hash(s.as_bytes()))
                            })).collect();
                            if !checkpoints.is_empty() {
                                // This exact text is forwarded below. No native payload is logged.
                                app.store.record(&json!({"kind":"native_binding_native_output","transport":"websocket","connection":connection_id,"event":event["type"],"frame_sha256":crate::hash(text.as_bytes()),"checkpoints":checkpoints,"forwarded_unchanged":true,"ts_ms":now_ms()})).await;
                            }
                            if matches!(event["type"].as_str(), Some("response.completed" | "response.failed" | "response.incomplete" | "error")) {
                                app.store.record(&json!({"kind":"native_binding_response","transport":"websocket","connection":connection_id,"event":event["type"],"usage":usage_summary(&event["response"]["usage"]),"ts_ms":now_ms()})).await;
                            }
                        } else if app.config.native_tool_binding != NativeToolBindingMode::Observe {
                            let _ = downstream.send(Message::Text(json!({"type":"error","error":{"code":"native_binding_unsupported","message":"NATIVE_BINDING_NON_JSON_RESPONSE_UNSUPPORTED"}}).to_string().into())).await;
                            break;
                        }
                        Message::Text(text.to_string().into())
                    }
                    tungstenite::Message::Binary(b) => {
                        if app.config.native_tool_binding == NativeToolBindingMode::Observe { Message::Binary(b) }
                        else {
                            let _ = downstream.send(Message::Text(json!({"type":"error","error":{"code":"native_binding_unsupported","message":"NATIVE_BINDING_BINARY_RESPONSE_UNSUPPORTED"}}).to_string().into())).await;
                            break;
                        }
                    },
                    tungstenite::Message::Ping(b) => Message::Ping(b),
                    tungstenite::Message::Pong(b) => Message::Pong(b),
                    tungstenite::Message::Close(_) => break,
                    tungstenite::Message::Frame(_) => continue,
                };
                if downstream.send(outgoing).await.is_err() { break; }
            }
        }
    }
    let _ = upstream.close(None).await;
    let _ = downstream.close().await;
    app.store.record(&json!({"kind":"native_binding_transport","transport":"websocket","connection":connection_id,"outcome":"closed","ts_ms":now_ms()})).await;
}

fn usage_summary(usage: &Value) -> Value {
    // Provider usage can also contain per-item attribution and opaque item IDs.
    // Keep only aggregate numeric accounting, separately from request bytes.
    let mut summary = serde_json::Map::new();
    for key in ["input_tokens", "output_tokens", "total_tokens"] {
        if let Some(n) = usage[key].as_u64() {
            summary.insert(key.into(), json!(n));
        }
    }
    for (group, keys) in [
        (
            "input_tokens_details",
            &["cached_tokens", "cache_write_tokens"][..],
        ),
        ("output_tokens_details", &["reasoning_tokens"][..]),
    ] {
        for key in keys {
            if let Some(n) = usage[group][key].as_u64() {
                summary.insert(format!("{group}.{key}"), json!(n));
            }
        }
    }
    Value::Object(summary)
}
