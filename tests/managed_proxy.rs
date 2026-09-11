use astral::managed_proxy::ManagedProxy;
use serde_json::Value;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

fn address(proxy: &ManagedProxy) -> SocketAddr {
    let url = reqwest::Url::parse(proxy.base_url()).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.path(), "/backend-api/codex");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert!(url.username().is_empty());
    assert!(url.password().is_none());
    assert!(url.query().is_none());
    assert!(url.fragment().is_none());
    SocketAddr::from(([127, 0, 0, 1], url.port().unwrap()))
}

async fn health(address: SocketAddr) -> Value {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let response = client
        .get(format!("http://{address}/healthz"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.json().await.unwrap()
}

async fn released(address: SocketAddr) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(listener) = tokio::net::TcpListener::bind(address).await {
                drop(listener);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned listener must release its port");
}

async fn start_error(state: &Path) -> astral::project::Error {
    match ManagedProxy::start(state).await {
        Ok(proxy) => {
            proxy.stop().await.unwrap();
            panic!("fixture should fail initialization")
        }
        Err(error) => error,
    }
}

#[tokio::test]
async fn start_returns_ready_loopback_endpoint_and_stop_releases_listener_and_state() {
    let state = tempfile::tempdir().unwrap();
    let proxy = ManagedProxy::start(state.path()).await.unwrap();
    let bound = address(&proxy);
    assert_ne!(bound.port(), 0);
    let metadata = health(bound).await;
    assert_eq!(metadata["status"], "ok");
    assert_eq!(metadata["requests_received"], 0);
    proxy.stop().await.unwrap();
    released(bound).await;
    // Cleanup releases the Store lock but preserves caller-owned diagnostics.
    assert!(state.path().join("process.lock").is_file());
    assert!(state.path().join("ledger.jsonl").is_file());
    assert_eq!(
        std::fs::metadata(state.path().join("ledger.jsonl"))
            .unwrap()
            .len(),
        0
    );
    ManagedProxy::start(state.path())
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn drop_closes_owned_listener_without_stopping_another_managed_instance() {
    let first_state = tempfile::tempdir().unwrap();
    let second_state = tempfile::tempdir().unwrap();
    let first = ManagedProxy::start(first_state.path()).await.unwrap();
    let second = ManagedProxy::start(second_state.path()).await.unwrap();
    let first_address = address(&first);
    let second_address = address(&second);
    assert_ne!(first_address, second_address);
    drop(first);
    released(first_address).await;
    assert_eq!(health(second_address).await["status"], "ok");
    second.stop().await.unwrap();
    released(second_address).await;
}

#[tokio::test]
async fn occupied_state_fails_without_disturbing_owner_or_disclosing_its_path() {
    let state = tempfile::tempdir().unwrap();
    let owner = ManagedProxy::start(state.path()).await.unwrap();
    let owner_address = address(&owner);
    let error = start_error(state.path()).await;
    assert_eq!(error.code, "MANAGED_PROXY_START_FAILED");
    assert!(
        !error
            .message
            .contains(&state.path().to_string_lossy().to_string())
    );
    assert_eq!(health(owner_address).await["status"], "ok");
    owner.stop().await.unwrap();
    ManagedProxy::start(state.path())
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn unavailable_state_fails_without_changing_existing_file() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state-is-a-file");
    std::fs::write(&state, "preserve this caller-owned file").unwrap();
    let error = start_error(&state).await;
    assert_eq!(error.code, "MANAGED_PROXY_START_FAILED");
    assert_eq!(
        std::fs::read_to_string(&state).unwrap(),
        "preserve this caller-owned file"
    );
}
