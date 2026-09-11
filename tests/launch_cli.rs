#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

const FAKE: &str = r#"#!/usr/bin/env python3
import json, os, pathlib, re, sys, tomllib
root = pathlib.Path.cwd()
log = pathlib.Path(os.environ['ASTRAL_TEST_LOG'])
def record(kind, **values):
    with log.open('a') as f: f.write(json.dumps(dict(kind=kind, **values)) + '\n')
record('argv', args=sys.argv[1:], cwd=str(root))
mode = os.environ.get('ASTRAL_TEST_MODE', '')
if sys.argv[1] == '--version':
    print('codex-cli ' + ('0.153.0' if mode == 'wrong-version' else '0.154.0'))
elif sys.argv[1] == 'app-server':
    if mode == 'server-exit': sys.exit(17)
    config = {}
    for i, arg in enumerate(sys.argv):
        if arg == '-c': config.update(tomllib.loads(sys.argv[i+1]))
    for line in sys.stdin:
        request = json.loads(line)
        record('rpc', request=request)
        if 'id' not in request: continue
        if mode == 'initialize-fail' and request['method'] == 'initialize':
            print(json.dumps(dict(id=request['id'], error=dict(code=-1,message='fixture initialization rejection'))), flush=True)
            continue
        if mode == 'thread-start-drop' and request['method'] == 'thread/start': sys.exit(0)
        if mode == 'inject-drop' and request['method'] == 'thread/inject_items': sys.exit(0)
        result = {}
        if request['method'] == 'config/read':
            result = dict(config=config)
            if mode == 'route-conflict': result['config']['openai_base_url'] = 'https://wrong.invalid'
        if request['method'] in ['thread/start', 'thread/resume']:
            result = dict(thread=dict(id='01234567-89ab-cdef-0123-456789abcdef', ephemeral=False), cwd=str(root), model='gpt-6-astra', modelProvider='openai')
        if mode == 'native-stage-fail' and request['method'] == 'thread/inject_items':
            print(json.dumps(dict(id=request['id'], error=dict(code=-1,message='fixture rejection'))), flush=True)
            continue
        print(json.dumps(dict(id=request['id'], result=result)), flush=True)
    record('closed')
    if mode == 'shutdown-fail': sys.exit(17)
elif sys.argv[1] == 'resume' or sys.argv[1:3] == ['exec', 'resume']:
    assert any(json.loads(row)['kind'] == 'closed' for row in log.read_text().splitlines())
    record('resumed')
    if sys.argv[1] == 'exec': record('headless_stdin', value=sys.stdin.read())
    sys.exit(23 if mode == 'fail' else 0)
else:
    prompt = sys.argv[2] if sys.argv[1] == 'exec' else sys.argv[1]
    assert 'Astral' in prompt and 'schema_version = 1' in prompt
    if mode == 'fail': sys.exit(23)
    if mode == 'noop': sys.exit(0)
    if mode == 'invalid':
        (root / '.astral').mkdir()
        (root / '.astral/project.toml').write_text('invalid schema')
        sys.exit(0)
    target = None
    lines = iter(prompt.splitlines())
    for line in lines:
        if line.startswith('### `') and line.endswith('`'): target = line[5:-1]
        elif line == '```toml':
            rows = []
            for line in lines:
                if line == '```': break
                rows.append(line)
            path = root / target
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('\n'.join(rows) + '\n')
    for name in ['core/ARCHITECTURE.md','core/RUN.md','core/TEST.md','core/project-context/README.md','projections/initial-project-context/handoff.md']:
        (root / '.astral' / name).write_text('SYNTHETIC_DOC --yolo $(literal). Unverified.\n')
    work = root / '.astral/work/items.jsonl'
    work.parent.mkdir(parents=True)
    work.write_text('')
"#;

fn fixture(path: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let bin = path.join("fake-codex");
    fs::write(&bin, FAKE).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
    (bin, path.join("audit.jsonl"))
}

fn invoke(root: &Path, bin: &Path, log: &Path, args: &[&str], mode: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_astral"))
        .args(["--root"])
        .arg(root)
        .args(args)
        .env("ASTRAL_CODEX_BIN", bin)
        .env("ASTRAL_TEST_LOG", log)
        .env("ASTRAL_TEST_MODE", mode)
        .output()
        .unwrap()
}

fn error(output: &Output) -> String {
    let text = String::from_utf8_lossy(&output.stderr);
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .next_back()
        .unwrap()["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn work_launch_carries_new_record_and_resumes_same_worktree_without_copying_code_edits() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let (bin, log) = fixture(temp.path());
    assert!(
        invoke(&source, &bin, &log, &["init", "--non-interactive"], "")
            .status
            .success()
    );
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&source)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
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
    fs::write(source.join("code.txt"), "committed code").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "fixture"]);
    fs::write(source.join("code.txt"), "source edit").unwrap();
    let created = invoke(
        &source,
        &bin,
        &log,
        &[
            "work",
            "create",
            "Fixture worker",
            "--acceptance",
            "exercise binding",
        ],
        "",
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let record: Value = serde_json::from_slice(&created.stdout).unwrap();
    let work = record["item"]["id"].as_str().unwrap();
    let inspect = invoke(
        &source,
        &bin,
        &log,
        &["project", "--work", work, "--inspect"],
        "",
    );
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspection: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    let target = std::path::PathBuf::from(inspection["worktree"]["root"].as_str().unwrap());
    assert!(!target.exists(), "inspection created the worktree");
    let launched = invoke(
        &source,
        &bin,
        &log,
        &[
            "project",
            "--work",
            work,
            "--non-interactive",
            "--",
            "--json",
            "literal $() ; task",
        ],
        "",
    );
    assert!(
        launched.status.success(),
        "{}",
        String::from_utf8_lossy(&launched.stderr)
    );
    assert_eq!(
        fs::read_to_string(target.join("code.txt")).unwrap(),
        "committed code"
    );
    assert_eq!(
        fs::read_to_string(source.join("code.txt")).unwrap(),
        "source edit"
    );
    assert_eq!(
        fs::read(target.join(".astral/work/items.jsonl")).unwrap(),
        fs::read(source.join(".astral/work/items.jsonl")).unwrap()
    );
    fs::write(target.join("code.txt"), "worker edit").unwrap();
    let resumed = invoke(
        &source,
        &bin,
        &log,
        &[
            "project",
            "--work",
            work,
            "--non-interactive",
            "--",
            "continue",
        ],
        "",
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        fs::read_to_string(target.join("code.txt")).unwrap(),
        "worker edit"
    );
    let rows: Vec<Value> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        rows.iter()
            .filter(|r| r["kind"] == "rpc" && r["request"]["method"] == "thread/start")
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r["kind"] == "rpc" && r["request"]["method"] == "thread/resume")
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r["kind"] == "rpc" && r["request"]["method"] == "thread/inject_items")
            .count(),
        1
    );
    assert!(rows.iter().any(|r| {
        r["kind"] == "argv"
            && r["args"]
                .as_array()
                .is_some_and(|a| a.last() == Some(&serde_json::json!("literal $() ; task")))
            && r["cwd"] == target.to_str().unwrap()
    }));
    let receipt_path = source
        .join(".git/astral/work-bindings")
        .join(work)
        .join("receipt.json");
    let mut receipt: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
    receipt["worker_metadata"]["context_initialized"] = serde_json::json!(false);
    fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let incomplete = invoke(
        &source,
        &bin,
        &log,
        &["project", "--work", work, "--non-interactive"],
        "",
    );
    assert_eq!(error(&incomplete), "WORKSPACE_CONTEXT_INCOMPLETE");
    assert_eq!(
        fs::read_to_string(target.join("code.txt")).unwrap(),
        "worker edit"
    );

    let broken = invoke(
        &source,
        &bin,
        &log,
        &[
            "work",
            "create",
            "Failed staging",
            "--acceptance",
            "retain unknown staging outcome",
        ],
        "",
    );
    let record: Value = serde_json::from_slice(&broken.stdout).unwrap();
    let broken_work = record["item"]["id"].as_str().unwrap();
    let failure = invoke(
        &source,
        &bin,
        &log,
        &["project", "--work", broken_work, "--non-interactive"],
        "native-stage-fail",
    );
    assert_eq!(error(&failure), "CODEX_REQUEST_FAILED");
    let retried = invoke(
        &source,
        &bin,
        &log,
        &["project", "--work", broken_work, "--non-interactive"],
        "",
    );
    assert_eq!(error(&retried), "WORKSPACE_STAGE_INCOMPLETE");
    fs::write(source.join(".astral/core/RUN.md"), "source context edit").unwrap();
    let new = invoke(
        &source,
        &bin,
        &log,
        &[
            "work",
            "create",
            "Second worker",
            "--acceptance",
            "reject dirty context",
        ],
        "",
    );
    let record: Value = serde_json::from_slice(&new.stdout).unwrap();
    let new_work = record["item"]["id"].as_str().unwrap();
    let rejected = invoke(
        &source,
        &bin,
        &log,
        &["project", "--work", new_work, "--non-interactive"],
        "",
    );
    assert_eq!(error(&rejected), "WORKSPACE_UNCOMMITTED_CONTEXT");
}

#[test]
fn pre_thread_worker_failures_leave_same_work_retryable_for_fresh_and_native_seeds() {
    for native in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let (bin, log) = fixture(temp.path());
        if native {
            native_fixture(&source, &bin, &log);
        } else {
            assert!(
                invoke(&source, &bin, &log, &["init", "--non-interactive"], "")
                    .status
                    .success()
            );
        }
        for args in [
            vec!["init", "-b", "main"],
            vec!["add", "."],
            vec!["commit", "-m", "fixture"],
        ] {
            let out = Command::new("git")
                .current_dir(&source)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        for (mode, expected) in [
            ("missing-executable", "CODEX_UNAVAILABLE"),
            ("wrong-version", "NATIVE_RUNTIME_MISMATCH"),
            ("server-exit", "CODEX_PROTOCOL"),
            ("initialize-fail", "CODEX_REQUEST_FAILED"),
            ("route-conflict", "NATIVE_ROUTE_CONFLICT"),
        ] {
            let created = invoke(
                &source,
                &bin,
                &log,
                &[
                    "work",
                    "create",
                    mode,
                    "--acceptance",
                    "retry known no-thread failure",
                ],
                "",
            );
            assert!(created.status.success());
            let record: Value = serde_json::from_slice(&created.stdout).unwrap();
            let work = record["item"]["id"].as_str().unwrap();
            let mut args = vec!["project", "--work", work, "--non-interactive"];
            if native || mode == "route-conflict" {
                args.push("--proxy");
            }
            let before = native_rows(&log).len();
            let missing = temp.path().join("does-not-exist");
            let first = invoke(
                &source,
                if mode == "missing-executable" {
                    &missing
                } else {
                    &bin
                },
                &log,
                &args,
                mode,
            );
            assert_eq!(
                error(&first),
                expected,
                "native={native} mode={mode}: {}",
                String::from_utf8_lossy(&first.stderr)
            );
            let receipt_path = source
                .join(".git/astral/work-bindings")
                .join(work)
                .join("receipt.json");
            let receipt: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
            assert_eq!(receipt["worker_metadata"]["context_initialized"], true);
            assert_eq!(
                receipt["worker_metadata"]["staging_in_progress"], false,
                "native={native} mode={mode}"
            );
            assert!(receipt["worker_metadata"]["thread_id"].is_null());
            assert!(
                !native_rows(&log)[before..]
                    .iter()
                    .any(|r| r["request"]["method"] == "thread/start")
            );
            let root = receipt["root"].clone();
            let retried = invoke(&source, &bin, &log, &args, "");
            assert!(
                retried.status.success(),
                "native={native} mode={mode}: {}",
                String::from_utf8_lossy(&retried.stderr)
            );
            let completed: Value =
                serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
            assert_eq!(completed["root"], root);
            assert_eq!(completed["worker_metadata"]["staging_in_progress"], false);
            assert_eq!(
                completed["worker_metadata"]["selected_bundle_recorded"],
                true
            );
            assert_eq!(
                completed["worker_metadata"]["selected_bundle_sha256"].is_string(),
                native
            );
            assert_eq!(
                native_rows(&log)[before..]
                    .iter()
                    .filter(|r| r["request"]["method"] == "thread/start")
                    .count(),
                1
            );
        }
    }
}

#[test]
fn lost_thread_start_response_retains_uncertain_marker_and_blocks_duplicate_creation() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let (bin, log) = fixture(temp.path());
    assert!(
        invoke(&source, &bin, &log, &["init", "--non-interactive"], "")
            .status
            .success()
    );
    for args in [
        vec!["init", "-b", "main"],
        vec!["add", "."],
        vec!["commit", "-m", "fixture"],
    ] {
        let out = Command::new("git")
            .current_dir(&source)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let created = invoke(
        &source,
        &bin,
        &log,
        &[
            "work",
            "create",
            "uncertain creation",
            "--acceptance",
            "do not duplicate a potentially created thread",
        ],
        "",
    );
    let record: Value = serde_json::from_slice(&created.stdout).unwrap();
    let work = record["item"]["id"].as_str().unwrap();
    let args = ["project", "--work", work, "--non-interactive"];
    let failed = invoke(&source, &bin, &log, &args, "thread-start-drop");
    assert_eq!(error(&failed), "CODEX_PROTOCOL");
    let path = source
        .join(".git/astral/work-bindings")
        .join(work)
        .join("receipt.json");
    let receipt: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(receipt["worker_metadata"]["staging_in_progress"], true);
    assert!(receipt["worker_metadata"]["thread_id"].is_null());
    let before = fs::read(&log).unwrap();
    let retried = invoke(&source, &bin, &log, &args, "");
    assert_eq!(error(&retried), "WORKSPACE_STAGE_INCOMPLETE");
    assert_eq!(fs::read(&log).unwrap(), before);
    assert_eq!(
        native_rows(&log)
            .iter()
            .filter(|r| r["request"]["method"] == "thread/start")
            .count(),
        1
    );
}

#[test]
fn staging_recovery_and_uncertain_resume_never_duplicate_thread_or_context() {
    for native in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let (bin, log) = fixture(temp.path());
        if native {
            native_fixture(&source, &bin, &log);
        } else {
            assert!(
                invoke(&source, &bin, &log, &["init", "--non-interactive"], "")
                    .status
                    .success()
            );
        }
        for args in [
            vec!["init", "-b", "main"],
            vec!["add", "."],
            vec!["commit", "-m", "fixture"],
        ] {
            let out = Command::new("git")
                .current_dir(&source)
                .args([
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success());
        }
        let created = invoke(
            &source,
            &bin,
            &log,
            &[
                "work",
                "create",
                "Recover staged worker",
                "--acceptance",
                "No duplicate creation",
            ],
            "",
        );
        let record: Value = serde_json::from_slice(&created.stdout).unwrap();
        let work = record["item"]["id"].as_str().unwrap();
        let mut args = vec!["project", "--work", work, "--non-interactive"];
        if native {
            args.push("--proxy");
        }
        assert_eq!(
            error(&invoke(&source, &bin, &log, &args, "shutdown-fail")),
            "CODEX_SHUTDOWN_FAILED"
        );
        let before = fs::read(&log).unwrap();
        assert_eq!(
            error(&invoke(&source, &bin, &log, &args, "")),
            "WORKSPACE_RECOVERY_REQUIRED"
        );
        assert_eq!(fs::read(&log).unwrap(), before);
        let preview = invoke(&source, &bin, &log, &["recover", "--work", work], "");
        assert!(
            preview.status.success(),
            "{}",
            String::from_utf8_lossy(&preview.stderr)
        );
        let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();
        assert_eq!(plan["classification"], "staging_acknowledged");
        let applied = invoke(
            &source,
            &bin,
            &log,
            &[
                "recover",
                "--work",
                work,
                "--apply",
                plan["plan_sha256"].as_str().unwrap(),
            ],
            "",
        );
        assert!(
            applied.status.success(),
            "{}",
            String::from_utf8_lossy(&applied.stderr)
        );
        assert_eq!(
            fs::read(&log).unwrap(),
            before,
            "recovery must not invoke Codex"
        );
        let continued = invoke(&source, &bin, &log, &args, "");
        assert!(
            continued.status.success(),
            "{}",
            String::from_utf8_lossy(&continued.stderr)
        );
        assert_eq!(
            native_rows(&log)
                .iter()
                .filter(|r| r["request"]["method"] == "thread/start")
                .count(),
            1
        );
        // The initial context was acknowledged before the shutdown failure.
        assert_eq!(
            native_rows(&log)
                .iter()
                .filter(|r| r["request"]["method"] == "thread/inject_items")
                .count(),
            1
        );
        let receipt: Value = serde_json::from_slice(
            &fs::read(source.join(format!(".git/astral/work-bindings/{work}/receipt.json")))
                .unwrap(),
        )
        .unwrap();
        let target = Path::new(receipt["root"].as_str().unwrap());
        fs::write(
            target.join(".astral/core/ARCHITECTURE.md"),
            "New readable context.\n",
        )
        .unwrap();
        assert_eq!(
            error(&invoke(&source, &bin, &log, &args, "inject-drop")),
            "CODEX_PROTOCOL"
        );
        let before_retry = fs::read(&log).unwrap();
        let preview = invoke(&source, &bin, &log, &["recover", "--work", work], "");
        let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();
        assert_eq!(plan["classification"], "staging_unknown");
        assert!(plan["repair"].is_null());
        assert_eq!(
            error(&invoke(&source, &bin, &log, &args, "")),
            "WORKSPACE_STAGE_INCOMPLETE"
        );
        assert_eq!(fs::read(&log).unwrap(), before_retry);
    }
}

#[test]
fn cli_initializes_validates_stages_and_resumes_with_literal_arguments() {
    let root = tempfile::tempdir().unwrap();
    let (bin, log) = fixture(root.path());
    let initialized = invoke(
        root.path(),
        &bin,
        &log,
        &[
            "init",
            "--non-interactive",
            "--",
            "--sandbox",
            "workspace-write",
            "-c",
            "approval_policy=\"never\"",
        ],
        "",
    );
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    let before = fs::read(root.path().join(".astral/core/RUN.md")).unwrap();
    let raw = [
        "--model",
        "gpt-6-astra",
        "--sandbox=read-only",
        "--ask-for-approval",
        "never",
        "literal $() `text` ; with spaces",
    ];
    let mut args = vec!["project", "--"];
    args.extend(raw);
    let launched = invoke(root.path(), &bin, &log, &args, "");
    assert!(
        launched.status.success(),
        "{}",
        String::from_utf8_lossy(&launched.stderr)
    );
    assert!(launched.stdout.is_empty());
    assert_eq!(
        fs::read(root.path().join(".astral/core/RUN.md")).unwrap(),
        before
    );
    let rows: Vec<Value> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let args = rows.iter().rfind(|row| row["kind"] == "argv").unwrap()["args"]
        .as_array()
        .unwrap();
    assert_eq!(
        &args[..2],
        &[
            Value::from("resume"),
            Value::from("01234567-89ab-cdef-0123-456789abcdef")
        ]
    );
    assert_eq!(&args[2..], raw.map(Value::from));
    let injected = rows
        .iter()
        .find(|row| row["request"]["method"] == "thread/inject_items")
        .unwrap();
    let text = injected["request"]["params"]["items"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("SYNTHETIC_DOC --yolo $(literal)"));
    assert!(text.contains("not restored native memory"));
    assert!(
        !rows
            .iter()
            .any(|row| row["request"]["method"] == "turn/start")
    );
    let failed = invoke(root.path(), &bin, &log, &["project"], "fail");
    assert_eq!(failed.status.code(), Some(23));
}

#[test]
fn headless_project_executes_resume_with_null_stdin_and_exact_raw_suffix() {
    use std::io::Write;
    use std::process::Stdio;
    let root = tempfile::tempdir().unwrap();
    let (bin, log) = fixture(root.path());
    assert!(
        invoke(root.path(), &bin, &log, &["init", "--non-interactive"], "")
            .status
            .success()
    );
    let raw = [
        "-c",
        "sandbox_mode=\"read-only\"",
        "-c",
        "approval_policy=\"never\"",
        "--json",
        "literal prompt $() `quoted` ; spaces",
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_astral"))
        .arg("--root")
        .arg(root.path())
        .args(["project", "--non-interactive", "--"])
        .args(raw)
        .env("ASTRAL_CODEX_BIN", &bin)
        .env("ASTRAL_TEST_LOG", &log)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not a Codex prompt\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: Vec<Value> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let argv = rows.iter().rfind(|r| r["kind"] == "argv").unwrap()["args"]
        .as_array()
        .unwrap();
    assert_eq!(
        &argv[..3],
        &[
            Value::from("exec"),
            Value::from("resume"),
            Value::from("01234567-89ab-cdef-0123-456789abcdef")
        ]
    );
    assert_eq!(&argv[3..], raw.map(Value::from));
    assert_eq!(
        rows.iter().find(|r| r["kind"] == "headless_stdin").unwrap()["value"],
        ""
    );
    let failed = invoke(
        root.path(),
        &bin,
        &log,
        &["project", "--non-interactive"],
        "fail",
    );
    assert_eq!(failed.status.code(), Some(23));
}

#[test]
fn cli_initialization_failures_keep_generated_files_and_propagate_exit_status() {
    for (mode, status, expected_error) in [
        ("fail", 23, None),
        ("noop", 2, Some("INITIALIZATION_INVALID")),
        ("invalid", 2, Some("INITIALIZATION_INVALID")),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (bin, log) = fixture(root.path());
        let out = invoke(
            root.path(),
            &bin,
            &log,
            &["init", "--non-interactive"],
            mode,
        );
        assert_eq!(out.status.code(), Some(status));
        if let Some(expected) = expected_error {
            assert_eq!(error(&out), expected);
        }
        if mode == "invalid" {
            assert_eq!(
                fs::read_to_string(root.path().join(".astral/project.toml")).unwrap(),
                "invalid schema"
            );
        }
    }
}

#[test]
fn missing_index_does_not_prompt_or_launch_without_a_terminal() {
    let root = tempfile::tempdir().unwrap();
    let (bin, log) = fixture(root.path());
    let out = invoke(root.path(), &bin, &log, &["project"], "");
    assert_eq!(error(&out), "PROJECT_NOT_INITIALIZED");
    assert!(!log.exists());
    assert!(!root.path().join(".astral").exists());
}

#[test]
fn inspect_initializer_is_read_only_and_existing_index_is_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let (bin, log) = fixture(root.path());
    let out = invoke(
        root.path(),
        &bin,
        &log,
        &[
            "init",
            "--inspect",
            "--non-interactive",
            "--",
            "-c",
            "literal=\"--inspect\"",
        ],
        "",
    );
    assert!(out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["prompt_version"], "project-init-v1");
    assert_eq!(value["non_interactive"], true);
    assert_eq!(
        value["launch_request"]["codex_args"][1],
        "literal=\"--inspect\""
    );
    assert!(!log.exists());
    fs::create_dir(root.path().join(".astral")).unwrap();
    let out = invoke(root.path(), &bin, &log, &["init"], "");
    assert_eq!(error(&out), "PROJECT_ALREADY_INITIALIZED");
    assert!(!log.exists());
}

#[test]
fn invalid_native_reference_and_conflicting_session_selector_do_not_launch() {
    let root = tempfile::tempdir().unwrap();
    let (bin, log) = fixture(root.path());
    assert!(
        invoke(root.path(), &bin, &log, &["init", "--non-interactive"], "")
            .status
            .success()
    );
    let before = fs::read(&log).unwrap();
    let out = invoke(root.path(), &bin, &log, &["project", "--", "--last"], "");
    assert_eq!(error(&out), "UNSUPPORTED_LAUNCH_ARGUMENT");
    assert_eq!(fs::read(&log).unwrap(), before);
    let manifest = root
        .path()
        .join(".astral/projections/initial-project-context/projection.toml");
    let old = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        old.replace(
            "native_payload_in_repository = false",
            "native_payload_in_repository = true",
        ),
    )
    .unwrap();
    let out = invoke(root.path(), &bin, &log, &["project"], "");
    assert_eq!(error(&out), "INVALID_NATIVE_REFERENCE");
    assert_eq!(fs::read(&log).unwrap(), before);
}

fn native_fixture(root: &Path, bin: &Path, log: &Path) {
    assert!(
        invoke(root, bin, log, &["init", "--non-interactive"], "")
            .status
            .success()
    );
    let projection = root.join(".astral/projections/initial-project-context");
    let bundle = projection.join("bundles/demo");
    fs::create_dir_all(&bundle).unwrap();
    let payload = br#"[{"type":"compaction","encrypted_content":"SYNTHETIC_NATIVE_OPAQUE"},{"type":"message","role":"user","content":[{"type":"input_text","text":"HISTORICAL_NATIVE_TAIL"}]}]"#;
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"format":"astral-codex-native",
        "payload":{"file":"window.json","sha256":ostk_gpt_cache::hash(payload),"bytes":payload.len(),"item_count":2},
        "compatibility":{"runtime":"codex","runtime_version":"0.154.0","protocol":"openai-responses-lite","provider":"openai","model":"gpt-6-astra","requires_tool_rebinding":true,"identity_scope":"same-account"},
        "source":{"project_id":"project","revision":null,"dirty":false,"selection_sha256":"a".repeat(64),"history_sha256":"b".repeat(64)},
        "capture":{"boundary":"completed-turn","history_complete":true,"last_checkpoint_index":0},"parents":[]
    })).unwrap();
    fs::write(bundle.join("manifest.json"), &manifest).unwrap();
    fs::write(bundle.join("window.json"), payload).unwrap();
    fs::write(projection.join("projection.toml"), format!("schema_version=1\nid='initial-project-context'\nkind='native-checkpoint'\nsubsystems=['project-context']\nhandoff='handoff.md'\nnative_payload_in_repository=true\nsources=[]\n[native_bundle]\nmanifest='bundles/demo/manifest.json'\nsha256='{}'\n", ostk_gpt_cache::hash(&manifest))).unwrap();
}

fn invoke_native(
    root: &Path,
    bin: &Path,
    log: &Path,
    state: &Path,
    args: &[&str],
    mode: &str,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_astral"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env("ASTRAL_CODEX_BIN", bin)
        .env("ASTRAL_TEST_LOG", log)
        .env("ASTRAL_TEST_MODE", mode)
        .env("ASTRAL_LAUNCH_STATE_DIR", state)
        .output()
        .unwrap()
}

fn receipt(state: &Path) -> (String, Value) {
    let entries: Vec<_> = fs::read_dir(state).unwrap().map(|e| e.unwrap()).collect();
    assert_eq!(entries.len(), 1);
    let directory = entries[0].path();
    let id = entries[0].file_name().into_string().unwrap();
    let receipt: Value =
        serde_json::from_slice(&fs::read(directory.join("receipt.json")).unwrap()).unwrap();
    (id, receipt)
}

fn native_rows(log: &Path) -> Vec<Value> {
    fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_owned_proxy_closed(rows: &[Value]) {
    let argv = rows
        .iter()
        .rfind(|r| r["kind"] == "argv" && r["args"][0] == "app-server")
        .unwrap()["args"]
        .as_array()
        .unwrap();
    let value = argv
        .iter()
        .filter_map(Value::as_str)
        .find_map(|arg| arg.strip_prefix("openai_base_url="))
        .unwrap();
    let url: String = serde_json::from_str(value).unwrap();
    let url = reqwest::Url::parse(&url).unwrap();
    let address = format!("127.0.0.1:{}", url.port().unwrap());
    assert!(std::net::TcpStream::connect(address).is_err());
}

#[test]
fn native_cli_stages_once_resumes_same_receipt_and_closes_owned_proxy() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let root = parent.join("workspace");
    fs::create_dir(&root).unwrap();
    let (bin, log) = fixture(&root);
    native_fixture(&root, &bin, &log);
    let state = parent.join("private-launches");
    let before = fs::read(&log).unwrap();
    let direct = invoke_native(
        &root,
        &bin,
        &log,
        &state,
        &["project", "--non-interactive"],
        "",
    );
    assert_eq!(error(&direct), "NATIVE_PROXY_REQUIRED");
    assert_eq!(fs::read(&log).unwrap(), before);
    assert!(!state.exists());
    let raw = [
        "-c",
        "sandbox_mode=\"read-only\"",
        "-c",
        "approval_policy=\"never\"",
        "literal $() ; prompt",
    ];
    let mut args = vec!["project", "--proxy", "--non-interactive", "--"];
    args.extend(raw);
    let out = invoke_native(&root, &bin, &log, &state, &args, "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (id, first) = receipt(&state);
    assert_eq!(first["status"], "finished");
    assert_eq!(first["exit_code"], 0);
    let first_rows = native_rows(&log);
    let injected: Vec<_> = first_rows
        .iter()
        .filter(|r| r["request"]["method"] == "thread/inject_items")
        .collect();
    assert_eq!(injected.len(), 1);
    let items = injected[0]["request"]["params"]["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["encrypted_content"], "SYNTHETIC_NATIVE_OPAQUE");
    let context = items[2]["content"][0]["text"].as_str().unwrap();
    assert!(context.contains("SYNTHETIC_DOC"));
    assert!(!context.contains("SYNTHETIC_NATIVE_OPAQUE"));
    let argv = first_rows.iter().rfind(|r| r["kind"] == "argv").unwrap()["args"]
        .as_array()
        .unwrap();
    assert_eq!(
        &argv[..3],
        &[
            Value::from("exec"),
            Value::from("resume"),
            first["thread_id"].clone()
        ]
    );
    assert_eq!(&argv[7..], raw.map(Value::from));
    assert_owned_proxy_closed(&first_rows);
    let resumed = invoke_native(
        &root,
        &bin,
        &log,
        &state,
        &[
            "project",
            "--proxy",
            "--non-interactive",
            "--resume",
            &id,
            "--",
            "-c",
            "sandbox_mode=\"read-only\"",
            "next prompt",
        ],
        "fail",
    );
    assert_eq!(
        resumed.status.code(),
        Some(23),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let (_, second) = receipt(&state);
    assert_eq!(second["thread_id"], first["thread_id"]);
    assert_eq!(second["exit_code"], 23);
    let rows = native_rows(&log);
    assert_eq!(
        rows.iter()
            .filter(|r| r["request"]["method"] == "thread/inject_items")
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r["request"]["method"] == "thread/resume")
            .count(),
        1
    );
    assert_owned_proxy_closed(&rows);
}

#[test]
fn native_staging_failure_records_receipt_and_stops_proxy_without_child_launch() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let root = parent.join("workspace");
    fs::create_dir(&root).unwrap();
    let (bin, log) = fixture(&root);
    native_fixture(&root, &bin, &log);
    let state = parent.join("private-launches");
    let out = invoke_native(
        &root,
        &bin,
        &log,
        &state,
        &["project", "--proxy", "--non-interactive"],
        "native-stage-fail",
    );
    assert!(!out.status.success());
    let (_, recorded) = receipt(&state);
    assert_eq!(recorded["status"], "failed");
    assert!(recorded["thread_id"].is_null());
    let rows = native_rows(&log);
    assert!(!rows.iter().any(|r| r["kind"] == "resumed"));
    assert_owned_proxy_closed(&rows);
}
