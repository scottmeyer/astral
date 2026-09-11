#![cfg(unix)]
use ostk_gpt_cache::{
    codex::{StagingOptions, stage_native},
    native_bundle::NativeBundle,
};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use tempfile::TempDir;

const ID: &str = "01a10000-1234-7000-8000-000000000001";
const URL: &str = "http://127.0.0.1:12345/backend-api/codex";
const SERVER: &str = r#"#!/usr/bin/env python3
import json, sys, os
from pathlib import Path
root=Path(__file__).resolve().parent
mode=(root/'mode').read_text()
if sys.argv[1:] == ['--version']:
    print('codex-cli ' + ('0.153.0' if mode=='version' else '0.154.0'))
    sys.exit(0)
(root/'argv.json').write_text(json.dumps(sys.argv[1:]))
log=(root/'requests.jsonl').open('a',buffering=1)
for line in sys.stdin:
    log.write(line)
    req=json.loads(line); method=req['method']
    if method=='initialized': continue
    if method=='initialize': result={}
    elif method=='config/read':
        config={'openai_base_url':'http://127.0.0.1:12345/backend-api/codex','features':{'enable_request_compression':False},'model_provider':'openai'}
        if mode=='route': config['openai_base_url']='https://wrong.invalid'
        if mode=='compression': config['features']['enable_request_compression']=True
        if mode=='provider-config': config['model_providers']={'openai':{'base_url':'https://wrong.invalid'}}
        if mode=='config-provider': config['model_provider']='other'
        result={'config':config}
    elif method in ('thread/start','thread/resume'):
        result={'thread':{'id':'01a10000-1234-7000-8000-000000000001','ephemeral':False},'cwd':str(root),'model':'gpt-6-astra','modelProvider':'openai'}
        if mode=='model': result['model']='different'
        if mode=='provider': result['modelProvider']='different'
        if mode=='cwd': result['cwd']=str(root.parent)
        if mode=='ephemeral': result['thread']['ephemeral']=True
        if mode=='id': result['thread']['id']='bad'
        if mode=='resume-id': result['thread']['id']='01a10000-1234-7000-8000-000000000002'
    elif method=='thread/inject_items': result={}
    else:
        (root/'unexpected').write_text(method); sys.exit(99)
    print(json.dumps({'id':req['id'],'result':result}),flush=True)
(root/'closed').write_text('yes')
"#;

fn bundle(payload: &str) -> NativeBundle {
    let items: Value = serde_json::from_str(payload).unwrap();
    let manifest = json!({
        "schema_version":1,"format":"astral-codex-native",
        "payload":{"file":"window.json","sha256":ostk_gpt_cache::hash(payload.as_bytes()),"bytes":payload.len(),"item_count":items.as_array().unwrap().len()},
        "compatibility":{"runtime":"codex","runtime_version":"0.154.0","protocol":"openai-responses-lite","provider":"openai","model":"gpt-6-astra","requires_tool_rebinding":true,"identity_scope":"same-account"},
        "source":{"project_id":"fixture","revision":null,"dirty":false,"selection_sha256":"a".repeat(64),"history_sha256":"b".repeat(64)},
        "capture":{"boundary":"completed-turn","history_complete":true,"last_checkpoint_index":0},"parents":[]
    });
    NativeBundle::validate(&serde_json::to_vec(&manifest).unwrap(), payload.as_bytes()).unwrap()
}
struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    exe: PathBuf,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let exe = root.join("codex");
        fs::write(&exe, SERVER).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(root.join("mode"), mode).unwrap();
        Self {
            _temp: temp,
            root,
            exe,
        }
    }
    fn requests(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}
const PAYLOAD: &str = r#"[{ "type":"compaction", "encrypted_content":"SYNTHETIC\\opaque", "id":"cmp_fixture" },{"type":"message","role":"user","content":[{"type":"input_text","text":"historical tail"}]}]"#;

#[tokio::test]
async fn native_import_preserves_raw_items_and_appends_current_context_without_execution() {
    let f = Fixture::new("ok");
    let b = bundle(PAYLOAD);
    let options = StagingOptions::from_args(
        &f.root,
        &[
            "-s".into(),
            "read-only".into(),
            "-a".into(),
            "never".into(),
            "-c".into(),
            "model_reasoning_effort=\"xhigh\"".into(),
        ],
    )
    .unwrap();
    let staged = stage_native(
        f.exe.as_os_str(),
        &f.root,
        options,
        &b,
        "current docs",
        URL,
        None,
    )
    .await
    .unwrap();
    assert_eq!(staged.thread_id, ID);
    assert_eq!(staged.model, "gpt-6-astra");
    let requests = f.requests();
    let methods: Vec<_> = requests
        .iter()
        .map(|v| v["method"].as_str().unwrap())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "config/read",
            "thread/start",
            "thread/inject_items"
        ]
    );
    assert_eq!(requests[3]["params"]["sandbox"], "read-only");
    assert_eq!(requests[3]["params"]["approvalPolicy"], "never");
    assert_eq!(
        requests[4]["params"]["items"][2]["content"][0]["text"],
        "current docs"
    );
    let log = fs::read_to_string(f.root.join("requests.jsonl")).unwrap();
    for item in b.raw_items() {
        assert!(log.contains(item.get()));
    }
    assert!(f.root.join("closed").exists());
    assert!(!f.root.join("unexpected").exists());
    let argv: Value = serde_json::from_slice(&fs::read(f.root.join("argv.json")).unwrap()).unwrap();
    assert_eq!(argv[4], format!("openai_base_url={URL:?}"));
    assert_eq!(argv[6], "features.enable_request_compression=false");
}

#[tokio::test]
async fn cold_resume_uses_same_thread_without_reinjecting_native_history_or_docs() {
    let f = Fixture::new("ok");
    let b = bundle(PAYLOAD);
    let options = StagingOptions::from_args(&f.root, &[]).unwrap();
    let staged = stage_native(
        f.exe.as_os_str(),
        &f.root,
        options,
        &b,
        "must not inject",
        URL,
        Some(ID),
    )
    .await
    .unwrap();
    assert_eq!(staged.thread_id, ID);
    let req = f.requests();
    assert_eq!(req.last().unwrap()["method"], "thread/resume");
    assert_eq!(req.last().unwrap()["params"]["threadId"], ID);
    assert!(!req.iter().any(|v| v["method"] == "thread/inject_items"));
}

#[tokio::test]
async fn runtime_and_route_mismatches_never_send_native_payload() {
    for (mode, code) in [
        ("version", "NATIVE_RUNTIME_MISMATCH"),
        ("route", "NATIVE_ROUTE_CONFLICT"),
        ("compression", "NATIVE_ROUTE_CONFLICT"),
        ("provider-config", "NATIVE_ROUTE_CONFLICT"),
        ("config-provider", "NATIVE_ROUTE_CONFLICT"),
        ("model", "NATIVE_RUNTIME_MISMATCH"),
        ("provider", "NATIVE_RUNTIME_MISMATCH"),
        ("cwd", "WORKSPACE_CONFLICT"),
        ("ephemeral", "CODEX_PROTOCOL"),
        ("id", "CODEX_PROTOCOL"),
        ("resume-id", "CODEX_PROTOCOL"),
    ] {
        let f = Fixture::new(mode);
        let options = StagingOptions::from_args(&f.root, &[]).unwrap();
        let err = stage_native(
            f.exe.as_os_str(),
            &f.root,
            options,
            &bundle(PAYLOAD),
            "docs",
            URL,
            if mode == "resume-id" { Some(ID) } else { None },
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, code, "{mode}");
        assert!(
            !f.requests()
                .iter()
                .any(|v| v["method"] == "thread/inject_items"),
            "{mode}"
        );
        assert!(!format!("{err:?}").contains("SYNTHETIC"));
    }
}

#[test]
fn import_gate_rejects_fields_codex_would_drop_without_changing_bundle_availability() {
    for payload in [
        r#"[{"type":"compaction","encrypted_content":"opaque","unknown_future_field":true}]"#,
        r#"[{"type":"compaction","encrypted_content":"opaque","internal_chat_message_metadata_passthrough":{"cell_id":"host-owned"}}]"#,
        r#"[{"type":"compaction","encrypted_content":"opaque"},{"type":"message","role":"user","content":[{"type":"input_text","text":"tail","annotations":[]}]}]"#,
    ] {
        let b = bundle(payload);
        assert_eq!(b.payload_bytes(), payload.as_bytes());
        assert_eq!(
            ostk_gpt_cache::native_import::validate(&b)
                .unwrap_err()
                .code,
            "NATIVE_IMPORT_UNSUPPORTED"
        );
    }
}

#[test]
fn import_gate_rejects_lossy_numeric_values_inside_opaque_tool_search_json() {
    for (number, accepted) in [
        ("18446744073709551615", true),
        ("18446744073709551617", false),
        ("1.2300e2", true),
        ("0.123456789012345678901", false),
    ] {
        let payload = format!(
            r#"[{{"type":"compaction","encrypted_content":"opaque"}},{{"type":"tool_search_call","execution":"server","call_id":null,"status":"completed","arguments":{{"value":{number},"literal":"9999999999999999999999999"}}}}]"#
        );
        let b = bundle(&payload);
        assert_eq!(
            ostk_gpt_cache::native_import::validate(&b).is_ok(),
            accepted,
            "{number}"
        );
        assert_eq!(b.payload_bytes(), payload.as_bytes());
    }
}

#[test]
fn import_gate_rejects_reasoning_content_that_codex_omits() {
    for (content, accepted) in [
        (r#"[{"type":"text","text":"must survive"}]"#, false),
        (r#"[{"type":"reasoning_text","text":"preserved"}]"#, true),
        (
            r#"[{"type":"reasoning_text","text":"preserved"},{"type":"text","text":"also preserved"}]"#,
            true,
        ),
    ] {
        let payload = format!(
            r#"[{{"type":"compaction","encrypted_content":"opaque"}},{{"type":"reasoning","summary":[],"content":{content}}}]"#
        );
        assert_eq!(
            ostk_gpt_cache::native_import::validate(&bundle(&payload)).is_ok(),
            accepted
        );
    }
}
