//! Additional wire adapters share the listener, archive, transport policy and ledger.
use super::{App, error, http};
use crate::{
    now_ms,
    providers::Wire,
    usage::{Observer, Usage},
};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, OriginalUri, Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::atomic::Ordering, time::Instant};

pub(super) fn archive_scope(app: &App, headers: &HeaderMap, body: &Value) -> String {
    let identities: Vec<_> = [
        "authorization",
        "api-key",
        "x-api-key",
        "chatgpt-account-id",
        "openai-organization",
        "openai-project",
        "x-astral-session-id",
        "session_id",
        "openai-session-id",
        "x-astral-workspace-id",
        "x-astral-harness-id",
    ]
    .iter()
    .map(|name| (*name, headers.get(*name).map(|v| v.as_bytes())))
    .collect();
    // Without explicit conversation identity we share only immutable equal content;
    // no inferred session ever drives native rolling or mutable conversation state.
    let first_user = body["input"]
        .as_array()
        .or_else(|| body["messages"].as_array())
        .and_then(|messages| messages.iter().find(|m| m["role"] == "user"));
    crate::fingerprint(&json!([
        app.provider.name,
        app.config.upstream,
        identities,
        body["model"],
        body["metadata"],
        body["system"],
        body["instructions"],
        body["tools"],
        first_user
    ]))
}

pub(super) fn anthropic_messages(app: App) -> Router {
    let limit = app.config.max_body_bytes;
    Router::new()
        .route("/v1/messages", post(messages))
        .route("/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route("/messages/count_tokens", post(count_tokens))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(app)
}

pub(super) fn anthropic_router(app: App) -> Router {
    anthropic_messages(app.clone()).merge(
        Router::new()
            .route("/v1/models", get(http::models))
            .route("/models", get(http::models))
            .with_state(app),
    )
}

pub(super) async fn chat(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    relay(
        app,
        uri,
        headers,
        body,
        Wire::Chat,
        "/chat/completions",
        true,
    )
    .await
}
async fn messages(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    relay(app, uri, headers, body, Wire::Messages, "/messages", true).await
}
async fn count_tokens(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Counting observes the caller's exact input and never archives or rewrites it.
    relay(
        app,
        uri,
        headers,
        body,
        Wire::Messages,
        "/messages/count_tokens",
        false,
    )
    .await
}

async fn relay(
    app: App,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: Bytes,
    wire: Wire,
    path: &str,
    reduce: bool,
) -> Response {
    app.requests_received.fetch_add(1, Ordering::Relaxed);
    let headers = match app.headers(headers) {
        Ok(h) => h,
        Err((s, m)) => return error(s, m),
    };
    let suffix = format!(
        "{path}{}",
        uri.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    let mut outgoing = body.to_vec();
    let mut streaming = false;
    let compressed = headers
        .get("content-encoding")
        .is_some_and(|v| v != "identity");
    if !compressed {
        if let Ok(mut value) = serde_json::from_slice::<Value>(&body) {
            streaming = value["stream"] == true;
            if reduce && app.reduce_tools(&mut value, &headers, wire).await {
                outgoing = serde_json::to_vec(&value).unwrap();
            }
        }
    }
    let start = Instant::now();
    let bytes_out = outgoing.len();
    let response = match http::upstream_request(&app, &headers, Method::POST, &suffix, false)
        .body(outgoing)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return error(StatusCode::BAD_GATEWAY, "upstream transport error"),
    };
    let status = response.status();
    let sse = response
        .headers()
        .get("content-type")
        .map(|v| {
            v.to_str().is_ok_and(|s| {
                s.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("text/event-stream")
            })
        })
        .unwrap_or(streaming);
    let builder = http::response_builder(&response);
    let mut chunks = response.bytes_stream();
    let stream = async_stream::stream! {
        let mut observer = Observer::with_wire(sse, app.config.max_observation_bytes, wire);
        let mut clean = true;
        let mut response_bytes = 0usize;
        while let Some(chunk) = chunks.next().await {
            match chunk {
                Ok(bytes) => { response_bytes += bytes.len(); observer.feed(&bytes); yield Ok::<Bytes,std::io::Error>(bytes); }
                Err(_) => { clean = false; yield Err(std::io::Error::other("upstream stream interrupted")); break; }
            }
        }
        observer.finish();
        app.store.record(&json!({"kind":"response","provider":app.provider.name,"ts_ms":now_ms(),"status":status.as_u16(),"outcome":if clean && status.is_success() && observer.completed() {"completed"} else {"not_completed"},"bytes_in":body.len(),"bytes_out":bytes_out,"response_bytes":response_bytes,"elapsed_ms":start.elapsed().as_millis(),"usage":observer.usage,"observation_overflow":observer.overflow,"cache_hit_rate":observer.usage.as_ref().and_then(Usage::hit_rate)})).await;
    };
    builder.body(Body::from_stream(stream)).unwrap()
}

#[derive(Deserialize)]
struct Retrieval {
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_size")]
    limit: usize,
    query: Option<String>,
}
fn page_size() -> usize {
    4096
}

pub(super) fn artifact_router(app: App) -> Router {
    let limit = app.config.max_body_bytes;
    Router::new()
        .route("/_astral/artifacts/{handle}", get(retrieve))
        .route("/_astral/intake", post(intake))
        .layer(DefaultBodyLimit::max(limit))
        .with_state(app)
}

async fn intake(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    if headers.contains_key("origin") || !app.config.listen.ip().is_loopback() {
        return error(
            StatusCode::FORBIDDEN,
            "tool intake requires a loopback listener without browser origins",
        );
    }
    let Ok(mut request) = serde_json::from_slice::<crate::tool_intake::Request>(&body) else {
        return error(StatusCode::BAD_REQUEST, "invalid tool intake request");
    };
    if !crate::archive::valid_handle(&request.scope)
        || request.tool.is_empty()
        || request.tool.len() > 256
    {
        return error(StatusCode::BAD_REQUEST, "invalid tool intake scope or name");
    }
    let worker = app.clone();
    let scope = request.scope.clone();
    let result = tokio::task::spawn_blocking(move || {
        // Keep the Store's writer lock through durable publication on cancellation.
        let report = crate::tool_intake::compact(&mut request, &worker.config, &worker.archive);
        drop(worker);
        (request.result, report)
    })
    .await;
    let Ok((result, report)) = result else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "tool intake unavailable");
    };
    app.store
        .record(&json!({"kind":"tool_intake", "scope":scope, "ts_ms":now_ms(), "report":report}))
        .await;
    let mut response = axum::Json(json!({"result":result, "report":report})).into_response();
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

async fn retrieve(
    State(app): State<App>,
    Path(handle): Path<String>,
    Query(query): Query<Retrieval>,
    headers: HeaderMap,
) -> Response {
    // No browser-origin access and no listings. The full opaque handle is the capability.
    if headers.contains_key("origin") {
        return error(
            StatusCode::FORBIDDEN,
            "browser origins cannot retrieve artifacts",
        );
    }
    if !crate::archive::valid_handle(&handle) {
        return error(StatusCode::BAD_REQUEST, "invalid artifact handle");
    }
    if query.limit == 0
        || query.limit > crate::archive::PAGE_LIMIT
        || query
            .query
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 1024)
    {
        return error(StatusCode::BAD_REQUEST, "invalid retrieval page or query");
    }
    let result = tokio::task::spawn_blocking(move || match query.query {
        Some(text) => app.archive.search(&handle, &text, query.offset),
        None => app.archive.recall(&handle, query.offset, query.limit),
    })
    .await;
    match result {
        Ok(Ok(value)) => {
            let mut response = axum::Json(value).into_response();
            response
                .headers_mut()
                .insert("cache-control", "no-store".parse().unwrap());
            response
        }
        _ => error(
            StatusCode::NOT_FOUND,
            "artifact unavailable or invalid retrieval offset",
        ),
    }
}
