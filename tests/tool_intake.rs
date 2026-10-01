use astral::{
    archive::Archive,
    config::Config,
    tool_intake::{self, Request},
};
use clap::Parser;
use serde_json::{Value, json};

fn original() -> Value {
    json!({"content":[{"type":"text","text":format!("HEAD\n{}\nhidden-世界-marker\n{}\nTAIL", "ordinary line\n".repeat(1400), "more 😀 lines\n".repeat(1400))}],"isError":false})
}

fn request(result: Value, scope: &str) -> Request {
    Request {
        scope: astral::hash(scope.as_bytes()),
        tool: "read_logs".into(),
        policy: tool_intake::Policy::Preview,
        result,
    }
}

fn handle(value: &Value) -> &str {
    value["content"][0]["text"]
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
fn first_delivery_is_stable_and_every_original_byte_survives_restart() {
    let root = tempfile::tempdir().unwrap();
    let config = Config::parse_from(["astral"]);
    let source = original();
    let mut first = request(source.clone(), "one");
    let archive = Archive::open(root.path(), config.archive_max_bytes).unwrap();
    let report = tool_intake::compact(&mut first, &config, &archive);
    assert!(report.archived);
    assert!(report.bytes_out < report.bytes_in / 5);
    assert!(!first.result.to_string().contains("hidden-世界-marker"));
    assert!(first.result.to_string().contains("HEAD") && first.result.to_string().contains("TAIL"));
    let mut again = request(source.clone(), "one");
    tool_intake::compact(&mut again, &config, &archive);
    assert_eq!(first.result, again.result);
    drop(archive);
    let archive = Archive::open(root.path(), config.archive_max_bytes).unwrap();
    let mut after_restart = request(source.clone(), "one");
    tool_intake::compact(&mut after_restart, &config, &archive);
    assert_eq!(first.result, after_restart.result);
    let mut recovered = Vec::new();
    loop {
        let page = archive
            .recall(handle(&first.result), recovered.len(), 511)
            .unwrap();
        let data = page["data"].as_str().unwrap();
        if page["encoding"] == "hex" {
            recovered.extend(
                (0..data.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&data[i..i + 2], 16).unwrap()),
            );
        } else {
            recovered.extend(data.as_bytes());
        }
        assert_eq!(page["next_offset"], recovered.len());
        if page["eof"] == true {
            assert_eq!(page["sha256"], astral::hash(&recovered));
            break;
        }
    }
    assert_eq!(recovered, serde_json::to_vec(&source).unwrap());
    let matches = archive
        .search(handle(&first.result), "hidden-世界-marker", 0)
        .unwrap();
    let offset = matches["matches"][0]["offset"].as_u64().unwrap() as usize;
    let page = archive.recall(handle(&first.result), offset, 100).unwrap();
    assert!(
        page["data"]
            .as_str()
            .unwrap()
            .starts_with("hidden-世界-marker")
    );
    let mut other = request(source, "two");
    tool_intake::compact(&mut other, &config, &archive);
    assert_ne!(handle(&first.result), handle(&other.result));
}

#[test]
fn errors_rich_results_and_compact_views_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let config = Config::parse_from(["astral"]);
    let archive = Archive::open(root.path(), config.archive_max_bytes).unwrap();
    let mut protected = Vec::new();
    let mut error = original();
    error["isError"] = json!(true);
    protected.push(error);
    let mut structured = original();
    structured["structuredContent"] = json!({"answer":42});
    protected.push(structured);
    let mut annotated = original();
    annotated["content"][0]["annotations"] = json!({"audience":["user"]});
    protected.push(annotated);
    let mut meta = original();
    meta["_meta"] = json!({"key":"value"});
    protected.push(meta);
    let mut image = original();
    image["content"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"image","data":"AA==","mimeType":"image/png"}));
    protected.push(image);
    let mut future = original();
    future["futureField"] = json!(true);
    protected.push(future);
    for failure in [
        json!({"isError":true,"text":"x".repeat(20000)}),
        json!({"exit_code":1,"stderr":"x".repeat(20000)}),
    ] {
        protected.push(json!({"content":[{"type":"text","text":failure.to_string()}]}));
    }
    protected.push(json!({"content":[{"type":"text","text":format!("[astral archived tool output;{}", "x".repeat(20000))}]}));
    for source in protected {
        let mut value = request(source.clone(), "one");
        let report = tool_intake::compact(&mut value, &config, &archive);
        assert_eq!(report.skipped, Some("protected_result"));
        assert_eq!(source, value.result);
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1); // key only
}

#[test]
fn small_outputs_and_full_archive_keep_originals() {
    let root = tempfile::tempdir().unwrap();
    let config = Config::parse_from(["astral"]);
    let archive = Archive::open(root.path(), 1).unwrap();
    for (source, reason) in [
        (original(), "archive_unavailable"),
        (
            json!({"content":[{"type":"text","text":"small"}]}),
            "below_threshold",
        ),
    ] {
        let mut value = request(source.clone(), "one");
        let report = tool_intake::compact(&mut value, &config, &archive);
        assert_eq!(report.skipped, Some(reason));
        assert_eq!(value.result, source);
        assert_eq!(report.bytes_in, report.bytes_out);
    }
}

#[test]
fn default_folds_repetition_and_retains_distinct_facts_and_sequence_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let config = Config::parse_from(["astral"]);
    let archive = Archive::open(root.path(), config.archive_max_bytes).unwrap();
    let mut text = String::from("START facts\n");
    for i in 0..500 {
        text.push_str(&format!(
            "{i:04} task status is unchanged and all checks succeeded\n"
        ));
        if i == 149 {
            text.push_str("unique finding: latency=571, correlation=東京\n");
        }
    }
    text.push_str(&"a repeated line with enough detail to qualify for reduction\n".repeat(200));
    text.push_str("END facts\n");
    let source = json!({"content":[{"type":"text","text":text}],"isError":false});
    let mut value = request(source.clone(), "repetitions");
    value.policy = tool_intake::Policy::Repetitions;
    let report = tool_intake::compact(&mut value, &config, &archive);
    assert!(report.archived && report.bytes_out < report.bytes_in / 5);
    let view = value.result["content"][0]["text"].as_str().unwrap();
    for retained in [
        "START facts",
        "unique finding: latency=571, correlation=東京",
        "END facts",
        "0000 task",
        "0149 task",
        "0150 task",
        "0499 task",
        "150 lines total",
        "350 lines total",
        "200 lines total",
    ] {
        assert!(view.contains(retained), "lost {retained}");
    }
    let archived = archive.recall(handle(&value.result), 0, 16384).unwrap();
    assert_eq!(
        archived["sha256"],
        astral::hash(&serde_json::to_vec(&source).unwrap())
    );
    let mut repeated = request(source, "repetitions");
    repeated.policy = tool_intake::Policy::Repetitions;
    tool_intake::compact(&mut repeated, &config, &archive);
    assert_eq!(value.result, repeated.result);
}

#[test]
fn default_preserves_dense_unique_text_instead_of_cutting_to_a_preview() {
    let root = tempfile::tempdir().unwrap();
    let config = Config::parse_from(["astral"]);
    let archive = Archive::open(root.path(), config.archive_max_bytes).unwrap();
    let text: String = (0..1000)
        .map(|i| {
            format!(
                "{i:04} this unique observation has value {} and must survive\n",
                i * 17
            )
        })
        .collect();
    let source = json!({"content":[{"type":"text","text":text}]});
    let mut value = request(source.clone(), "dense");
    value.policy = tool_intake::Policy::Repetitions;
    let report = tool_intake::compact(&mut value, &config, &archive);
    assert_eq!(report.skipped, Some("no_savings"));
    assert_eq!(value.result, source);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}
