use astral::{archive::Archive, config::Config, providers::Wire, tool_history::reduce};
use clap::Parser;
use serde_json::{Value, json};

fn config() -> Config {
    Config::parse_from([
        "test",
        "--keep-recent-turns",
        "1",
        "--tool-result-bytes",
        "2048",
        "--tool-preview-bytes",
        "128",
    ])
}

fn body(wire: Wire, payload: Value) -> Value {
    match wire {
        Wire::Responses => json!({"model":"any","tools":[{"name":"read"}],"input":[
            {"role":"user","content":"inspect"},
            {"type":"reasoning","encrypted_content":"untouched","summary":[]},
            {"type":"function_call","call_id":"c1","name":"read","arguments":"{}"},
            {"type":"function_call_output","call_id":"c1","output":payload},
            {"role":"assistant","phase":"final_answer","content":"found"},
            {"role":"user","content":"next"}
        ]}),
        Wire::Messages => {
            json!({"model":"any","system":[{"type":"text","text":"system","cache_control":{"type":"ephemeral"}}],"messages":[
                {"role":"user","content":"inspect"},
                {"role":"assistant","content":[{"type":"thinking","thinking":"signed","signature":"unchanged"},{"type":"tool_use","id":"c1","name":"read","input":{}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"c1","content":payload,"cache_control":{"type":"ephemeral"}}]},
                {"role":"assistant","content":"found"},{"role":"user","content":"next"}
            ]})
        }
        Wire::Chat => json!({"model":"any","messages":[
            {"role":"system","content":"system"},{"role":"user","content":"inspect"},
            {"role":"assistant","tool_calls":[{"id":"c1","type":"function","function":{"name":"read","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"c1","content":payload},
            {"role":"assistant","content":"found"},{"role":"user","content":"next"}
        ]}),
    }
}
fn path(wire: Wire) -> &'static str {
    match wire {
        Wire::Responses => "/input/3/output",
        Wire::Messages => "/messages/2/content/0/content",
        Wire::Chat => "/messages/3/content",
    }
}
fn handle(value: &Value) -> &str {
    value
        .as_str()
        .unwrap()
        .split("handle=")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap()
}

#[test]
fn three_protocols_preserve_everything_except_old_text_and_exact_recall_survives_restart() {
    for wire in [Wire::Responses, Wire::Messages, Wire::Chat] {
        let root = tempfile::tempdir().unwrap();
        let archive = Archive::open(root.path(), 1_000_000).unwrap();
        let text = format!(
            "{}SECRET_IN_MIDDLE{}",
            "héllo ".repeat(600),
            "世界".repeat(600)
        );
        let original = body(wire, json!(text));
        let mut reduced = original.clone();
        let report = reduce(&mut reduced, wire, &config(), &archive, &"a".repeat(64));
        assert_eq!(report.archived, 1);
        assert!(report.bytes_saved > 4000);
        let replacement = reduced.pointer(path(wire)).unwrap().clone();
        assert!(!replacement.as_str().unwrap().contains("SECRET_IN_MIDDLE"));
        *reduced.pointer_mut(path(wire)).unwrap() = original.pointer(path(wire)).unwrap().clone();
        assert_eq!(reduced, original);
        let token = handle(&replacement).to_owned();
        let mut second = original.clone();
        reduce(&mut second, wire, &config(), &archive, &"a".repeat(64));
        assert_eq!(second.pointer(path(wire)).unwrap(), &replacement);
        drop(archive);
        let archive = Archive::open(root.path(), 1_000_000).unwrap();
        assert_eq!(archive.recall(&token, 0, 16_384).unwrap()["data"], text);
        let search = archive.search(&token, "SECRET_IN_MIDDLE", 0).unwrap();
        let offset = search["matches"][0]["offset"].as_u64().unwrap() as usize;
        assert_eq!(
            archive.recall(&token, offset, 16).unwrap()["data"],
            "SECRET_IN_MIDDLE"
        );
        assert!(archive.recall("../key", 0, 5).is_err());
    }
}

#[test]
fn scope_and_store_identity_isolate_handles_and_quota_fails_open() {
    let root = tempfile::tempdir().unwrap();
    let archive = Archive::open(root.path(), 100_000).unwrap();
    let a = archive.save(&"a".repeat(64), "utf8", "private").unwrap();
    let b = archive.save(&"b".repeat(64), "utf8", "private").unwrap();
    assert_ne!(a, b);
    let other = tempfile::tempdir().unwrap();
    let other = Archive::open(other.path(), 100_000).unwrap();
    assert_ne!(a, other.save(&"a".repeat(64), "utf8", "private").unwrap());
    assert!(other.recall(&a, 0, 10).is_err());
    let small = tempfile::tempdir().unwrap();
    let small = Archive::open(small.path(), 1).unwrap();
    let mut request = body(Wire::Responses, json!("data".repeat(2000)));
    let original = request.clone();
    let report = reduce(
        &mut request,
        Wire::Responses,
        &config(),
        &small,
        &"a".repeat(64),
    );
    assert_eq!(report.unavailable, 1);
    assert_eq!(request, original);
}

#[test]
fn recent_results_errors_images_cache_annotations_and_owned_context_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let archive = Archive::open(root.path(), 1_000_000).unwrap();
    for wire in [Wire::Responses, Wire::Messages, Wire::Chat] {
        let mut variants = vec![body(wire, json!("x".repeat(5000)))];
        let field = if wire == Wire::Responses {
            "input"
        } else {
            "messages"
        };
        variants[0][field].as_array_mut().unwrap().pop();
        variants.push(body(
            wire,
            json!(json!({"exit_code":1,"output":"x".repeat(5000)}).to_string()),
        ));
        variants.push(body(wire,json!([{"type":"text","text":"x".repeat(5000)},{"type":"image","source":{"type":"base64","data":"unchanged"}}])));
        variants.push(body(
            wire,
            json!([{"type":"text","text":"x".repeat(5000),"cache_control":{"type":"ephemeral"}}]),
        ));
        let mut owned = body(wire, json!("x".repeat(5000)));
        owned["context_management"] = json!({"edits":[]});
        variants.push(owned);
        for mut request in variants {
            let original = request.clone();
            assert_eq!(
                reduce(&mut request, wire, &config(), &archive, &"a".repeat(64)).archived,
                0
            );
            assert_eq!(request, original);
        }
    }
    let mut request = body(Wire::Messages, json!("error".repeat(1000)));
    request["messages"][2]["content"][0]["is_error"] = json!(true);
    assert_eq!(
        reduce(
            &mut request,
            Wire::Messages,
            &config(),
            &archive,
            &"a".repeat(64)
        )
        .archived,
        0
    );
}

#[test]
fn pending_duplicate_and_orphan_tool_calls_do_not_get_rewritten() {
    let root = tempfile::tempdir().unwrap();
    let archive = Archive::open(root.path(), 1_000_000).unwrap();
    for kind in 0..3 {
        let mut request = body(Wire::Responses, json!("x".repeat(5000)));
        match kind {
            0 => request["input"].as_array_mut().unwrap().push(
                json!({"type":"function_call","name":"read","call_id":"pending","arguments":"{}"}),
            ),
            1 => {
                let duplicate = request["input"][2].clone();
                request["input"]
                    .as_array_mut()
                    .unwrap()
                    .insert(3, duplicate);
            }
            _ => request["input"][3]["call_id"] = json!("orphan"),
        }
        let original = request.clone();
        let report = reduce(
            &mut request,
            Wire::Responses,
            &config(),
            &archive,
            &"a".repeat(64),
        );
        assert!(report.skipped.is_some());
        assert_eq!(request, original);
    }
}

#[test]
fn archive_detects_corruption_and_exact_json_arrays_are_recoverable() {
    let root = tempfile::tempdir().unwrap();
    let archive = Archive::open(root.path(), 100_000).unwrap();
    let payload = json!([{"type":"text","text":"x".repeat(4000)},{"type":"text","text":"tail"}]);
    let mut request = body(Wire::Messages, payload.clone());
    assert_eq!(
        reduce(
            &mut request,
            Wire::Messages,
            &config(),
            &archive,
            &"a".repeat(64)
        )
        .archived,
        1
    );
    let token = handle(request.pointer(path(Wire::Messages)).unwrap());
    let saved = archive.recall(token, 0, 16384).unwrap();
    assert_eq!(saved["format"], "json");
    assert_eq!(
        serde_json::from_str::<Value>(saved["data"].as_str().unwrap()).unwrap(),
        payload
    );
    std::fs::write(root.path().join(format!("{token}.json")), b"{}").unwrap();
    assert!(archive.recall(token, 0, 100).is_err());
}

#[test]
fn codex_responses_input_text_blocks_archive_and_recover_exactly() {
    let root = tempfile::tempdir().unwrap();
    let archive = Archive::open(root.path(), 1_000_000).unwrap();
    let config = Config::parse_from(["test"]);
    let text = format!(
        "{}hidden-marker{}",
        "padding ".repeat(2200),
        "tail ".repeat(2200)
    );
    // Codex 0.159.2 code-mode output observed during live HTTP verification.
    let payload = json!([
        {"type":"input_text","text":"Script completed\nWall time 0.0 seconds\nOutput:\n"},
        {"type":"input_text","text":json!({"content":[{"type":"text","text":text}],"isError":false}).to_string()}
    ]);
    let mut request = body(Wire::Responses, payload.clone());
    request["input"][2]["type"] = json!("custom_tool_call");
    request["input"][3]["type"] = json!("custom_tool_call_output");
    request["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"user","content":"recall"}));
    let original = request.clone();
    let report = reduce(
        &mut request,
        Wire::Responses,
        &config,
        &archive,
        &"a".repeat(64),
    );
    assert_eq!(report.archived, 1);
    assert!(report.bytes_saved > 16_384);
    let replacement = request.pointer(path(Wire::Responses)).unwrap().clone();
    assert!(!replacement.as_str().unwrap().contains("hidden-marker"));
    let token = handle(&replacement).to_owned();
    let mut restored = Vec::new();
    let mut offset = 0;
    loop {
        let page = archive.recall(&token, offset, 4096).unwrap();
        restored.extend_from_slice(page["data"].as_str().unwrap().as_bytes());
        if page["eof"] == true {
            break;
        }
        offset = page["next_offset"].as_u64().unwrap() as usize;
    }
    assert_eq!(serde_json::from_slice::<Value>(&restored).unwrap(), payload);
    *request.pointer_mut(path(Wire::Responses)).unwrap() = payload;
    assert_eq!(request, original);
}

#[test]
fn responses_input_text_support_preserves_annotations_images_and_other_protocols() {
    let root = tempfile::tempdir().unwrap();
    let archive = Archive::open(root.path(), 1_000_000).unwrap();
    let text = "x".repeat(20_000);
    for (wire, payload) in [
        (
            Wire::Responses,
            json!([{"type":"input_text","text":text,"annotations":[]}]),
        ),
        (
            Wire::Responses,
            json!([{"type":"input_text","text":text},{"type":"input_image","image_url":"data:image/png;base64,AAAA"}]),
        ),
        (Wire::Messages, json!([{"type":"input_text","text":text}])),
        (Wire::Chat, json!([{"type":"input_text","text":text}])),
    ] {
        let mut request = body(wire, payload);
        let original = request.clone();
        let report = reduce(&mut request, wire, &config(), &archive, &"a".repeat(64));
        assert_eq!(report.archived, 0);
        assert_eq!(request, original);
    }
}

#[test]
fn long_turns_archive_consumed_results_but_keep_the_entire_latest_parallel_batch() {
    let root = tempfile::tempdir().unwrap();
    let archive = Archive::open(root.path(), 1_000_000).unwrap();
    for wire in [Wire::Responses, Wire::Messages] {
        let mut items = vec![json!({"role":"user","content":"a long task"})];
        for round in 0..6 {
            if wire == Wire::Responses {
                items.push(json!({"type":"function_call","name":"read","call_id":format!("r{round}"),"arguments":"{}"}));
                items.push(json!({"type":"function_call_output","call_id":format!("r{round}"),"output":"x".repeat(5000)}));
            } else {
                items.push(json!({"role":"assistant","content":[{"type":"tool_use","name":"read","id":format!("r{round}"),"input":{}}]}));
                items.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":format!("r{round}"),"content":"x".repeat(5000)}]}));
            }
        }
        let field = if wire == Wire::Responses {
            "input"
        } else {
            "messages"
        };
        let mut request = json!({"model":"any"});
        request[field] = json!(items);
        let before = request.clone();
        let report = reduce(&mut request, wire, &config(), &archive, &"a".repeat(64));
        assert_eq!(report.archived, 2);
        assert_eq!(request[field][9], before[field][9]);
        assert_eq!(request[field][12], before[field][12]);
        let mut no_intra = config();
        no_intra.keep_recent_tool_results = 0;
        let mut request = before.clone();
        assert_eq!(
            reduce(&mut request, wire, &no_intra, &archive, &"a".repeat(64)).archived,
            0
        );
        assert_eq!(request, before);
    }
    let mut request = json!({"model":"any","messages":[{"role":"user","content":"parallel task"},{"role":"assistant","content":[]},{"role":"user","content":[]}]});
    for n in 0..10 {
        request["messages"][1]["content"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"tool_use","name":"read","id":format!("p{n}"),"input":{}}));
        request["messages"][2]["content"].as_array_mut().unwrap().push(json!({"type":"tool_result","tool_use_id":format!("p{n}"),"content":"x".repeat(5000)}));
    }
    let before = request.clone();
    assert_eq!(
        reduce(
            &mut request,
            Wire::Messages,
            &config(),
            &archive,
            &"a".repeat(64)
        )
        .archived,
        0
    );
    assert_eq!(request, before);
}
