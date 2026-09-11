//! A launcher-owned loopback relay with an isolated task/runtime lifetime.
//!
//! Axum upgrades spawn connection tasks independently of its listener. A private
//! runtime lets cleanup stop those tasks too, without affecting another server
//! or the launcher's runtime. No runtime configuration comes from bundle data.

use crate::config::{CompactionBackend, Config, Mode, NativeToolBindingMode, Retention};
use crate::project::{Error, Result};
use crate::proxy::{App, router};
use axum::routing::get;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

const UPSTREAM: &str = "https://chatgpt.com/backend-api/codex";
const START_TIMEOUT: Duration = Duration::from_secs(5);
const GRACE_TIMEOUT: Duration = Duration::from_millis(250);
const RUNTIME_TIMEOUT: Duration = Duration::from_millis(250);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

fn failure(code: &'static str, message: &'static str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

fn configuration(state_dir: &Path) -> Config {
    // Construct directly: Clap's environment-backed defaults must not change
    // this launcher's upstream, certificate policy, or operating mode.
    Config {
        listen: SocketAddr::from(([127, 0, 0, 1], 0)),
        upstream: UPSTREAM.into(),
        upstream_ca_bundle: None,
        compact_path: "/responses/compact".into(),
        compaction_backend: CompactionBackend::Standalone,
        inline_threshold_tokens: 8192,
        inline_tool_boundaries: false,
        economic_roll_policy: false,
        mode: Mode::Passthrough,
        native_tool_binding: NativeToolBindingMode::Rebind,
        state_dir: state_dir.to_owned(),
        allow_compatible_compaction: false,
        retention: Retention::ProviderDefault,
        roll_bytes: 160_000,
        keep_recent_turns: 2,
        min_compact_bytes: 32_000,
        idle_roll_seconds: 0,
        min_roll_seconds: 60,
        min_savings: 0.15,
        max_body_bytes: 32 * 1024 * 1024,
        max_observation_bytes: 16 * 1024 * 1024,
        max_sessions: 1024,
        compact_timeout_seconds: 180,
        request_timeout_seconds: 900,
    }
}

struct Ready {
    address: SocketAddr,
    listener: AbortHandle,
}

/// This value owns only its listener, runtime and state-directory lock. The
/// caller owns the state directory and retains any aggregate diagnostic files.
pub struct ManagedProxy {
    base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
    finished: Option<oneshot::Receiver<Result<()>>>,
    listener: Option<AbortHandle>,
}

impl ManagedProxy {
    /// Start and locally verify the fixed Codex upstream's native rebinding relay.
    /// This makes no request to the upstream and reads no authentication files.
    pub async fn start(state_dir: &Path) -> Result<Self> {
        Self::start_config(configuration(state_dir)).await
    }

    /// The process-scoped OpenAI base URL, including `/backend-api/codex`.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn start_config(config: Config) -> Result<Self> {
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|_| {
            failure(
                "MANAGED_PROXY_START_FAILED",
                "could not create managed proxy readiness token",
            )
        })?;
        let token = crate::hash(&random);
        let path = format!("/__astral_managed_ready/{token}");
        let (ready_tx, ready_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (finished_tx, finished_rx) = oneshot::channel();
        let mut proxy = Self {
            base_url: String::new(),
            shutdown: Some(shutdown_tx),
            finished: Some(finished_rx),
            listener: None,
        };
        let worker_path = path.clone();
        let worker_token = token.clone();
        // Dropping this handle detaches only the OS thread handle; the shutdown
        // channel and finished receipt retain explicit ownership of its lifetime.
        std::thread::Builder::new()
            .name("astral-managed-proxy".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let result = match runtime {
                    Ok(runtime) => {
                        let result = runtime.block_on(serve(
                            config,
                            worker_path,
                            worker_token,
                            ready_tx,
                            shutdown_rx,
                        ));
                        // This also drops HTTP and upgraded WebSocket tasks spawned
                        // by Axum. Blocking filesystem work has a bounded wait.
                        runtime.shutdown_timeout(RUNTIME_TIMEOUT);
                        result
                    }
                    Err(_) => Err(failure(
                        "MANAGED_PROXY_START_FAILED",
                        "could not create managed proxy runtime",
                    )),
                };
                let _ = finished_tx.send(result);
            })
            .map_err(|_| {
                failure(
                    "MANAGED_PROXY_START_FAILED",
                    "could not start managed proxy worker",
                )
            })?;

        let ready = match tokio::time::timeout(START_TIMEOUT, ready_rx).await {
            Ok(Ok(ready)) => ready,
            _ => {
                let _ = proxy.stop().await;
                return Err(failure(
                    "MANAGED_PROXY_START_FAILED",
                    "managed proxy could not initialize its listener and state",
                ));
            }
        };
        proxy.listener = Some(ready.listener);
        proxy.base_url = format!("http://{}/backend-api/codex", ready.address);
        let readiness = verify_ready(&format!("http://{}{path}", ready.address), &token).await;
        if readiness.is_err() {
            let _ = proxy.stop().await;
            return Err(failure(
                "MANAGED_PROXY_NOT_READY",
                "managed proxy did not pass its local readiness check",
            ));
        }
        Ok(proxy)
    }

    /// Allow a short graceful drain, then tear down all owned connection tasks.
    /// Cancellation of this future still triggers the Drop cleanup path.
    pub async fn stop(mut self) -> Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let result = match self.finished.take() {
            Some(finished) => match tokio::time::timeout(STOP_TIMEOUT, finished).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(failure(
                    "MANAGED_PROXY_STOP_FAILED",
                    "managed proxy worker ended without a cleanup receipt",
                )),
                Err(_) => Err(failure(
                    "MANAGED_PROXY_STOP_TIMEOUT",
                    "managed proxy cleanup exceeded its deadline",
                )),
            },
            None => Ok(()),
        };
        if result.is_ok() {
            self.listener = None;
        }
        result
    }
}

impl Drop for ManagedProxy {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.take() {
            listener.abort();
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

async fn verify_ready(url: &str, token: &str) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|_| {
            failure(
                "MANAGED_PROXY_NOT_READY",
                "could not create local readiness client",
            )
        })?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| failure("MANAGED_PROXY_NOT_READY", "local readiness request failed"))?;
    if !response.status().is_success() {
        return Err(failure(
            "MANAGED_PROXY_NOT_READY",
            "local readiness status was unsuccessful",
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| failure("MANAGED_PROXY_NOT_READY", "local readiness response failed"))?;
    if bytes.as_ref() != token.as_bytes() {
        return Err(failure(
            "MANAGED_PROXY_NOT_READY",
            "local readiness identity did not match",
        ));
    }
    Ok(())
}

async fn serve(
    config: Config,
    path: String,
    token: String,
    ready: oneshot::Sender<Ready>,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<()> {
    let initialization = async {
        let app = App::new(config).await.map_err(|_| {
            failure(
                "MANAGED_PROXY_START_FAILED",
                "managed proxy state or configuration could not initialize",
            )
        })?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| {
                failure(
                    "MANAGED_PROXY_START_FAILED",
                    "managed loopback listener could not bind",
                )
            })?;
        let address = listener.local_addr().map_err(|_| {
            failure(
                "MANAGED_PROXY_START_FAILED",
                "managed loopback listener address is unavailable",
            )
        })?;
        let routes = router(app).route(
            &path,
            get(move || {
                let token = token.clone();
                async move { token }
            }),
        );
        Ok::<_, Error>((listener, address, routes))
    };
    let (listener, address, routes) = tokio::select! {
        result = initialization => result?,
        _ = &mut shutdown => return Ok(()),
    };
    let (graceful_tx, graceful_rx) = oneshot::channel();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, routes)
            .with_graceful_shutdown(async {
                let _ = graceful_rx.await;
            })
            .await
    });
    let _ = ready.send(Ready {
        address,
        listener: server.abort_handle(),
    });
    tokio::select! {
        _ = &mut shutdown => {
            let _ = graceful_tx.send(());
            match tokio::time::timeout(GRACE_TIMEOUT, &mut server).await {
                Ok(result) => server_result(result),
                Err(_) => {
                    server.abort();
                    server_result(server.await)
                }
            }
        }
        result = &mut server => server_result(result),
    }
}

fn server_result(
    result: std::result::Result<std::io::Result<()>, tokio::task::JoinError>,
) -> Result<()> {
    match result {
        Ok(Ok(())) => Ok(()),
        Err(error) if error.is_cancelled() => Ok(()),
        _ => Err(failure(
            "MANAGED_PROXY_SERVE_FAILED",
            "managed proxy listener ended unexpectedly",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{State, WebSocketUpgrade, ws::Message};
    use axum::response::Response;
    use futures_util::StreamExt;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    async fn open_socket(State(closed): State<Arc<AtomicUsize>>, ws: WebSocketUpgrade) -> Response {
        ws.on_upgrade(move |mut socket| async move {
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Close(_) = message {
                    break;
                }
            }
            closed.fetch_add(1, Ordering::SeqCst);
        })
    }

    #[tokio::test]
    async fn stop_and_drop_close_active_upgrades_without_stopping_another_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let closed_upstreams = Arc::new(AtomicUsize::new(0));
        let routes = axum::Router::new()
            .route("/backend-api/codex/responses", get(open_socket))
            .route(
                "/alive",
                get(|| async { "unrelated fixture remains alive" }),
            )
            .with_state(closed_upstreams.clone());
        let upstream = tokio::spawn(async move {
            axum::serve(listener, routes).await.unwrap();
        });
        for (index, abrupt) in [false, true].into_iter().enumerate() {
            let state = tempfile::tempdir().unwrap();
            let mut config = configuration(state.path());
            // Upstream injection exists only in this private unit test.
            config.upstream = format!("http://{address}/backend-api/codex");
            let proxy = ManagedProxy::start_config(config).await.unwrap();
            let mut request = format!("{}/responses", proxy.base_url())
                .replacen("http://", "ws://", 1)
                .into_client_request()
                .unwrap();
            request.headers_mut().insert(
                "authorization",
                "Bearer synthetic-managed-test".parse().unwrap(),
            );
            let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
            if abrupt {
                drop(proxy);
            } else {
                proxy.stop().await.unwrap();
            }
            let closed = tokio::time::timeout(STOP_TIMEOUT, socket.next())
                .await
                .expect("owned WebSocket must close");
            assert!(
                closed.is_none()
                    || closed.is_some_and(|value| value.is_err()
                        || matches!(value, Ok(tokio_tungstenite::tungstenite::Message::Close(_))))
            );
            tokio::time::timeout(STOP_TIMEOUT, async {
                while closed_upstreams.load(Ordering::SeqCst) <= index {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("owned upstream WebSocket must also close");
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}/alive"))
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
        }
        upstream.abort();
        let _ = upstream.await;
    }

    #[test]
    fn managed_configuration_fixes_routing_and_disables_rolling() {
        let config = configuration(Path::new("synthetic-state"));
        assert_eq!(config.upstream, UPSTREAM);
        assert_eq!(config.listen, SocketAddr::from(([127, 0, 0, 1], 0)));
        assert_eq!(config.mode, Mode::Passthrough);
        assert_eq!(config.native_tool_binding, NativeToolBindingMode::Rebind);
        assert!(config.upstream_ca_bundle.is_none());
        assert!(!config.allow_compatible_compaction);
        assert!(!config.inline_tool_boundaries);
        config.validate().unwrap();
    }
}
