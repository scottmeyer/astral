#![cfg(unix)]

use ostk_gpt_cache::{hash, native_bundle::NativeBundle, project::Project, projection_save};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const THREAD: &str = "01234567-89ab-cdef-0123-456789abcdef";
// A synthetic app-server supplies protocol events and an owned rollout. It never
// connects to a provider; the production save/launch/proxy/publication paths run.
const FAKE: &str = r#"#!/usr/bin/env python3
import json, os, pathlib, sys, tomllib
root = pathlib.Path.cwd()
rollout = pathlib.Path(os.environ['SAVE_ROLLOUT'])
log = pathlib.Path(os.environ['SAVE_LOG'])
thread = '01234567-89ab-cdef-0123-456789abcdef'
def append(kind, payload):
    ordinal = len(rollout.read_text().splitlines()) if rollout.exists() else 0
    with rollout.open('a') as f:
        f.write(json.dumps(dict(timestamp='2026-09-11T00:00:00Z', ordinal=ordinal, type=kind, payload=payload)) + '\n')
def emit(method, params):
    print(json.dumps(dict(method=method, params=params)), flush=True)
if sys.argv[1] == '--version':
    print('codex-cli 0.154.0')
elif sys.argv[1:3] == ['exec', 'resume']:
    assert sys.argv[3] == thread
    print('synthetic worker resumed')
elif sys.argv[1] == 'app-server':
    config = {}
    for i, arg in enumerate(sys.argv):
        if arg == '-c': config.update(tomllib.loads(sys.argv[i+1]))
    for line in sys.stdin:
        req = json.loads(line)
        with log.open('a') as f: f.write(json.dumps(req) + '\n')
        if 'id' not in req: continue
        method, params = req['method'], req['params']
        result = {}
        if method == 'config/read': result = dict(config=config)
        if method in ['thread/start', 'thread/resume', 'thread/read']:
            if method == 'thread/start':
                assert not rollout.exists()
                append('session_meta', dict(id=thread, cli_version='0.154.0', history_mode='paginated', history_base=None))
            else: assert params['threadId'] == thread
            result = dict(thread=dict(id=thread, ephemeral=False, path=str(rollout)), cwd=str(root), model='gpt-6-astra', modelProvider='openai')
        if method == 'thread/inject_items':
            assert params['threadId'] == thread
            for item in params['items']: append('response_item', item)
            if os.environ.get('SAVE_INJECT_DROP'): sys.exit(0)
        if method == 'thread/compact/start':
            assert params['threadId'] == thread
            if os.environ.get('SAVE_FAIL'):
                print(json.dumps(dict(id=req['id'], result={})), flush=True)
                emit('error', dict(threadId=thread, message='synthetic failure'))
                continue
            number = len(rollout.read_text().splitlines())
            turn, item = 'save-' + str(number), '11234567-89ab-cdef-0123-' + format(number, '012x')
            emit('turn/started', dict(threadId=thread, turn=dict(id=turn)))
            emit('item/started', dict(threadId=thread, turnId=turn, item=dict(id=item, type='contextCompaction')))
            append('event_msg', dict(type='task_started', turn_id=turn))
            append('compacted', dict(message='', replacement_history=[dict(type='compaction', encrypted_content='synthetic-' + turn)], window_number=number))
            append('event_msg', dict(type='item_completed', thread_id=thread, turn_id=turn, item=dict(id=item, type='ContextCompaction')))
            append('event_msg', dict(type='task_complete', turn_id=turn, error=None))
            emit('item/completed', dict(threadId=thread, turnId=turn, item=dict(id=item, type='contextCompaction')))
            emit('turn/completed', dict(threadId=thread, turn=dict(id=turn, status='completed', error=None)))
        print(json.dumps(dict(id=req['id'], result=result)), flush=True)
else: raise AssertionError(sys.argv)
"#;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    bin: PathBuf,
    log: PathBuf,
    rollout: PathBuf,
    work: String,
    target: PathBuf,
    seed: String,
}

fn put(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn bundle(opaque: &str) -> NativeBundle {
    let payload =
        serde_json::to_vec(&json!([{"type":"compaction","encrypted_content":opaque}])).unwrap();
    let manifest = serde_json::to_vec(&json!({
        "schema_version":1,"format":"astral-codex-native",
        "payload":{"file":"window.json","sha256":hash(&payload),"bytes":payload.len(),"item_count":1},
        "compatibility":{"runtime":"codex","runtime_version":"0.154.0","protocol":"openai-responses-lite","provider":"openai","model":"gpt-6-astra","requires_tool_rebinding":true,"identity_scope":"same-account"},
        "source":{"project_id":"save-test","revision":null,"dirty":true,"selection_sha256":"a".repeat(64),"history_sha256":"b".repeat(64)},
        "capture":{"boundary":"completed-compaction","history_complete":true,"last_checkpoint_index":0},"parents":[]
    })).unwrap();
    NativeBundle::validate(&manifest, &payload).unwrap()
}

fn checked(out: Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or(Value::Null)
}

fn error(out: Output) -> String {
    assert!(!out.status.success());
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .next_back()
        .unwrap()["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("source");
        fs::create_dir(&root).unwrap();
        put(
            &root,
            ".astral/project.toml",
            "schema_version = 1\nid = \"save-test\"\nname = \"Save test\"\ndescription = \"Synthetic fixture\"\ncore = \"core\"\nprojections = \"projections\"\nwork_items = \"work/items.jsonl\"\n[identity]\nscope = \"local\"\nruntime_bindings = \"external\"\n[subsystems]\nweb = \"core/web\"\ncommon = \"core/common\"\n",
        );
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            put(
                &root,
                &format!(".astral/core/{name}"),
                "Synthetic current document.\n",
            );
        }
        for (id, depends) in [("web", "[\"common\"]"), ("common", "[]")] {
            put(
                &root,
                &format!(".astral/core/{id}/subsystem.toml"),
                format!(
                    "schema_version = 1\nid = {id:?}\npurpose = \"Fixture\"\nreadme = \"README.md\"\nrules = []\ndecisions = []\nwork_items = []\nprojection = \"p\"\ndepends_on = {depends}\n"
                ),
            );
            put(
                &root,
                &format!(".astral/core/{id}/README.md"),
                "Synthetic subsystem.\n",
            );
        }
        put(
            &root,
            ".astral/projections/p/projection.toml",
            "schema_version = 1\nid = \"p\"\nkind = \"fresh-context\"\nsubsystems = [\"web\"]\nhandoff = \"handoff.md\"\nnative_payload_in_repository = false\nsources = []\n",
        );
        put(
            &root,
            ".astral/projections/p/handoff.md",
            "Preserve authored handoff.\n",
        );
        put(&root, ".astral/work/items.jsonl", "");
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .current_dir(&root)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-b", "main"]);
        let initial = bundle("initial-native-seed");
        let seed = initial.summary().manifest_sha256;
        projection_save::publish(&root, "p", &["web".into(), "common".into()], &initial, None)
            .unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "save fixture"]);
        let bin = base.join("fake-codex");
        fs::write(&bin, FAKE).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
        let mut fixture = Self {
            _temp: temp,
            root,
            bin,
            log: base.join("rpc.jsonl"),
            rollout: base.join("rollout.jsonl"),
            work: String::new(),
            target: PathBuf::new(),
            seed,
        };
        let work = checked(fixture.invoke(
            &[
                "work",
                "create",
                "Save fixture",
                "--acceptance",
                "resume selected p after saving p and q",
            ],
            false,
        ));
        fixture.work = work["item"]["id"].as_str().unwrap().into();
        let inspect = checked(fixture.invoke(
            &[
                "project",
                "projection:p",
                "--work",
                &fixture.work,
                "--inspect",
            ],
            false,
        ));
        fixture.target = PathBuf::from(inspect["worktree"]["root"].as_str().unwrap());
        checked(fixture.launch());
        fixture
    }

    fn invoke(&self, args: &[&str], fail: bool) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_astral"));
        cmd.arg("--root")
            .arg(&self.root)
            .args(args)
            .env("ASTRAL_CODEX_BIN", &self.bin)
            .env("SAVE_LOG", &self.log)
            .env("SAVE_ROLLOUT", &self.rollout);
        if fail {
            cmd.env("SAVE_FAIL", "1");
        } else {
            cmd.env_remove("SAVE_FAIL");
        }
        cmd.output().unwrap()
    }
    fn launch(&self) -> Output {
        self.invoke(
            &[
                "project",
                "projection:p",
                "--work",
                &self.work,
                "--proxy",
                "--non-interactive",
                "--",
                "-c",
                "sandbox_mode=\"read-only\"",
                "-c",
                "approval_policy=\"never\"",
                "continue",
            ],
            false,
        )
    }
    fn save(&self, name: &str, fail: bool) -> Output {
        self.invoke(
            &[
                "save",
                name,
                "--context",
                "projection:p",
                "--work",
                &self.work,
                "--proxy",
                "--",
                "--sandbox",
                "read-only",
                "--ask-for-approval",
                "never",
            ],
            fail,
        )
    }
    fn metadata(&self) -> Value {
        let bytes = fs::read(
            self.root
                .join(".git/astral/work-bindings")
                .join(&self.work)
                .join("receipt.json"),
        )
        .unwrap();
        serde_json::from_slice::<Value>(&bytes).unwrap()["worker_metadata"].clone()
    }
    fn rows(&self) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}

#[test]
fn save_selected_then_another_name_resumes_the_original_worker_and_preserves_scope() {
    let f = Fixture::new();
    let b = checked(f.save("p", false));
    let b_hash = b["publication"]["bundle_manifest"]["sha256"]
        .as_str()
        .unwrap();
    assert_ne!(b_hash, f.seed);
    let c = checked(f.save("q", false));
    let c_hash = c["publication"]["bundle_manifest"]["sha256"]
        .as_str()
        .unwrap();
    assert_ne!(b_hash, c_hash);
    let metadata = f.metadata();
    assert_eq!(metadata["seed_bundle_sha256"], f.seed);
    assert_eq!(metadata["selected_bundle_sha256"], b_hash);
    assert_eq!(metadata["saved_bundle_sha256"], c_hash);
    assert_eq!(metadata["selected_bundle_recorded"], true);
    assert_eq!(metadata["thread_id"], THREAD);
    checked(f.launch());
    let rows = f.rows();
    assert_eq!(
        rows.iter()
            .filter(|v| v["method"] == "thread/start")
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|v| v["method"] == "thread/compact/start")
            .count(),
        2
    );
    assert_eq!(
        rows.iter()
            .filter(|v| v["method"] == "thread/resume")
            .count(),
        3
    );
    let native_injections = rows
        .iter()
        .filter(|v| v["method"] == "thread/inject_items")
        .flat_map(|v| v["params"]["items"].as_array().unwrap())
        .filter(|v| v["type"] == "compaction")
        .count();
    assert_eq!(
        native_injections, 1,
        "resume must never reinject old native history"
    );
    let manifest: toml::Value = toml::from_str(
        &fs::read_to_string(f.target.join(".astral/projections/p/projection.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["subsystems"].as_array().unwrap().len(), 1);
    assert_eq!(manifest["subsystems"][0].as_str(), Some("web"));
    let saved = Project::load(&f.target)
        .unwrap()
        .launch_context("projection:q", None)
        .unwrap()
        .native
        .unwrap();
    assert!(saved.manifest().parents.contains(&f.seed));
    assert!(saved.manifest().parents.contains(&b_hash.to_owned()));

    // An unrelated edit of the selected projection still cannot silently rebind
    // this existing worker, even after successful named exports.
    projection_save::publish(
        &f.target,
        "p",
        &["common".into(), "web".into()],
        &bundle("unrelated"),
        None,
    )
    .unwrap();
    let count = f.rows().len();
    assert_eq!(error(f.launch()), "WORKSPACE_SEED_CHANGED");
    assert_eq!(error(f.save("q", false)), "WORKSPACE_SEED_CHANGED");
    assert_eq!(f.rows().len(), count);
}

#[test]
fn failed_compaction_does_not_publish_and_the_same_worker_can_retry() {
    let f = Fixture::new();
    let original = fs::read(f.target.join(".astral/projections/p/projection.toml")).unwrap();
    assert_eq!(error(f.save("q", true)), "CAPTURE_FAILED");
    assert!(!f.target.join(".astral/projections/q").exists());
    assert_eq!(
        fs::read(f.target.join(".astral/projections/p/projection.toml")).unwrap(),
        original
    );
    assert_eq!(f.metadata()["requires_tool_rebinding"], true);
    assert_eq!(f.metadata()["saved_bundle_sha256"], Value::Null);
    checked(f.save("q", false));
    checked(f.launch());
    assert_eq!(f.metadata()["thread_id"], THREAD);
    assert_eq!(f.metadata()["selected_bundle_sha256"], f.seed);
}

#[test]
fn lost_save_context_injection_acknowledgement_blocks_blind_retry() {
    let f = Fixture::new();
    put(
        &f.target,
        ".astral/core/ARCHITECTURE.md",
        "Changed save context.\n",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_astral"))
        .arg("--root")
        .arg(&f.root)
        .args([
            "save",
            "q",
            "--context",
            "projection:p",
            "--work",
            &f.work,
            "--proxy",
        ])
        .env("ASTRAL_CODEX_BIN", &f.bin)
        .env("SAVE_LOG", &f.log)
        .env("SAVE_ROLLOUT", &f.rollout)
        .env("SAVE_INJECT_DROP", "1")
        .output()
        .unwrap();
    assert_eq!(error(out), "CODEX_PROTOCOL");
    assert_eq!(f.metadata()["staging_in_progress"], true);
    let log = fs::read(&f.log).unwrap();
    assert_eq!(error(f.save("q", false)), "WORKSPACE_RECOVERY_REQUIRED");
    assert_eq!(error(f.launch()), "WORKSPACE_STAGE_INCOMPLETE");
    let preview = checked(f.invoke(&["recover", "--work", &f.work], false));
    assert_eq!(preview["classification"], "staging_unknown");
    assert!(preview["repair"].is_null());
    assert_eq!(fs::read(&f.log).unwrap(), log);
}
