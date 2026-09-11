use crate::{
    config::{CompactionBackend, Config, Mode},
    engine::{self, Lane},
    hash, now_ms, policy,
    store::Store,
    usage::{Observer, Usage},
};
use anyhow::Context;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, OriginalUri, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::OwnedMutexGuard;

const HOP: &str = "x-ostk-gpt-hop";
type Pending = (
    String,
    OwnedMutexGuard<Option<Lane>>,
    Lane,
    Option<usize>,
    String,
);

#[derive(Clone)]
pub struct App {
    pub config: Arc<Config>,
    client: reqwest::Client,
    pub store: Arc<Store>,
    requests_received: Arc<AtomicU64>,
}

impl App {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        config.validate()?;
        let store = Arc::new(
            Store::open(
                config.state_dir.clone(),
                config.max_sessions,
                config.max_body_bytes,
            )
            .await?,
        );
        let mut client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(config.request_timeout_seconds));
        if let Some(path) = &config.upstream_ca_bundle {
            let pem = tokio::fs::read(path)
                .await
                .context("cannot read upstream CA bundle")?;
            let certificates = reqwest::Certificate::from_pem_bundle(&pem)
                .context("invalid upstream CA bundle")?;
            anyhow::ensure!(
                !certificates.is_empty(),
                "upstream CA bundle contains no certificates"
            );
            for certificate in certificates {
                client = client.add_root_certificate(certificate);
            }
        }
        let client = client.build()?;
        Ok(Self {
            config: Arc::new(config),
            client,
            store,
            requests_received: Arc::new(AtomicU64::new(0)),
        })
    }
}

pub fn router(app: App) -> Router {
    let limit = app.config.max_body_bytes;
    Router::new()
        .route(
            "/healthz",
            get(|State(app): State<App>| async move {
                axum::Json(json!({"status":"ok", "version":env!("CARGO_PKG_VERSION"),
                    "requests_received":app.requests_received.load(Ordering::Relaxed)}))
            }),
        )
        .route("/v1/responses", post(handle))
        .route("/responses", post(handle))
        .route("/backend-api/codex/responses", post(handle))
        .route("/v1/responses/compact", post(direct_compact))
        .route("/responses/compact", post(direct_compact))
        .route("/compact", post(direct_compact))
        .route("/backend-api/codex/responses/compact", post(direct_compact))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(app)
}

fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        axum::Json(json!({"error":{"type":"ostk_proxy_error", "message":message}})),
    )
        .into_response()
}

fn effective_headers(mut headers: HeaderMap) -> Result<HeaderMap, (StatusCode, &'static str)> {
    if headers.contains_key(HOP) {
        return Err((StatusCode::LOOP_DETECTED, "proxy loop detected"));
    }
    for name in [
        "authorization",
        "api-key",
        "x-api-key",
        "chatgpt-account-id",
        "openai-organization",
        "openai-project",
        "x-ostk-session-id",
        "session_id",
        "openai-session-id",
    ] {
        if headers.get_all(name).iter().count() > 1 {
            return Err((StatusCode::BAD_REQUEST, "duplicate identity header"));
        }
    }
    if !["authorization", "api-key", "x-api-key"]
        .iter()
        .any(|name| headers.contains_key(*name))
    {
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            let value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "invalid configured authorization",
                )
            })?;
            headers.insert("authorization", value);
        }
    }
    Ok(headers)
}

fn strip_header(name: &str, headers: &HeaderMap) -> bool {
    matches!(
        name,
        "host"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
            | "accept-encoding"
    ) || name.starts_with("x-ostk-")
        || headers
            .get_all("connection")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|v| v.trim().eq_ignore_ascii_case(name))
}

fn request(
    app: &App,
    headers: &HeaderMap,
    suffix: &str,
    body: Vec<u8>,
    compact: bool,
) -> reqwest::RequestBuilder {
    let url = format!("{}{suffix}", app.config.upstream.trim_end_matches('/'));
    let mut r = app.client.post(url);
    for (k, v) in headers {
        if !strip_header(k.as_str(), headers)
            && !(compact
                && matches!(
                    k.as_str(),
                    "idempotency-key"
                        | "content-type"
                        | "content-encoding"
                        | "accept"
                        | "x-client-request-id"
                ))
        {
            r = r.header(k, v);
        }
    }
    r = r.header(HOP, "1").header("accept-encoding", "identity");
    if compact {
        r = r
            .header("content-type", "application/json")
            .header("accept", "application/json");
    }
    r.body(body)
}

fn lane_id(app: &App, headers: &HeaderMap, value: &Value) -> Option<String> {
    // Request IDs and cache routing keys are NOT conversation identities.
    let session = ["x-ostk-session-id", "session_id", "openai-session-id"]
        .iter()
        .find_map(|h| {
            headers
                .get(*h)
                .and_then(|v| v.to_str().ok())
                .filter(|s| !s.is_empty())
        })?;
    let parts = json!([
        app.config.upstream,
        app.config.compact_path,
        app.config.compaction_backend,
        app.config.inline_threshold_tokens,
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        headers
            .get("api-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        headers
            .get("chatgpt-account-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        headers
            .get("openai-organization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        headers
            .get("openai-project")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        session,
        value["model"],
    ]);
    Some(hash(&serde_json::to_vec(&parts).unwrap()))
}

async fn direct_compact(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    app.requests_received.fetch_add(1, Ordering::Relaxed);
    let headers = match effective_headers(headers) {
        Ok(h) => h,
        Err((status, message)) => return error(status, message),
    };
    let suffix = format!(
        "{}{}",
        app.config.compact_path,
        uri.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    forward(
        app,
        headers,
        body.to_vec(),
        body.len(),
        &suffix,
        None,
        "caller_compact",
        false,
    )
    .await
}

async fn handle(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    app.requests_received.fetch_add(1, Ordering::Relaxed);
    let headers = match effective_headers(headers) {
        Ok(h) => h,
        Err((status, message)) => return error(status, message),
    };
    let suffix = format!(
        "/responses{}",
        uri.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    let request_stream = serde_json::from_slice::<Value>(&body)
        .ok()
        .is_some_and(|v| v["stream"] == true);
    let compressed = headers
        .get("content-encoding")
        .is_some_and(|v| v != "identity");
    if app.config.mode == Mode::Passthrough || compressed {
        return forward(
            app,
            headers,
            body.to_vec(),
            body.len(),
            &suffix,
            None,
            "passthrough",
            request_stream,
        )
        .await;
    }
    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid JSON"),
    };
    if let Some(reason) = engine::exclusion(&value) {
        return forward(
            app,
            headers,
            body.to_vec(),
            body.len(),
            &suffix,
            None,
            reason,
            request_stream,
        )
        .await;
    }
    let Some(id) = lane_id(&app, &headers, &value) else {
        return forward(
            app,
            headers,
            body.to_vec(),
            body.len(),
            &suffix,
            None,
            "missing_session",
            request_stream,
        )
        .await;
    };
    if let Err(e) = policy::apply(
        &mut value,
        app.config.platform(),
        app.config.retention,
        Some(&id),
    ) {
        return error(StatusCode::BAD_REQUEST, &e.to_string());
    }
    let guard = match app.store.lane(&id).await {
        Ok(g) => g,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
    };
    let mut plan = engine::plan(
        &value,
        guard.as_ref().unwrap(),
        now_ms(),
        &app.config,
        headers.get("x-ostk-roll").is_some_and(|v| v == "1"),
    );
    let mut reason = plan.reason;
    let mut inline_bytes = None;
    if plan.compact_input.is_some() && app.config.compaction_backend == CompactionBackend::Inline {
        if app.config.platform() || app.config.allow_compatible_compaction {
            inline_bytes = Some(engine::bytes(&plan.effective));
            value["context_management"] = json!([{"type":"compaction", "compact_threshold":app.config.inline_threshold_tokens}]);
            reason = "scheduled_inline";
        } else {
            reason = "compatible_compaction_disabled";
        }
    } else if let Some(prefix) = plan.compact_input.clone() {
        if app.config.platform() || app.config.allow_compatible_compaction {
            let start = Instant::now();
            let mut compact_body = json!({"model":value["model"], "input":prefix});
            if let Some(instructions) = value.get("instructions") {
                compact_body["instructions"] = instructions.clone();
            }
            let result = compact(&app, &headers, &compact_body).await;
            let mut compact_usage = None;
            let mut status = None;
            let mut output_bytes = None;
            let mut rejection_reason = None;
            let outcome = match result {
                Ok((code, reply)) => {
                    status = Some(code);
                    compact_usage = reply.get("usage").and_then(Usage::parse);
                    if (200..300).contains(&code) {
                        if let Some(output) = reply.get("output").and_then(Value::as_array) {
                            output_bytes = Some(engine::bytes(output));
                            match plan.accept(
                                output,
                                value["input"].as_array().unwrap(),
                                app.config.min_savings,
                            ) {
                                Ok(()) => "accepted",
                                Err(error) => {
                                    rejection_reason = Some(error.to_string());
                                    reason = "compact_rejected";
                                    "rejected_output"
                                }
                            }
                        } else {
                            reason = "compact_failed";
                            "missing_output"
                        }
                    } else {
                        reason = "compact_failed";
                        "upstream_error"
                    }
                }
                Err(_) => {
                    reason = "compact_failed";
                    "transport_or_decode_error"
                }
            };
            app.store.record(&json!({"kind":"compact", "ts_ms":now_ms(), "lane":id, "outcome":outcome, "status":status,
                "input_bytes":engine::bytes(&prefix), "output_bytes":output_bytes, "usage":compact_usage,
                "rejection_reason":rejection_reason,
                "elapsed_ms":start.elapsed().as_millis()})).await;
        } else {
            reason = "compatible_compaction_disabled";
        }
    }
    value["input"] = json!(plan.effective);
    let outgoing = serde_json::to_vec(&value).unwrap();
    if outgoing.len() > app.config.max_body_bytes {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "projected request exceeds max-body-bytes; no history was discarded",
        );
    }
    let pending = Some((
        id,
        guard,
        plan.next,
        inline_bytes,
        crate::fingerprint(&value["input"]),
    ));
    forward(
        app,
        headers,
        outgoing,
        body.len(),
        &suffix,
        pending,
        reason,
        request_stream,
    )
    .await
}

async fn compact(app: &App, headers: &HeaderMap, body: &Value) -> anyhow::Result<(u16, Value)> {
    let r = request(
        app,
        headers,
        &app.config.compact_path,
        serde_json::to_vec(body)?,
        true,
    )
    .timeout(Duration::from_secs(app.config.compact_timeout_seconds))
    .send()
    .await?;
    let status = r.status().as_u16();
    let mut stream = r.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= app.config.max_observation_bytes,
            "compact output too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, serde_json::from_slice(&bytes)?))
}

#[allow(clippy::too_many_arguments)]
async fn forward(
    app: App,
    headers: HeaderMap,
    outgoing: Vec<u8>,
    bytes_in: usize,
    suffix: &str,
    mut pending: Option<Pending>,
    reason: &'static str,
    request_stream: bool,
) -> Response {
    let start = Instant::now();
    let bytes_out = outgoing.len();
    let lane = pending.as_ref().map(|p| p.0.clone());
    let response = match request(&app, &headers, suffix, outgoing, false)
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => {
            eprintln!(
                "upstream transport error: {:#}",
                anyhow::Error::new(err.without_url())
            );
            app.store.record(&json!({"kind":"response","ts_ms":now_ms(),"lane":lane,"reason":reason,"outcome":"transport_error","bytes_in":bytes_in,"bytes_out":bytes_out})).await;
            return error(
                StatusCode::BAD_GATEWAY,
                "upstream transport error; projection was not committed",
            );
        }
    };
    let status = response.status();
    let sse = match response.headers().get("content-type") {
        Some(v) => v.to_str().is_ok_and(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("text/event-stream")
        }),
        None => {
            request_stream
                || headers
                    .get("accept")
                    .is_some_and(|v| v.to_str().is_ok_and(|v| v.contains("text/event-stream")))
        }
    };
    let mut builder = Response::builder().status(status.as_u16());
    for (k, v) in response.headers() {
        if !strip_header(k.as_str(), response.headers()) {
            builder = builder.header(k, v);
        }
    }
    let mut chunks = response.bytes_stream();
    let inline_bytes = pending.as_ref().and_then(|p| p.3);
    let stream = async_stream::stream! {
        let mut observer = Observer::new(sse, app.config.max_observation_bytes);
        if inline_bytes.is_some() { observer.capture_output(); }
        let mut response_bytes = 0usize;
        let mut first_byte_ms = None;
        let mut clean_eof = true;
        while let Some(chunk) = chunks.next().await {
            match chunk {
                Ok(bytes) => {
                    if first_byte_ms.is_none() { first_byte_ms = Some(start.elapsed().as_millis()); }
                    response_bytes += bytes.len(); observer.feed(&bytes);
                    yield Ok::<Bytes, std::io::Error>(bytes);
                }
                Err(_) => {
                    clean_eof = false;
                    // Record before yielding error: the downstream may drop us immediately afterwards.
                    app.store.record(&json!({"kind":"response","ts_ms":now_ms(),"lane":lane,"reason":reason,"outcome":"stream_error","bytes_in":bytes_in,"bytes_out":bytes_out,"response_bytes":response_bytes})).await;
                    yield Err(std::io::Error::other("upstream stream interrupted"));
                    break;
                }
            }
        }
        observer.finish();
        let inline_output = inline_bytes.and_then(|_| observer.output());
        let complete = clean_eof && status.is_success() && observer.completed()
            && (inline_bytes.is_none() || inline_output.is_some());
        let mut committed = false;
        let mut state_error = false;
        let mut inline_adopted = 0usize;
        let mut inline_rejection = None;
        let inline_observed = inline_output.as_ref().map_or(0, |items| items.iter().filter(|i| i["type"] == "compaction").count());
        if complete {
            if let Some((id, guard, candidate, _, _)) = pending.as_mut() {
                if let (Some(output), Some(bytes)) = (&inline_output, inline_bytes) {
                    match engine::adopt_inline(candidate, output, bytes, app.config.min_savings) {
                        Ok(count) => inline_adopted = count,
                        Err(error) => inline_rejection = Some(error.to_string()),
                    }
                }
                candidate.last_success_ms = now_ms();
                match app.store.commit(id, candidate).await {
                    Ok(()) => { **guard = Some(candidate.clone()); committed = true; }
                    Err(e) => { state_error = true; eprintln!("projection persistence failed: {e}"); }
                }
            }
        }
        if clean_eof {
            app.store.record(&json!({"kind":"response", "ts_ms":now_ms(), "lane":lane, "reason":reason,
                "outcome":if complete { "completed" } else { "not_completed" }, "status":status.as_u16(),
                "epoch":pending.as_ref().map(|p| p.2.epoch), "committed":committed, "state_error":state_error,
                "cut":pending.as_ref().map(|p| p.2.cut),
                "projection_sha256":pending.as_ref().map(|p| crate::fingerprint(&json!(p.2.projection))),
                "effective_input_sha256":pending.as_ref().map(|p| &p.4),
                "inline_scheduled":inline_bytes.is_some(), "inline_checkpoints_observed":inline_observed,
                "inline_checkpoints_adopted":if committed { inline_adopted } else { 0 },
                "inline_rejection_reason":inline_rejection,
                "bytes_in":bytes_in, "bytes_out":bytes_out, "response_bytes":response_bytes,
                "first_byte_ms":first_byte_ms, "elapsed_ms":start.elapsed().as_millis(),
                "observation_overflow":observer.overflow, "usage":observer.usage,
                "cache_hit_rate":observer.usage.as_ref().and_then(Usage::hit_rate)})).await;
        }
        // Dropping the generator at any earlier yield drops the guard without committing.
        drop(pending);
    };
    builder.body(Body::from_stream(stream)).unwrap()
}
