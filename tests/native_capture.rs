use ostk_gpt_cache::hash;
use ostk_gpt_cache::native_capture::{MAX_ROLLOUT_BYTES, capture_bytes};
use serde_json::{Value, json};

const THREAD: &str = "01a08e8f-c4af-77a3-9643-7c474187c260";
const TURN: &str = "save-turn";
const ITEM: &str = "11a08e8f-c4af-77a3-9643-7c474187c260";
const PAYLOAD: &str = r#"[{ "type":"compaction", "encrypted_content":"\u006fpAqUE==", "future":{"n":1234567890123456789012345678901234567890,"large":1e99999,"minus":-0.000e+7} }, {"type":"agent_message","author":"/root/a","recipient":"/root","content":[{"type":"encrypted_content","encrypted_content":"agent-opaque"}]}]"#;

#[derive(Clone)]
struct Fixture {
    before: Vec<u8>,
    suffix: Vec<(&'static str, String)>,
    paginated: bool,
}
fn event(kind: &str) -> Value {
    json!({"type":kind,"turn_id":TURN})
}
fn compact(payload: &str) -> String {
    format!(
        r#"{{"message":"","replacement_history":{payload},"window_number":2,"window_id":"window-two","compaction_response_id":"response-save"}}"#
    )
}
fn encode(records: &[(&str, String)], paginated: bool, start: usize) -> Vec<u8> {
    records.iter().enumerate().map(|(i, (kind, payload))| {
        let ordinal = if paginated { format!(",\"ordinal\":{}", start + i) } else { String::new() };
        format!(r#"{{"timestamp":"2026-09-11T00:00:00Z"{ordinal},"type":"{kind}","payload":{payload}}}"#) + "\n"
    }).collect::<String>().into_bytes()
}
impl Fixture {
    fn new(paginated: bool) -> Self {
        let header = json!({"id":THREAD,"cli_version":"0.154.0","history_mode":if paginated {"paginated"} else {"legacy"},"history_base":null});
        let before = encode(&[
            ("session_meta", header.to_string()),
            ("event_msg", json!({"type":"task_started","turn_id":"old"}).to_string()),
            ("response_item", json!({"type":"message","role":"user","content":[{"type":"input_text","text":"historical"}]}).to_string()),
            ("event_msg", json!({"type":"task_complete","turn_id":"old","error":null}).to_string()),
        ], paginated, 0);
        let completion = if paginated {
            json!({"type":"item_completed","thread_id":THREAD,"turn_id":TURN,"item":{"type":"ContextCompaction","id":ITEM}})
        } else {
            json!({"type":"context_compacted"})
        };
        let suffix = vec![
            (
                "event_msg",
                json!({"type":"thread_settings_applied","thread_id":THREAD,"thread_settings":{}})
                    .to_string(),
            ),
            ("event_msg", event("task_started").to_string()),
            ("compacted", compact(PAYLOAD)),
            ("world_state", json!({"full":true,"state":{}}).to_string()),
            (
                "event_msg",
                json!({"type":"thread_settings_applied","thread_id":THREAD,"thread_settings":{}})
                    .to_string(),
            ),
            (
                "event_msg",
                json!({"type":"token_count","info":null}).to_string(),
            ),
            ("event_msg", completion.to_string()),
            ("event_msg", event("task_complete").to_string()),
            (
                "event_msg",
                json!({"type":"token_count","info":null}).to_string(),
            ),
        ];
        Self {
            before,
            suffix,
            paginated,
        }
    }
    fn after(&self) -> Vec<u8> {
        let mut bytes = self.before.clone();
        bytes.extend(encode(&self.suffix, self.paginated, 4));
        bytes
    }
    fn result(
        &self,
    ) -> ostk_gpt_cache::project::Result<ostk_gpt_cache::native_capture::CapturedWindow> {
        capture_bytes(&self.before, &self.after(), THREAD, TURN, ITEM)
    }
    fn reject(&self) {
        assert!(self.result().is_err());
    }
}

#[test]
fn fresh_paginated_checkpoint_preserves_exact_native_lexemes() {
    let f = Fixture::new(true);
    let capture = f.result().unwrap();
    assert_eq!(capture.payload_bytes, PAYLOAD.as_bytes());
    assert_eq!(capture.source_history_sha256, hash(&f.after()));
    assert_eq!(capture.last_checkpoint_index, 0);
    assert_eq!(capture.item_count, 2);
    let debug = format!("{capture:?}");
    assert!(!debug.contains("encrypted_content"));
    assert!(!debug.contains("agent-opaque"));
    assert!(!debug.contains("historical"));
}

#[test]
fn legacy_completion_marker_and_turn_aliases_work() {
    let mut f = Fixture::new(false);
    f.suffix[1].1 = event("turn_started").to_string();
    f.suffix[7].1 = event("turn_complete").to_string();
    assert_eq!(f.result().unwrap().payload_bytes, PAYLOAD.as_bytes());
    f.suffix[6].1 = json!({"type":"item_completed","thread_id":THREAD,"turn_id":TURN,"item":{"type":"ContextCompaction","id":ITEM}}).to_string();
    f.reject();
}

#[test]
fn unique_checkpoint_is_required_inside_the_new_turn() {
    for index in [1, 2, 6, 7] {
        let mut f = Fixture::new(true);
        f.suffix.remove(index);
        f.reject();
    }
    for index in [1, 2, 6, 7] {
        let mut f = Fixture::new(true);
        f.suffix.insert(index, f.suffix[index].clone());
        f.reject();
    }
    let mut f = Fixture::new(true);
    f.suffix.swap(1, 2);
    f.reject();
    let mut f = Fixture::new(true);
    f.suffix.swap(6, 7);
    f.reject();
}

#[test]
fn completion_identity_errors_abort_and_failure_are_rejected() {
    for (index, key) in [
        (1, "turn_id"),
        (6, "turn_id"),
        (6, "thread_id"),
        (7, "turn_id"),
    ] {
        let mut f = Fixture::new(true);
        let mut value: Value = serde_json::from_str(&f.suffix[index].1).unwrap();
        value[key] = json!("different");
        f.suffix[index].1 = value.to_string();
        f.reject();
    }
    for replacement in [
        json!({"type":"task_complete","turn_id":TURN,"error":{"message":"private error"}}),
        json!({"type":"turn_aborted","turn_id":TURN}),
        json!({"type":"task_started","turn_id":"another"}),
    ] {
        let mut f = Fixture::new(true);
        f.suffix[7].1 = replacement.to_string();
        f.reject();
    }
    let f = Fixture::new(true);
    assert!(
        capture_bytes(
            &f.before,
            &f.after(),
            THREAD,
            TURN,
            "21a08e8f-c4af-77a3-9643-7c474187c260"
        )
        .is_err()
    );
}

#[test]
fn native_agent_control_and_unknown_suffixes_are_never_dropped() {
    for kind in [
        "response_item",
        "inter_agent_communication",
        "inter_agent_communication_metadata",
        "retained_context",
        "realtime_item",
        "session_meta",
        "new_unknown_record",
    ] {
        for index in [2, 4, 8, 9] {
            let mut f = Fixture::new(true);
            f.suffix.insert(index, (kind, "{}".into()));
            f.reject();
        }
    }
    for event_type in [
        "error",
        "thread_rolled_back",
        "user_message",
        "agent_message",
        "thread_goal_updated",
        "hook_completed",
        "unknown",
    ] {
        let mut f = Fixture::new(true);
        f.suffix
            .push(("event_msg", json!({"type":event_type}).to_string()));
        f.reject();
    }
}

#[test]
fn only_installation_baseline_sidecars_are_allowed_after_checkpoint() {
    let mut f = Fixture::new(true);
    f.suffix[3].1 = json!({"full":false,"state":{}}).to_string();
    f.reject();
    for record in [
        ("world_state", json!({"full":true,"state":{}}).to_string()),
        ("turn_context", json!({"turn_id":TURN}).to_string()),
        (
            "event_msg",
            json!({"type":"thread_settings_applied","thread_id":THREAD}).to_string(),
        ),
    ] {
        let mut f = Fixture::new(true);
        f.suffix.push(record);
        f.reject();
    }
    let mut f = Fixture::new(true);
    f.suffix
        .insert(3, ("turn_context", json!({"turn_id":TURN}).to_string()));
    f.result().unwrap();
    f.suffix[3].1 = json!({"turn_id":"wrong"}).to_string();
    f.reject();
}

#[test]
fn host_metadata_is_excluded_and_processing_overrides_reject() {
    let mut f = Fixture::new(true);
    let native = r#"[{"type":"compaction","encrypted_content":"opaque"}]"#;
    f.suffix[2].1 = format!(
        r#"{{"message":"","replacement_history":{native},"window_number":2,"guardian_history":[{{"private":"guardian"}}],"retained_context":{{"private":"host-only"}},"mcp_resource_origins":{{"private":"account"}},"replacement_history_metadata":[{{"client_authored":false,"compaction_model_hash":"source-compat","user_input_order":1,"inherited_user_message":true}}]}}"#
    );
    assert_eq!(f.result().unwrap().payload_bytes, native.as_bytes());
    for metadata in [
        json!([{"fallback_token_limit_override":8192}]),
        json!([{"harness_authored_configuration":true}]),
        json!([{"unknown_flag":true}]),
        json!([{"client_authored":"true"}]),
        json!([]),
        json!([{}, {}]),
    ] {
        let mut f = Fixture::new(true);
        let mut value = json!({"message":"","replacement_history":[{"type":"compaction","encrypted_content":"opaque"}],"window_number":2});
        value["replacement_history_metadata"] = metadata;
        f.suffix[2].1 = value.to_string();
        f.reject();
    }
}

#[test]
fn source_identity_version_lineage_and_old_rollback_are_checked() {
    for (key, value) in [
        ("id", json!(ITEM)),
        ("cli_version", json!("0.155.0")),
        ("history_base", json!({"thread_id":ITEM})),
        ("history_mode", json!("unknown")),
    ] {
        let mut f = Fixture::new(true);
        let mut lines: Vec<Value> = std::str::from_utf8(&f.before)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        lines[0]["payload"][key] = value;
        f.before = lines
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>()
            .into_bytes();
        f.reject();
    }
    let mut f = Fixture::new(false);
    f.before.extend(encode(
        &[(
            "event_msg",
            json!({"type":"thread_rolled_back","num_turns":1}).to_string(),
        )],
        false,
        0,
    ));
    f.reject();
    let mut f = Fixture::new(false);
    f.before.extend(encode(
        &[(
            "event_msg",
            json!({"type":"task_started","turn_id":"active"}).to_string(),
        )],
        false,
        0,
    ));
    f.reject();
}

#[test]
fn byte_prefix_newline_duplicate_keys_and_ordinals_are_strict() {
    let f = Fixture::new(true);
    let mut after = f.after();
    after[2] = b'X';
    assert!(capture_bytes(&f.before, &after, THREAD, TURN, ITEM).is_err());
    let mut after = f.after();
    after.pop();
    assert!(capture_bytes(&f.before, &after, THREAD, TURN, ITEM).is_err());
    let mut f = Fixture::new(true);
    f.suffix[2].1 =
        r#"{"replacement_history":[],"replacement_history":[],"window_number":1}"#.into();
    f.reject();
    let mut f = Fixture::new(true);
    f.suffix[2].1 = r#"{"replacement_history":[{"type":"compaction","encrypted_content":"x","future":{"a":1,"\u0061":2}}],"window_number":1}"#.into();
    f.reject();
    let mut after = Fixture::new(true).after();
    let text = String::from_utf8(after)
        .unwrap()
        .replace("\"ordinal\":5", "\"ordinal\":99");
    after = text.into_bytes();
    let f = Fixture::new(true);
    assert!(capture_bytes(&f.before, &after, THREAD, TURN, ITEM).is_err());
}

#[test]
fn limits_and_no_native_checkpoint_reject_without_payload_diagnostics() {
    let mut f = Fixture::new(true);
    f.suffix[2].1 = compact(r#"[{"type":"message","role":"assistant","content":[]}]"#);
    f.reject();
    let mut f = Fixture::new(true);
    f.suffix[2].1 = compact(&format!("[{}{}{}]", "[".repeat(70), "0", "]".repeat(70)));
    f.reject();
    let f = Fixture::new(true);
    let after = vec![b'x'; MAX_ROLLOUT_BYTES + 1];
    assert!(capture_bytes(&f.before, &after, THREAD, TURN, ITEM).is_err());
    let mut f = Fixture::new(true);
    f.suffix[2].1 = "private invalid contents".into();
    assert!(!f.result().unwrap_err().to_string().contains("private"));
}

#[cfg(unix)]
mod files {
    use super::*;
    use ostk_gpt_cache::native_capture::{capture, read_snapshot};
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn read_only_snapshots_capture_same_identity_append() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("rollout.jsonl");
        let f = Fixture::new(true);
        fs::write(&path, &f.before).unwrap();
        let before = read_snapshot(&path, THREAD).unwrap();
        fs::write(&path, f.after()).unwrap();
        let after = read_snapshot(&path, THREAD).unwrap();
        assert_eq!(
            capture(&before, &after, THREAD, TURN, ITEM)
                .unwrap()
                .payload_bytes,
            PAYLOAD.as_bytes()
        );
        assert!(!format!("{after:?}").contains("opaque"));
    }

    #[test]
    fn symlink_hardlink_directory_and_replacement_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("rollout.jsonl");
        let f = Fixture::new(true);
        fs::write(&path, &f.before).unwrap();
        let before = read_snapshot(&path, THREAD).unwrap();
        let link = root.join("alias");
        symlink(&path, &link).unwrap();
        assert!(read_snapshot(&link, THREAD).is_err());
        let dirlink = root.join("parent-alias");
        symlink(&root, &dirlink).unwrap();
        assert!(read_snapshot(&dirlink.join("rollout.jsonl"), THREAD).is_err());
        assert!(read_snapshot(&root, THREAD).is_err());
        let hard = root.join("hard");
        fs::hard_link(&path, &hard).unwrap();
        assert!(read_snapshot(&path, THREAD).is_err());
        fs::remove_file(hard).unwrap();
        let replacement = root.join("replacement");
        fs::write(&replacement, f.after()).unwrap();
        fs::rename(&replacement, &path).unwrap();
        let after = read_snapshot(&path, THREAD).unwrap();
        assert!(capture(&before, &after, THREAD, TURN, ITEM).is_err());
    }

    #[test]
    fn fifo_returns_without_blocking_and_oversized_file_rejects() {
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("fifo");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_snapshot(&path, THREAD).is_err());
        let path = root.join("large");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_ROLLOUT_BYTES as u64 + 1).unwrap();
        assert!(read_snapshot(&path, THREAD).is_err());
    }
}
