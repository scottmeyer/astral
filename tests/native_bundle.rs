use ostk_gpt_cache::hash;
use ostk_gpt_cache::native_bundle::{
    MAX_ITEMS, MAX_JSON_DEPTH, MAX_JSON_NODES, MAX_MANIFEST_BYTES, MAX_PARENTS, MAX_PAYLOAD_BYTES,
    NativeBundle,
};
use serde_json::{Value, json, value::RawValue};

const CHECKPOINT: &str = r#"{"type":"compaction","encrypted_content":"opaque-synthetic"}"#;

fn manifest(payload: &[u8], last: usize) -> Value {
    let items: Vec<&RawValue> = serde_json::from_slice(payload).unwrap();
    json!({
        "schema_version":1,"format":"astral-codex-native",
        "payload":{"file":"window.json","sha256":hash(payload),"bytes":payload.len(),"item_count":items.len()},
        "compatibility":{"runtime":"codex","runtime_version":"0.154.0","protocol":"openai-responses-lite",
            "provider":"openai","model":"gpt-6-astra","requires_tool_rebinding":true,"identity_scope":"same-account"},
        "source":{"project_id":"fixture","revision":null,"dirty":false,"selection_sha256":"a".repeat(64),"history_sha256":"b".repeat(64)},
        "capture":{"boundary":"completed-turn","history_complete":true,"last_checkpoint_index":last},
        "parents":[]
    })
}

fn validate(payload: &str, last: usize) -> ostk_gpt_cache::project::Result<NativeBundle> {
    let manifest = serde_json::to_vec(&manifest(payload.as_bytes(), last)).unwrap();
    NativeBundle::validate(&manifest, payload.as_bytes())
}

fn window(items: &[&str]) -> String {
    format!("[{}]", items.join(","))
}

#[test]
fn exact_native_bytes_unknown_fields_and_numeric_spelling_survive() {
    let checkpoint = r#"{ "type":"compaction", "encrypted_content":"\u006fpAqUE==", "future":{"n":1234567890123456789012345678901234567890,"large":1e99999,"minus":-0.000e+7}, "metadata":{"executed_tool_calls":["old"]} }"#;
    let payload = format!(
        " \n[{checkpoint},\n{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"done\"}}]}}]\n"
    );
    let manifest_bytes = serde_json::to_vec_pretty(&manifest(payload.as_bytes(), 0)).unwrap();
    let bundle = NativeBundle::validate(&manifest_bytes, payload.as_bytes()).unwrap();
    assert_eq!(bundle.payload_bytes(), payload.as_bytes());
    assert_eq!(bundle.manifest_bytes(), manifest_bytes);
    assert_eq!(bundle.raw_items()[0].get(), checkpoint);
    assert_eq!(bundle.summary().manifest_sha256, hash(&manifest_bytes));
    assert_eq!(bundle.summary().checkpoint_count, 1);
    let debug = format!("{bundle:?}");
    assert!(!debug.contains("opAqUE"));
    assert!(!debug.contains("encrypted_content"));
    assert!(!debug.contains("done"));
    assert!(
        !serde_json::to_string(&bundle.summary())
            .unwrap()
            .contains("opAqUE")
    );
    // Manifest identity follows exact bytes, not a canonical reserialization.
    let compact = serde_json::to_vec(&bundle.manifest()).unwrap();
    assert_ne!(hash(&compact), bundle.summary().manifest_sha256);
}

#[test]
fn instructions_are_reported_as_source_metadata_but_capabilities_are_rejected() {
    let payload = window(&[
        r#"{"type":"message","role":"system","content":[{"type":"input_text","text":"historical system"}]}"#,
        CHECKPOINT,
        r#"{"type":"message","role":"developer","content":[{"type":"input_text","text":"historical developer"}]}"#,
    ]);
    let summary = validate(&payload, 1).unwrap().summary();
    assert_eq!(summary.source_instruction_roles, ["developer", "system"]);
    let text = serde_json::to_string(&summary).unwrap();
    assert!(!text.contains("historical developer"));
    for kind in [
        "additional_tools",
        "item_reference",
        "configuration_update",
        "context_compaction",
        "compaction_trigger",
        "local_shell_call",
        "compaction_summary",
        "unknown_future_kind",
    ] {
        let item = format!(r#"{{"type":"{kind}","id":"inert"}}"#);
        assert_eq!(
            validate(&window(&[CHECKPOINT, &item]), 0).unwrap_err().code,
            "NATIVE_UNSUPPORTED_ITEM",
            "{kind}"
        );
    }
}

#[test]
fn all_manifest_fields_are_required_and_unknown_fields_rejected() {
    let payload = window(&[CHECKPOINT]);
    let baseline = manifest(payload.as_bytes(), 0);
    for group in [
        None,
        Some("payload"),
        Some("compatibility"),
        Some("source"),
        Some("capture"),
    ] {
        let fields: Vec<String> = group
            .map_or(&baseline, |key| &baseline[key])
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        for key in fields {
            let mut changed = baseline.clone();
            let target = match group {
                None => &mut changed,
                Some(group) => &mut changed[group],
            };
            target.as_object_mut().unwrap().remove(&key);
            let err =
                NativeBundle::validate(&serde_json::to_vec(&changed).unwrap(), payload.as_bytes())
                    .unwrap_err();
            assert_eq!(
                err.code, "NATIVE_INVALID_MANIFEST",
                "missing {group:?}/{key}"
            );
        }
        let mut changed = baseline.clone();
        let target = match group {
            None => &mut changed,
            Some(group) => &mut changed[group],
        };
        target["unknown"] = json!(true);
        assert_eq!(
            NativeBundle::validate(&serde_json::to_vec(&changed).unwrap(), payload.as_bytes())
                .unwrap_err()
                .code,
            "NATIVE_INVALID_MANIFEST"
        );
    }
}

#[test]
fn compatibility_is_explicit_and_narrow() {
    let payload = window(&[CHECKPOINT]);
    for (key, value) in [
        ("runtime", json!("another")),
        ("runtime_version", json!("0.155.0")),
        ("protocol", json!("responses")),
        ("provider", json!("custom")),
        ("model", json!("other")),
        ("requires_tool_rebinding", json!(false)),
        ("identity_scope", json!("any-account")),
    ] {
        let mut m = manifest(payload.as_bytes(), 0);
        m["compatibility"][key] = value;
        assert_eq!(
            NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), payload.as_bytes())
                .unwrap_err()
                .code,
            "NATIVE_UNSUPPORTED_COMPATIBILITY"
        );
    }
    let mut m = manifest(payload.as_bytes(), 0);
    m["schema_version"] = json!(2);
    assert_eq!(
        NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), payload.as_bytes())
            .unwrap_err()
            .code,
        "NATIVE_UNSUPPORTED_FORMAT"
    );
}

#[test]
fn payload_tampering_and_truncation_never_validate() {
    let payload = window(&[CHECKPOINT]);
    let m = serde_json::to_vec(&manifest(payload.as_bytes(), 0)).unwrap();
    let tampered = payload.replace("opaque", "OPAQUE");
    assert_eq!(
        NativeBundle::validate(&m, tampered.as_bytes())
            .unwrap_err()
            .code,
        "NATIVE_INTEGRITY"
    );
    assert_eq!(
        NativeBundle::validate(&m, &payload.as_bytes()[..payload.len() - 1])
            .unwrap_err()
            .code,
        "NATIVE_INTEGRITY"
    );
    let mut m = manifest(payload.as_bytes(), 0);
    m["payload"]["item_count"] = json!(2);
    assert_eq!(
        NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), payload.as_bytes())
            .unwrap_err()
            .code,
        "NATIVE_INTEGRITY"
    );
    m["payload"]["item_count"] = json!(1);
    let truncated = &payload[..payload.len() - 1];
    m["payload"]["bytes"] = json!(truncated.len());
    m["payload"]["sha256"] = json!(hash(truncated.as_bytes()));
    assert_eq!(
        NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), truncated.as_bytes())
            .unwrap_err()
            .code,
        "NATIVE_INVALID_JSON"
    );
}

#[test]
fn duplicate_keys_are_rejected_at_every_depth_including_escape_aliases() {
    for item in [
        r#"{"type":"compaction","type":"compaction","encrypted_content":"x"}"#,
        r#"{"type":"compaction","encrypted_content":"x","unknown":{"k":1,"\u006b":2}}"#,
        r#"{"type":"compaction","encrypted_content":"x","unknown":[{"a":[{"z":null,"z":false}]}]}"#,
    ] {
        let payload = window(&[item]);
        assert_eq!(
            validate(&payload, 0).unwrap_err().code,
            "NATIVE_DUPLICATE_KEY"
        );
    }
    let payload = window(&[CHECKPOINT]);
    let manifest = serde_json::to_string(&manifest(payload.as_bytes(), 0)).unwrap();
    let duplicate = manifest.replacen("\"dirty\":false", "\"dirty\":false,\"dirty\":false", 1);
    assert_eq!(
        NativeBundle::validate(duplicate.as_bytes(), payload.as_bytes())
            .unwrap_err()
            .code,
        "NATIVE_DUPLICATE_KEY"
    );
}

const FUNCTION: &str = r#"{"type":"function_call","name":"pwd","call_id":"call-1","arguments":"{\"literal\": 12345678901234567890}"}"#;
const FUNCTION_OUTPUT: &str = r#"{"type":"function_call_output","call_id":"call-1","output":"ok"}"#;
const CUSTOM: &str = r#"{"type":"custom_tool_call","name":"terminal","call_id":"call-2","input":"pwd","status":"in_progress"}"#;
const CUSTOM_OUTPUT: &str = r#"{"type":"custom_tool_call_output","call_id":"call-2","output":[{"type":"input_text","text":"ok"},{"type":"encrypted_content","encrypted_content":"opaque"}]}"#;
const SEARCH: &str = r#"{"type":"tool_search_call","execution":"client","call_id":"call-3","arguments":{"query":"terminal","limit":2}}"#;
const SEARCH_OUTPUT: &str = r#"{"type":"tool_search_output","execution":"client","call_id":"call-3","status":"completed","tools":[]}"#;

#[test]
fn paired_function_custom_and_client_search_windows_validate_without_rewriting_calls() {
    let payload = window(&[
        FUNCTION,
        CUSTOM,
        SEARCH,
        CUSTOM_OUTPUT,
        FUNCTION_OUTPUT,
        SEARCH_OUTPUT,
        CHECKPOINT,
    ]);
    let bundle = validate(&payload, 6).unwrap();
    assert_eq!(bundle.raw_items()[0].get(), FUNCTION);
    assert_eq!(bundle.raw_items()[1].get(), CUSTOM);
    assert_eq!(bundle.raw_items()[3].get(), CUSTOM_OUTPUT);
}

#[test]
fn pending_calls_cannot_cross_checkpoints_or_end_of_window() {
    for (call, output) in [
        (FUNCTION, FUNCTION_OUTPUT),
        (CUSTOM, CUSTOM_OUTPUT),
        (SEARCH, SEARCH_OUTPUT),
    ] {
        assert_eq!(
            validate(&window(&[CHECKPOINT, call]), 0).unwrap_err().code,
            "NATIVE_PENDING_CALL"
        );
        assert_eq!(
            validate(&window(&[call, CHECKPOINT, output]), 1)
                .unwrap_err()
                .code,
            "NATIVE_PENDING_CALL"
        );
    }
}

#[test]
fn duplicate_ids_orphan_and_mismatched_outputs_are_rejected() {
    for payload in [
        window(&[CHECKPOINT, FUNCTION_OUTPUT]),
        window(&[CHECKPOINT, CUSTOM_OUTPUT]),
        window(&[CHECKPOINT, SEARCH_OUTPUT]),
        window(&[
            CHECKPOINT,
            FUNCTION,
            &CUSTOM_OUTPUT.replace("call-2", "call-1"),
        ]),
    ] {
        assert_eq!(
            validate(&payload, 0).unwrap_err().code,
            "NATIVE_ORPHAN_OUTPUT"
        );
    }
    for payload in [
        window(&[CHECKPOINT, FUNCTION, FUNCTION]),
        window(&[CHECKPOINT, FUNCTION, FUNCTION_OUTPUT, FUNCTION]),
        window(&[CHECKPOINT, FUNCTION, &CUSTOM.replace("call-2", "call-1")]),
    ] {
        assert_eq!(
            validate(&payload, 0).unwrap_err().code,
            "NATIVE_DUPLICATE_CALL"
        );
    }
    let unnamed = r#"{"type":"function_call_output","name":"message","output":"unpaired"}"#;
    assert_eq!(
        validate(&window(&[CHECKPOINT, unnamed]), 0)
            .unwrap_err()
            .code,
        "NATIVE_INVALID_ITEM"
    );
}

#[test]
fn server_search_and_completed_server_work_have_no_invented_client_pairing() {
    let search_call = r#"{"type":"tool_search_call","execution":"server","call_id":null,"status":"completed","arguments":{"query":"x"}}"#;
    let search_output = r#"{"type":"tool_search_output","execution":"server","call_id":null,"status":"completed","tools":[{"opaque_schema":{"large":1e999}}]}"#;
    let web = r#"{"type":"web_search_call","status":"completed","action":{"type":"search","queries":["a","b"]}}"#;
    let image = r#"{"type":"image_generation_call","status":"completed","result":"opaque-image","revised_prompt":"source prompt"}"#;
    validate(
        &window(&[search_call, CHECKPOINT, search_output, web, image]),
        1,
    )
    .unwrap();
    for item in [search_call, search_output, web, image] {
        let pending = item.replace("completed", "in_progress");
        assert_eq!(
            validate(&window(&[CHECKPOINT, &pending]), 0)
                .unwrap_err()
                .code,
            "NATIVE_INVALID_ITEM"
        );
    }
}

#[test]
fn historical_agent_messages_are_inert_and_preserved() {
    let message = r#"{"type":"agent_message","author":"old-agent","recipient":"old-coordinator","content":[{"type":"input_text","text":"PRIVATE_AGENT_TEXT"},{"type":"encrypted_content","encrypted_content":"PRIVATE_AGENT_OPAQUE"}]}"#;
    let bundle = validate(&window(&[CHECKPOINT, message]), 0).unwrap();
    assert_eq!(bundle.raw_items()[1].get(), message);
    assert_eq!(bundle.summary().agent_message_count, 1);
    assert!(bundle.summary().source_instruction_roles.is_empty());
    assert!(!format!("{bundle:?}").contains("PRIVATE_AGENT"));
    for malformed in [
        message.replace("\"old-agent\"", "false"),
        message.replace("\"old-coordinator\"", "[]"),
        message.replace("\"input_text\"", "\"input_image\""),
    ] {
        assert_eq!(
            validate(&window(&[CHECKPOINT, &malformed]), 0)
                .unwrap_err()
                .code,
            "NATIVE_INVALID_ITEM"
        );
    }
}

#[test]
fn known_optional_metadata_types_are_checked_without_discarding_extensions() {
    let base = r#"{"type":"compaction","encrypted_content":"x","internal_chat_message_metadata_passthrough":REPLACE}"#;
    for wrong in [
        "17",
        "\"x\"",
        "[]",
        r#"{"turn_id":{}}"#,
        r#"{"create_time":"17"}"#,
        r#"{"create_time":1e9999}"#,
    ] {
        let item = base.replace("REPLACE", wrong);
        assert_eq!(
            validate(&window(&[&item]), 0).unwrap_err().code,
            "NATIVE_INVALID_ITEM"
        );
    }
    let item = base.replace("REPLACE", r#"{"turn_id":"historic","create_time":17.005,"unknown_number":1e9999,"cell_id":{"inert":true}}"#);
    let bundle = validate(&window(&[&item]), 0).unwrap();
    assert_eq!(bundle.raw_items()[0].get(), item);
    for (call, output, field) in [
        (FUNCTION, FUNCTION_OUTPUT, "name"),
        (FUNCTION, FUNCTION_OUTPUT, "namespace"),
        (CUSTOM, CUSTOM_OUTPUT, "name"),
    ] {
        let wrong = output.replacen('{', &format!("{{\"{field}\":[],"), 1);
        assert_eq!(
            validate(&window(&[CHECKPOINT, call, &wrong]), 0)
                .unwrap_err()
                .code,
            "NATIVE_INVALID_ITEM"
        );
    }
    let audio = r#"{"type":"message","role":"user","content":[{"type":"input_audio","audio_url":"data:audio/wav;base64,YQ==","detail":{"future":true}}]}"#;
    validate(&window(&[CHECKPOINT, audio]), 0).unwrap();
}

#[test]
fn malformed_utf8_and_non_array_payloads_are_rejected_even_with_matching_hashes() {
    let baseline = window(&[CHECKPOINT]);
    for payload in [
        &b"[\xff]"[..],
        &b"{\"type\":\"compaction\",\"encrypted_content\":\"x\"}"[..],
        &b"[] true"[..],
    ] {
        let mut m = manifest(baseline.as_bytes(), 0);
        m["payload"]["bytes"] = json!(payload.len());
        m["payload"]["sha256"] = json!(hash(payload));
        assert!(NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), payload).is_err());
    }
}

#[test]
fn native_checkpoint_is_required_and_only_compaction_counts() {
    let reasoning = r#"{"type":"reasoning","summary":[],"encrypted_content":"ordinary-reasoning"}"#;
    assert_eq!(
        validate(&window(&[reasoning]), 0).unwrap_err().code,
        "NATIVE_BOUNDARY"
    );
    assert_eq!(
        validate(&window(&[CHECKPOINT, reasoning, CHECKPOINT]), 0)
            .unwrap_err()
            .code,
        "NATIVE_BOUNDARY"
    );
    assert_eq!(
        validate(&window(&[CHECKPOINT, reasoning, CHECKPOINT]), 2)
            .unwrap()
            .summary()
            .checkpoint_count,
        2
    );
    let empty = r#"{"type":"compaction","encrypted_content":""}"#;
    assert_eq!(
        validate(&window(&[empty]), 0).unwrap_err().code,
        "NATIVE_INVALID_ITEM"
    );
    let payload = window(&[CHECKPOINT]);
    for (field, value) in [
        ("history_complete", json!(false)),
        ("boundary", json!("in-progress")),
    ] {
        let mut m = manifest(payload.as_bytes(), 0);
        m["capture"][field] = value;
        assert_eq!(
            NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), payload.as_bytes())
                .unwrap_err()
                .code,
            "NATIVE_BOUNDARY"
        );
    }
}

#[test]
fn media_dependencies_must_be_self_contained_and_never_fetched() {
    for (kind, key) in [("input_image", "image_url"), ("input_audio", "audio_url")] {
        for url in [
            "https://example.test/private",
            "file:///private/tmp/secret",
            "/tmp/image",
        ] {
            let item = format!(
                r#"{{"type":"message","role":"user","content":[{{"type":"{kind}","{key}":"{url}"}}]}}"#
            );
            assert_eq!(
                validate(&window(&[CHECKPOINT, &item]), 0).unwrap_err().code,
                "NATIVE_EXTERNAL_REFERENCE"
            );
        }
        let item = format!(
            r#"{{"type":"message","role":"user","content":[{{"type":"{kind}","{key}":"data:application/octet-stream;base64,YQ=="}}]}}"#
        );
        validate(&window(&[CHECKPOINT, &item]), 0).unwrap();
    }
}

#[test]
fn byte_item_node_and_depth_bounds_are_enforced() {
    let payload = window(&[CHECKPOINT]);
    let m = serde_json::to_vec(&manifest(payload.as_bytes(), 0)).unwrap();
    assert_eq!(
        NativeBundle::validate(&vec![b' '; MAX_MANIFEST_BYTES + 1], payload.as_bytes())
            .unwrap_err()
            .code,
        "NATIVE_LIMIT"
    );
    assert_eq!(
        NativeBundle::validate(&m, &vec![b' '; MAX_PAYLOAD_BYTES + 1])
            .unwrap_err()
            .code,
        "NATIVE_LIMIT"
    );
    let many = window(&vec![CHECKPOINT; MAX_ITEMS + 1]);
    assert_eq!(
        validate(&many, MAX_ITEMS).unwrap_err().code,
        "NATIVE_INVALID_MANIFEST"
    );
    let mut m = manifest(many.as_bytes(), MAX_ITEMS);
    m["payload"]["item_count"] = json!(1);
    assert_eq!(
        NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), many.as_bytes())
            .unwrap_err()
            .code,
        "NATIVE_LIMIT"
    );
    let deep = format!(
        "{}0{}",
        "[".repeat(MAX_JSON_DEPTH + 1),
        "]".repeat(MAX_JSON_DEPTH + 1)
    );
    let item = format!(r#"{{"type":"compaction","encrypted_content":"x","future":{deep}}}"#);
    assert_eq!(
        validate(&window(&[&item]), 0).unwrap_err().code,
        "NATIVE_LIMIT"
    );
    let nodes = format!("[{}]", vec!["0"; MAX_JSON_NODES].join(","));
    let item = format!(r#"{{"type":"compaction","encrypted_content":"x","future":{nodes}}}"#);
    assert_eq!(
        validate(&window(&[&item]), 0).unwrap_err().code,
        "NATIVE_LIMIT"
    );
    let wide_object = (0..=MAX_JSON_NODES)
        .map(|n| format!("\"k{n}\":0"))
        .collect::<Vec<_>>()
        .join(",");
    let item =
        format!(r#"{{"type":"compaction","encrypted_content":"x","future":{{{wide_object}}}}}"#);
    assert_eq!(
        validate(&window(&[&item]), 0).unwrap_err().code,
        "NATIVE_LIMIT"
    );
}

#[test]
fn source_identity_revision_and_parent_digests_are_validated() {
    let payload = window(&[CHECKPOINT]);
    for (field, value) in [
        ("project_id", json!("../escape")),
        ("revision", json!("A".repeat(40))),
        ("history_sha256", json!("bad")),
    ] {
        let mut m = manifest(payload.as_bytes(), 0);
        m["source"][field] = value;
        assert_eq!(
            NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), payload.as_bytes())
                .unwrap_err()
                .code,
            "NATIVE_INVALID_MANIFEST"
        );
    }
    for parents in [
        vec!["x".repeat(64)],
        vec!["a".repeat(64); 2],
        (0..MAX_PARENTS + 1).map(|n| format!("{n:064x}")).collect(),
    ] {
        let mut m = manifest(payload.as_bytes(), 0);
        m["parents"] = json!(parents);
        assert_eq!(
            NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), payload.as_bytes())
                .unwrap_err()
                .code,
            "NATIVE_INVALID_PARENTS"
        );
    }
    let mut m = manifest(payload.as_bytes(), 0);
    m["source"]["revision"] = json!("c".repeat(64));
    m["parents"] = json!(["d".repeat(64), "e".repeat(64)]);
    NativeBundle::validate(&serde_json::to_vec(&m).unwrap(), payload.as_bytes()).unwrap();
}

#[test]
fn errors_never_echo_private_payload_or_arbitrary_metadata() {
    let item =
        r#"{"type":"PRIVATE_SECRET_IN_KIND","encrypted_content":"PRIVATE_SECRET_IN_PAYLOAD"}"#;
    let error = validate(&window(&[item]), 0).unwrap_err();
    let serialized = serde_json::to_string(&error).unwrap();
    assert!(!serialized.contains("PRIVATE_SECRET"));
    let malformed = b"{\"PRIVATE_SECRET\":";
    let error = NativeBundle::validate(malformed, b"[]").unwrap_err();
    assert!(!error.to_string().contains("PRIVATE_SECRET"));
}
