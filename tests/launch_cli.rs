#![cfg(unix)]

use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

const FAKE: &str = r#"#!/usr/bin/env python3
import json, os, pathlib, re, sys
root = pathlib.Path.cwd()
log = pathlib.Path(os.environ['ASTRAL_TEST_LOG'])
def record(kind, **values):
    with log.open('a') as f: f.write(json.dumps(dict(kind=kind, **values)) + '\n')
record('argv', args=sys.argv[1:], cwd=str(root))
mode = os.environ.get('ASTRAL_TEST_MODE', '')
if sys.argv[1] == 'app-server':
    for line in sys.stdin:
        request = json.loads(line)
        record('rpc', request=request)
        if 'id' not in request: continue
        result = {}
        if request['method'] == 'thread/start':
            result = dict(thread=dict(id='01234567-89ab-cdef-0123-456789abcdef', ephemeral=False), cwd=str(root))
        print(json.dumps(dict(id=request['id'], result=result)), flush=True)
    record('closed')
elif sys.argv[1] == 'resume':
    assert any(json.loads(row)['kind'] == 'closed' for row in log.read_text().splitlines())
    record('resumed')
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
