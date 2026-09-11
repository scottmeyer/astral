use ostk_gpt_cache::working::{Profile, Runtime, check_summary};
use serde_json::{Value, json};

fn profile() -> Profile {
    serde_json::from_value(json!({"files":[{"path":"app.txt","writable":true},{"path":"tests.txt","writable":false}],"checks":{}})).unwrap()
}
fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("app.txt"),
        "first\nsecond\nold detail: keep this exact\n",
    )
    .unwrap();
    std::fs::write(root.path().join("tests.txt"), "protected").unwrap();
    (root, state)
}

#[tokio::test]
async fn exact_artifacts_survive_edits_restart_and_action_replay() {
    let (root, state) = setup();
    let mut runtime = Runtime::open(root.path(), state.path(), profile(), true).unwrap();
    let read = runtime.dispatch(&json!({"op":"read_file","path":"app.txt","start_line":1,"lines":1,"action_id":"read-1"})).await.unwrap();
    assert_eq!(read["text"], "first");
    let write = json!({"op":"write_file","path":"app.txt","expected_sha256":read["version"]["sha256"],"content":"new file","action_id":"edit-1"});
    let receipt = runtime.dispatch(&write).await.unwrap();
    assert_eq!(receipt["version"]["generation"], 2);
    assert!(runtime.dispatch(&json!({"op":"write_file","path":"app.txt","expected_sha256":read["version"]["sha256"],"content":"stale"})).await.is_err());
    drop(runtime);
    let mut runtime = Runtime::open(root.path(), state.path(), profile(), true).unwrap();
    assert_eq!(runtime.dispatch(&write).await.unwrap(), receipt);
    let recalled = runtime
        .dispatch(&json!({"op":"recall","handle":read["handle"],"offset":0,"limit":4096}))
        .await
        .unwrap();
    assert_eq!(
        recalled["data"],
        "first\nsecond\nold detail: keep this exact\n"
    );
    let found = runtime
        .dispatch(&json!({"op":"search","handle":read["handle"],"query":"old detail"}))
        .await
        .unwrap();
    assert_eq!(found["matches"][0]["offset"], 13);
    let mut reused = write;
    reused["content"] = json!("other");
    assert!(runtime.dispatch(&reused).await.is_err());
}

#[tokio::test]
async fn permissions_scope_integrity_and_binary_boundaries_are_enforced() {
    let (root, state) = setup();
    let mut r = Runtime::open(root.path(), state.path(), profile(), true).unwrap();
    assert!(
        r.dispatch(&json!({"op":"read_file","path":"../outside"}))
            .await
            .is_err()
    );
    assert!(r.dispatch(&json!({"op":"write_file","path":"tests.txt","expected_sha256":null,"content":"changed"})).await.is_err());
    assert!(
        r.dispatch(&json!({"op":"recall","handle":"../events.jsonl"}))
            .await
            .is_err()
    );
    std::fs::write(root.path().join("app.txt"), "é").unwrap();
    let read = r
        .dispatch(&json!({"op":"read_file","path":"app.txt"}))
        .await
        .unwrap();
    let page = r
        .dispatch(&json!({"op":"recall","handle":read["handle"],"offset":0,"limit":1}))
        .await
        .unwrap();
    assert_eq!(page["encoding"], "hex");
    assert_eq!(page["data"], "c3");
    std::fs::write(
        state
            .path()
            .join("blobs")
            .join(read["handle"].as_str().unwrap()),
        "tamper",
    )
    .unwrap();
    assert!(
        r.dispatch(&json!({"op":"recall","handle":read["handle"]}))
            .await
            .is_err()
    );
    assert!(Runtime::open(root.path(), state.path(), profile(), true).is_err());
}

#[tokio::test]
async fn journal_replay_preserves_sources_and_external_mutations_invalidate_checks() {
    let (root, state) = setup();
    let mut r = Runtime::open(root.path(), state.path(), profile(), true).unwrap();
    r.dispatch(&json!({"op":"constraint","id":"budget","text":"17","source":"user turn 1"}))
        .await
        .unwrap();
    r.dispatch(
        &json!({"op":"constraint","id":"budget","text":"23","source":"user correction turn 2"}),
    )
    .await
    .unwrap();
    let snap = r.dispatch(&json!({"op":"snapshot"})).await.unwrap();
    drop(r);
    let path = state.path().join("events.jsonl");
    let mut lines = std::fs::read_to_string(&path).unwrap();
    let seq = lines.lines().count() + 1;
    lines.push_str(&format!("{}\n",json!({"sequence":seq,"kind":"check","value":{"name":"tests","input_digest":snap["input_digest"],"stable_during_run":true,"passed":true}})));
    lines.push_str("{\"sequence\":");
    std::fs::write(&path, lines).unwrap();
    let mut r = Runtime::open(root.path(), state.path(), profile(), true).unwrap();
    assert_eq!(
        r.dispatch(&json!({"op":"snapshot"})).await.unwrap()["checks"]["tests"]["current"],
        true
    );
    std::fs::write(root.path().join("app.txt"), "external change").unwrap();
    let snap = r.dispatch(&json!({"op":"snapshot"})).await.unwrap();
    assert_eq!(snap["checks"]["tests"]["current"], false);
    assert_eq!(snap["constraints"]["budget"]["text"], "23");
    assert_eq!(snap["files"]["app.txt"]["generation"], 2);
    let journal = std::fs::read_to_string(path).unwrap();
    assert!(journal.contains("user turn 1") && journal.contains("user correction turn 2"));
}

#[tokio::test]
async fn an_interrupted_action_cannot_be_reexecuted_implicitly() {
    let (root, state) = setup();
    let r = Runtime::open(root.path(), state.path(), profile(), true).unwrap();
    drop(r);
    let request = json!({"op":"write_file","path":"app.txt","expected_sha256":null,"content":"unsafe replay","action_id":"uncertain"});
    let path = state.path().join("events.jsonl");
    let mut journal = std::fs::read_to_string(&path).unwrap();
    let event = json!({"sequence":journal.lines().count()+1,"kind":"action","value":{"id":"uncertain","status":"started","request_sha256":ostk_gpt_cache::fingerprint(&request)}});
    journal.push_str(&format!("{event}\n"));
    std::fs::write(path, journal).unwrap();
    let mut r = Runtime::open(root.path(), state.path(), profile(), true).unwrap();
    assert!(
        r.dispatch(&request)
            .await
            .unwrap_err()
            .to_string()
            .contains("uncertain")
    );
    assert!(
        std::fs::read_to_string(root.path().join("app.txt"))
            .unwrap()
            .starts_with("first")
    );
}

#[test]
fn check_adapter_preserves_failures_warnings_and_exposes_omitted_fields() {
    let report = json!({"schema":"astral.check.v1","passed":9,"failed":1,"failures":[{"test":"bad","detail":"exact error"}],"warnings":["deprecated"],"detail_log":"large", "late_fact":"secret"});
    let summary = check_summary(&serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(summary["failures"], report["failures"]);
    assert_eq!(summary["warnings"], report["warnings"]);
    assert!(
        summary["available_fields"]
            .as_array()
            .unwrap()
            .contains(&json!("late_fact"))
    );
    let mut invalid: Value = report;
    invalid["failed"] = json!(0);
    assert!(check_summary(&serde_json::to_vec(&invalid).unwrap()).is_none());
    assert!(check_summary(b"all tests passed").is_none());
}
