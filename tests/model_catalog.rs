use axum::{
    Router,
    body::{Body, Bytes},
    extract::{OriginalUri, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::get,
};
use clap::Parser;
use ostk_gpt_cache::{
    config::{Config, Mode, NativeToolBindingMode},
    proxy::{App, router},
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

const CATALOG: &[u8] = b"{ \"models\": [], \"future_field\": 1.000e+20 }\n";

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
    chunked: bool,
    broken_body: bool,
    delay: Duration,
}
struct Seen {
    uri: String,
    headers: HeaderMap,
    body: Bytes,
}
struct Upstream {
    reply: Mutex<Reply>,
    seen: Mutex<Vec<Seen>>,
}

async fn models(
    State(state): State<Arc<Upstream>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.seen.lock().unwrap().push(Seen {
        uri: uri.to_string(),
        headers,
        body,
    });
    let reply = state.reply.lock().unwrap().clone();
    tokio::time::sleep(reply.delay).await;
    let body = if reply.broken_body {
        Body::from_stream(async_stream::stream! {
            yield Ok::<_, std::io::Error>(Bytes::from_static(b"SYNTHETIC_PARTIAL_CATALOG"));
            tokio::time::sleep(Duration::from_millis(20)).await;
            yield Err(std::io::Error::other("synthetic private upstream failure"));
        })
    } else if reply.chunked {
        Body::from_stream(futures_util::stream::iter(
            reply
                .body
                .chunks(7)
                .map(|chunk| Ok::<_, std::io::Error>(Bytes::copy_from_slice(chunk)))
                .collect::<Vec<_>>(),
        ))
    } else {
        Body::from(reply.body)
    };
    Response::builder()
        .status(reply.status)
        .header("content-type", "application/json")
        .header("etag", "W/\"catalog-version\"")
        .header("cache-control", "private, max-age=60")
        .header("retry-after", "7")
        .header("www-authenticate", "Bearer realm=synthetic")
        .header("location", "http://invalid.invalid/do-not-follow")
        .header("x-catalog-metadata", "first")
        .header("x-catalog-metadata", "second")
        .header("connection", "x-upstream-hop")
        .header("x-upstream-hop", "remove")
        .header("x-ostk-internal", "remove")
        .body(body)
        .unwrap()
}

struct Fixture {
    root: tempfile::TempDir,
    url: String,
    state: Arc<Upstream>,
    upstream: JoinHandle<()>,
    proxy: JoinHandle<()>,
    client: reqwest::Client,
}

impl Fixture {
    async fn new(native: bool, limit: usize, timeout: u64) -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(Upstream {
            reply: Mutex::new(Reply {
                status: StatusCode::OK,
                body: CATALOG.to_vec(),
                chunked: false,
                broken_body: false,
                delay: Duration::ZERO,
            }),
            seen: Mutex::new(Vec::new()),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let routes = Router::new()
            .route("/backend-api/codex/models", get(models))
            .with_state(state.clone());
        let upstream = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        let mut config = Config::parse_from(["test"]);
        config.upstream = format!("http://{address}/backend-api/codex/");
        config.state_dir = root.path().join("state");
        config.upstream_ca_bundle = None;
        config.max_body_bytes = limit;
        config.roll_bytes = 1;
        config.min_compact_bytes = 1;
        config.request_timeout_seconds = timeout;
        config.mode = if native {
            Mode::Passthrough
        } else {
            Mode::Rolling
        };
        config.native_tool_binding = if native {
            NativeToolBindingMode::Rebind
        } else {
            NativeToolBindingMode::Disabled
        };
        let app = App::new(config).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let proxy = tokio::spawn(async move { axum::serve(listener, router(app)).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        Self {
            root,
            url,
            state,
            upstream,
            proxy,
            client,
        }
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}{path}", self.url))
            .bearer_auth("synthetic-alpha")
    }

    fn no_conversation_state(&self) {
        assert!(
            std::fs::read(self.root.path().join("state/ledger.jsonl"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read_dir(self.root.path().join("state/lanes"))
                .unwrap()
                .count(),
            0
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.proxy.abort();
        self.upstream.abort();
    }
}

#[tokio::test]
async fn discovery_preserves_query_identity_bytes_and_provider_metadata_in_both_modes() {
    for native in [false, true] {
        let f = Fixture::new(native, 4096, 5).await;
        for prefix in ["/models", "/v1/models", "/backend-api/codex/models"] {
            let response = f
                .get(&format!(
                    "{prefix}?client_version=0.154.0&filter=a%2Fb&x=1&x=2"
                ))
                .header("chatgpt-account-id", "synthetic-account-alpha")
                .header("openai-organization", "synthetic-org")
                .header("openai-project", "synthetic-project")
                .header("if-none-match", "W/\"previous\"")
                .header("x-client-request-id", "synthetic-request")
                .header("connection", "x-client-hop")
                .header("x-client-hop", "remove")
                .header("x-ostk-session-id", "internal-only")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["etag"], "W/\"catalog-version\"");
            assert_eq!(response.headers()["cache-control"], "private, max-age=60");
            assert_eq!(
                response
                    .headers()
                    .get_all("x-catalog-metadata")
                    .iter()
                    .count(),
                2
            );
            assert!(!response.headers().contains_key("connection"));
            assert!(!response.headers().contains_key("x-upstream-hop"));
            assert!(!response.headers().contains_key("x-ostk-internal"));
            assert_eq!(response.bytes().await.unwrap().as_ref(), CATALOG);
        }
        let seen = f.state.seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        for request in seen.iter() {
            assert_eq!(
                request.uri,
                "/backend-api/codex/models?client_version=0.154.0&filter=a%2Fb&x=1&x=2"
            );
            assert_eq!(request.headers["authorization"], "Bearer synthetic-alpha");
            assert_eq!(
                request.headers["chatgpt-account-id"],
                "synthetic-account-alpha"
            );
            assert_eq!(request.headers["openai-organization"], "synthetic-org");
            assert_eq!(request.headers["openai-project"], "synthetic-project");
            assert_eq!(request.headers["if-none-match"], "W/\"previous\"");
            assert_eq!(request.headers["x-client-request-id"], "synthetic-request");
            assert_eq!(request.headers["x-ostk-gpt-hop"], "1");
            assert_eq!(request.headers["accept-encoding"], "identity");
            assert!(!request.headers.contains_key("x-client-hop"));
            assert!(!request.headers.contains_key("x-ostk-session-id"));
            assert!(request.body.is_empty());
        }
        f.no_conversation_state();
    }
}

#[tokio::test]
async fn discovery_does_not_cache_across_accounts_and_preserves_upstream_failures() {
    let f = Fixture::new(true, 4096, 5).await;
    for (index, status) in [
        StatusCode::OK,
        StatusCode::UNAUTHORIZED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::TEMPORARY_REDIRECT,
        StatusCode::NOT_MODIFIED,
    ]
    .into_iter()
    .enumerate()
    {
        let bytes = if status == StatusCode::NOT_MODIFIED {
            vec![]
        } else {
            format!("provider bytes {index}").into_bytes()
        };
        {
            let mut reply = f.state.reply.lock().unwrap();
            reply.status = status;
            reply.body = bytes.clone();
        }
        let response = f
            .client
            .get(format!("{}/models", f.url))
            .bearer_auth(format!("synthetic-{index}"))
            .header("chatgpt-account-id", format!("account-{index}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["etag"], "W/\"catalog-version\"");
        assert_eq!(response.headers()["retry-after"], "7");
        assert_eq!(
            response.headers()["www-authenticate"],
            "Bearer realm=synthetic"
        );
        assert_eq!(response.bytes().await.unwrap().as_ref(), bytes);
        let seen = f.state.seen.lock().unwrap();
        assert_eq!(seen.len(), index + 1);
        assert_eq!(
            seen[index].headers["authorization"],
            format!("Bearer synthetic-{index}")
        );
        assert_eq!(
            seen[index].headers["chatgpt-account-id"],
            format!("account-{index}")
        );
    }
    f.no_conversation_state();
}

#[tokio::test]
async fn duplicate_identity_loops_and_unrelated_routes_do_not_reach_upstream() {
    let f = Fixture::new(true, 4096, 5).await;
    for header in [
        "authorization",
        "chatgpt-account-id",
        "openai-organization",
        "openai-project",
    ] {
        let mut headers = HeaderMap::new();
        headers.append(header, "first".parse().unwrap());
        headers.append(header, "second".parse().unwrap());
        let response = f
            .client
            .get(format!("{}/models", f.url))
            .headers(headers)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        f.get("/models")
            .header("x-ostk-gpt-hop", "1")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::LOOP_DETECTED
    );
    assert_eq!(
        f.client
            .post(format!("{}/models", f.url))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        f.get("/backend-api/codex/arbitrary")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(f.state.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn catalog_body_and_request_time_are_bounded() {
    let f = Fixture::new(true, CATALOG.len(), 1).await;
    // Exactly the configured limit is accepted, with and without Content-Length.
    for chunked in [false, true] {
        {
            let mut reply = f.state.reply.lock().unwrap();
            reply.chunked = chunked;
            reply.body = CATALOG.to_vec();
        }
        assert_eq!(
            f.get("/models")
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .as_ref(),
            CATALOG
        );
        f.state.reply.lock().unwrap().body.push(b' ');
        assert_eq!(
            f.get("/models").send().await.unwrap().status(),
            StatusCode::BAD_GATEWAY
        );
    }
    f.state.reply.lock().unwrap().delay = Duration::from_secs(3);
    let response = tokio::time::timeout(Duration::from_secs(2), f.get("/models").send())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let error = response.text().await.unwrap();
    assert!(!error.contains("synthetic-alpha") && !error.contains(&f.url));
    f.no_conversation_state();
}

#[tokio::test]
async fn interrupted_catalog_body_returns_sanitized_error_without_partial_bytes() {
    let f = Fixture::new(true, 4096, 5).await;
    f.state.reply.lock().unwrap().broken_body = true;
    let response = f.get("/models").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = response.text().await.unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&body).unwrap()["error"].is_object());
    assert!(!body.contains("SYNTHETIC_PARTIAL_CATALOG"));
    assert!(!body.contains("synthetic private upstream failure"));
    assert!(!body.contains("synthetic-alpha"));
    f.no_conversation_state();
}
