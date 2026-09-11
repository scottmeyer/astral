//! HTTP forwarding policies shared by inference, compaction and model discovery.
use super::{App, error};
use axum::{
    body::{Body, Bytes},
    extract::{OriginalUri, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, response::Builder},
    response::Response,
};
use futures_util::StreamExt;
use std::sync::atomic::Ordering;

const HOP: &str = "x-astral-hop";

pub(crate) fn effective_headers(
    mut headers: HeaderMap,
) -> Result<HeaderMap, (StatusCode, &'static str)> {
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
        "x-astral-session-id",
        "session_id",
        "openai-session-id",
        "x-astral-roll-estimate",
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

pub(crate) fn strip_header(name: &str, headers: &HeaderMap) -> bool {
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
    ) || name.starts_with("x-astral-")
        || headers
            .get_all("connection")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|v| v.trim().eq_ignore_ascii_case(name))
}

fn upstream_request(
    app: &App,
    headers: &HeaderMap,
    method: Method,
    suffix: &str,
    compact: bool,
) -> reqwest::RequestBuilder {
    let url = format!("{}{suffix}", app.config.upstream.trim_end_matches('/'));
    let mut request = app.client.request(method, url);
    for (name, value) in headers {
        if !(strip_header(name.as_str(), headers)
            || compact
                && matches!(
                    name.as_str(),
                    "idempotency-key"
                        | "content-type"
                        | "content-encoding"
                        | "accept"
                        | "x-client-request-id"
                ))
        {
            request = request.header(name, value);
        }
    }
    request = request
        .header(HOP, "1")
        .header("accept-encoding", "identity");
    if compact {
        request = request
            .header("content-type", "application/json")
            .header("accept", "application/json");
    }
    request
}

pub(super) fn request(
    app: &App,
    headers: &HeaderMap,
    suffix: &str,
    body: Vec<u8>,
    compact: bool,
) -> reqwest::RequestBuilder {
    upstream_request(app, headers, Method::POST, suffix, compact).body(body)
}

pub(super) fn response_builder(response: &reqwest::Response) -> Builder {
    let mut builder = Response::builder().status(response.status());
    for (name, value) in response.headers() {
        if !strip_header(name.as_str(), response.headers()) {
            builder = builder.header(name, value);
        }
    }
    builder
}

pub(super) async fn bounded_body(
    response: reqwest::Response,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= limit,
            "upstream output too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Catalog bytes/status/metadata remain provider-owned. No model cache, inference
/// observer, native transformation, or conversation state is involved in this GET.
pub(super) async fn models(
    State(app): State<App>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Response {
    app.requests_received.fetch_add(1, Ordering::Relaxed);
    let headers = match effective_headers(headers) {
        Ok(headers) => headers,
        Err((status, message)) => return error(status, message),
    };
    let suffix = format!(
        "/models{}",
        uri.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    let response = match upstream_request(&app, &headers, Method::GET, &suffix, false)
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => {
            return error(
                StatusCode::BAD_GATEWAY,
                "upstream model catalog transport error",
            );
        }
    };
    let builder = response_builder(&response);
    match bounded_body(response, app.config.max_body_bytes).await {
        Ok(bytes) => builder.body(Body::from(Bytes::from(bytes))).unwrap(),
        Err(_) => error(
            StatusCode::BAD_GATEWAY,
            "upstream model catalog response unavailable or exceeds max-body-bytes",
        ),
    }
}
