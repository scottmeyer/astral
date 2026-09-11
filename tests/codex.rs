#![cfg(unix)]

use ostk_gpt_cache::codex::{StagingOptions, stage_fresh};
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const THREAD_ID: &str = "01a10000-1234-7000-8000-000000000001";

#[tokio::test]
async fn cancelling_the_owned_child_wait_terminates_its_process() {
    use std::time::Duration;
    let directory = TempDir::new().unwrap();
    let marker = directory.path().join("pid");
    let mut command = tokio::process::Command::new("/bin/sh");
    command
        .args([
            "-c",
            "printf '%s' \"$$\" > \"$1\"; exec sleep 60",
            "fixture",
        ])
        .arg(&marker);
    let task = tokio::spawn(ostk_gpt_cache::codex::wait_interactive(command));
    let pid: i32 = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(value) = fs::read_to_string(&marker) {
                if let Ok(pid) = value.parse() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), async {
        // SAFETY: signal zero only tests this fixture's recorded process ID.
        while unsafe { libc::kill(pid, 0) } == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("cancelled owned child remained alive");
}

// This executable implements only the three staging requests. In particular,
// it records and fails any attempt to start a model turn or execute a tool.
const SERVER: &str = r#"#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

root = Path(__file__).resolve().parent
mode = (root / "mode").read_text()
(root / "argv.json").write_text(json.dumps(sys.argv[1:]))
(root / "cwd").write_text(os.getcwd())
log = (root / "requests.jsonl").open("w", buffering=1)

def emit(value):
    print(json.dumps(value), flush=True)

for line in sys.stdin:
    request = json.loads(line)
    log.write(json.dumps(request) + "\n")
    method = request.get("method")
    if method == "initialized":
        continue
    if method not in ("initialize", "thread/start", "thread/inject_items"):
        (root / "unexpected-operation").write_text(method or "missing method")
        sys.exit(91)
    if method == "initialize":
        if mode == "early-eof":
            sys.exit(0)
        if mode == "malformed-json":
            print("{malformed", flush=True)
            continue
        if mode == "oversized-response":
            print("x" * (4 * 1024 * 1024 + 1), flush=True)
            continue
        if mode == "server-request":
            emit({"id": 99, "method": "item/commandExecution/requestApproval", "params": {}})
            continue
        if mode == "unexpected-id":
            emit({"id": request["id"] + 1, "result": {}})
            continue
        if mode == "provider-error":
            emit({"id": request["id"], "error": {"code": -32000, "message": "PRIVATE_PROVIDER_BODY_SENTINEL"}})
            continue
        if mode == "notification-limit":
            for _ in range(4096):
                emit({"method": "fixture/notification", "params": {}})
        emit({"method": "fixture/notification", "params": {"note": "notification before result"}})
        result = {"userAgent": "fixture/0.154.0"}
    elif method == "thread/start":
        result = {"thread": {"id": "01a10000-1234-7000-8000-000000000001", "ephemeral": False}, "cwd": str(root)}
        if mode == "invalid-thread-id":
            result["thread"]["id"] = "../../not-a-thread"
        elif mode == "missing-thread-id":
            del result["thread"]["id"]
        elif mode == "ephemeral":
            result["thread"]["ephemeral"] = True
        elif mode == "different-cwd":
            result["cwd"] = str(root / "other")
        elif mode == "missing-cwd":
            del result["cwd"]
    else:
        result = {}
    emit({"id": request["id"], "result": result})

log.close()
(root / "clean-eof").write_text("closed")
sys.exit(7 if mode == "nonzero-exit" else 0)
"#;

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    executable: PathBuf,
}

impl Fixture {
    fn new(mode: &str) -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let executable = root.join("fake-codex");
        fs::write(&executable, SERVER).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(root.join("mode"), mode).unwrap();
        fs::create_dir(root.join("other")).unwrap();
        Self {
            _temp: temp,
            root,
            executable,
        }
    }

    fn requests(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join("requests.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn assert_no_execution(&self) {
        assert!(!self.root.join("unexpected-operation").exists());
        assert!(self.requests().iter().all(|request| matches!(
            request["method"].as_str(),
            Some("initialize" | "initialized" | "thread/start" | "thread/inject_items")
        )));
    }
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn options(root: &Path, args: &[&str]) -> StagingOptions {
    StagingOptions::from_args(root, &os(args)).unwrap()
}

#[tokio::test]
async fn subprocess_stages_exact_user_context_and_exits_before_returning() {
    let fixture = Fixture::new("success");
    let context = "Project context\nUnicode: λ 🦀\nLiteral $(touch nope), `pwd`, \\\"quoted\\\"\n";
    let raw_args = os(&[
        "--model",
        "fixture-model",
        "-s",
        "read-only",
        "-a",
        "never",
        "-c",
        "model_reasoning_effort=\"xhigh\"",
        "--enable",
        "fixture_feature",
        "--disable=other_feature",
        "--strict-config",
        "a separate user prompt",
    ]);
    let original_args = raw_args.clone();
    let opts = StagingOptions::from_args(&fixture.root, &raw_args).unwrap();
    let expected_server_args: Vec<_> = opts
        .server_args
        .iter()
        .map(|arg| arg.to_str().unwrap())
        .collect();
    let expected_server_args = json!(expected_server_args);
    let expected_thread_params = opts.thread_params.clone();
    let id = stage_fresh(fixture.executable.as_os_str(), &fixture.root, opts, context)
        .await
        .unwrap();

    assert_eq!(id, THREAD_ID);
    assert_eq!(raw_args, original_args);
    assert_eq!(
        fs::read_to_string(fixture.root.join("clean-eof")).unwrap(),
        "closed"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("cwd")).unwrap(),
        fixture.root.to_str().unwrap()
    );
    let actual_argv: Value =
        serde_json::from_slice(&fs::read(fixture.root.join("argv.json")).unwrap()).unwrap();
    assert_eq!(actual_argv, expected_server_args);

    let requests = fixture.requests();
    let methods: Vec<_> = requests
        .iter()
        .map(|request| request["method"].as_str().unwrap())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "thread/start",
            "thread/inject_items"
        ]
    );
    assert_eq!(requests[0]["params"]["clientInfo"]["name"], "astral");
    assert_eq!(requests[2]["params"], expected_thread_params);
    assert_eq!(
        requests[3]["params"],
        json!({
            "threadId": THREAD_ID,
            "items": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": context}]}]
        })
    );
    fixture.assert_no_execution();
}

#[tokio::test]
async fn protocol_failures_stop_staging_and_release_the_owned_process() {
    for (mode, code) in [
        ("malformed-json", "CODEX_PROTOCOL"),
        ("oversized-response", "LIMIT_EXCEEDED"),
        ("server-request", "CODEX_UNEXPECTED_REQUEST"),
        ("unexpected-id", "CODEX_PROTOCOL"),
        ("provider-error", "CODEX_REQUEST_FAILED"),
        ("notification-limit", "LIMIT_EXCEEDED"),
        ("invalid-thread-id", "CODEX_PROTOCOL"),
        ("missing-thread-id", "CODEX_PROTOCOL"),
        ("different-cwd", "WORKSPACE_CONFLICT"),
        ("missing-cwd", "CODEX_PROTOCOL"),
    ] {
        let fixture = Fixture::new(mode);
        let err = stage_fresh(
            fixture.executable.as_os_str(),
            &fixture.root,
            options(&fixture.root, &[]),
            "must not reach injection",
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, code, "{mode}: {err:?}");
        assert!(!err.message.contains("PRIVATE_PROVIDER_BODY_SENTINEL"));
        assert!(
            fixture.root.join("clean-eof").exists(),
            "{mode}: process did not consume EOF"
        );
        assert!(
            !fixture
                .requests()
                .iter()
                .any(|request| request["method"] == "thread/inject_items"),
            "{mode}: injected after failure"
        );
        fixture.assert_no_execution();
    }
}

#[tokio::test]
async fn ephemeral_threads_cannot_be_returned_as_durable_resume_targets() {
    let fixture = Fixture::new("ephemeral");
    let result = stage_fresh(
        fixture.executable.as_os_str(),
        &fixture.root,
        options(&fixture.root, &[]),
        "context",
    )
    .await;
    assert!(
        result.is_err(),
        "ephemeral thread accepted as a resume target"
    );
    assert!(
        !fixture
            .requests()
            .iter()
            .any(|request| request["method"] == "thread/inject_items")
    );
    assert!(fixture.root.join("clean-eof").exists());
    fixture.assert_no_execution();
}

#[tokio::test]
async fn incomplete_startup_and_unsuccessful_shutdown_are_errors() {
    for (mode, code) in [
        ("early-eof", "CODEX_PROTOCOL"),
        ("nonzero-exit", "CODEX_SHUTDOWN_FAILED"),
    ] {
        let fixture = Fixture::new(mode);
        let err = stage_fresh(
            fixture.executable.as_os_str(),
            &fixture.root,
            options(&fixture.root, &[]),
            "context",
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, code, "{mode}: {err:?}");
        fixture.assert_no_execution();
    }
}

#[tokio::test]
async fn oversized_context_is_not_sent_to_the_child() {
    let fixture = Fixture::new("success");
    let err = stage_fresh(
        fixture.executable.as_os_str(),
        &fixture.root,
        options(&fixture.root, &[]),
        &"x".repeat(4 * 1024 * 1024),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "LIMIT_EXCEEDED");
    assert!(
        !fixture
            .requests()
            .iter()
            .any(|request| request["method"] == "thread/inject_items")
    );
    assert!(fixture.root.join("clean-eof").exists());
    fixture.assert_no_execution();
}

#[tokio::test]
async fn unavailable_executable_is_reported_without_starting_a_session() {
    let fixture = Fixture::new("success");
    let err = stage_fresh(
        fixture.root.join("does-not-exist").as_os_str(),
        &fixture.root,
        options(&fixture.root, &[]),
        "context",
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, "CODEX_UNAVAILABLE");
    assert!(!fixture.root.join("requests.jsonl").exists());
}

#[test]
fn staging_does_not_supply_default_model_instructions_or_permissions() {
    let fixture = Fixture::new("success");
    let opts = options(&fixture.root, &[]);
    assert_eq!(
        opts.server_args,
        os(&["app-server", "--listen", "stdio://"])
    );
    assert_eq!(opts.thread_params["cwd"], fixture.root.to_str().unwrap());
    for field in [
        "model",
        "modelProvider",
        "sandbox",
        "approvalPolicy",
        "approvalsReviewer",
        "baseInstructions",
        "developerInstructions",
        "dynamicTools",
    ] {
        assert!(
            opts.thread_params.get(field).is_none(),
            "unexpected default {field}"
        );
    }
}

#[test]
fn runtime_values_support_long_short_attached_and_equals_forms() {
    let fixture = Fixture::new("success");
    for args in [
        vec![
            "--model",
            "fixture-model",
            "--sandbox",
            "read-only",
            "--ask-for-approval",
            "never",
        ],
        vec!["-m", "fixture-model", "-s", "read-only", "-a", "never"],
        vec![
            "--model=fixture-model",
            "--sandbox=read-only",
            "--ask-for-approval=never",
        ],
        vec!["-mfixture-model", "-sread-only", "-anever"],
        vec!["-m=fixture-model", "-s=read-only", "-a=never"],
    ] {
        let opts = options(&fixture.root, &args);
        assert_eq!(opts.thread_params["model"], "fixture-model", "{args:?}");
        assert_eq!(opts.thread_params["sandbox"], "read-only", "{args:?}");
        assert_eq!(opts.thread_params["approvalPolicy"], "never", "{args:?}");
    }
}

#[test]
fn raw_config_values_are_copied_without_shell_interpretation() {
    let fixture = Fixture::new("success");
    let value = "developer_instructions=\"literal $(touch nope) `pwd`\\ntext\"";
    for args in [
        os(&["-c", value]),
        os(&["--config", value]),
        os(&[&format!("--config={value}")]),
        os(&[&format!("-c{value}")]),
        os(&[&format!("-c={value}")]),
    ] {
        let opts = StagingOptions::from_args(&fixture.root, &args).unwrap();
        assert!(
            opts.server_args.iter().any(|arg| arg == OsStr::new(value)),
            "{args:?}"
        );
        assert!(!fixture.root.join("nope").exists());
    }
}

#[test]
fn explicit_permission_aliases_match_codex_semantics() {
    let fixture = Fixture::new("success");
    for bypass in ["--dangerously-bypass-approvals-and-sandbox", "--yolo"] {
        for args in [
            vec![bypass],
            vec![bypass, "-s", "read-only"],
            vec!["-s", "read-only", bypass],
        ] {
            let opts = options(&fixture.root, &args);
            assert_eq!(
                opts.thread_params["sandbox"], "danger-full-access",
                "{args:?}"
            );
            assert_eq!(opts.thread_params["approvalPolicy"], "never", "{args:?}");
        }
    }
    for automatic in ["--approve-for-me", "--not-so-yolo"] {
        let opts = options(&fixture.root, &[automatic]);
        // Codex's own CLI expands this flag to these three config overrides.
        for setting in [
            "approvals_reviewer=\"auto_review\"",
            "approval_policy=\"on-request\"",
            "sandbox_mode=\"workspace-write\"",
        ] {
            assert!(
                opts.server_args
                    .windows(2)
                    .any(|pair| pair == os(&["-c", setting]))
            );
        }
    }
}

#[test]
fn explicit_search_and_hook_trust_flags_are_translated_without_defaults() {
    let fixture = Fixture::new("success");
    let opts = options(
        &fixture.root,
        &["--search", "--dangerously-bypass-hook-trust"],
    );
    assert!(
        opts.server_args
            .windows(2)
            .any(|pair| pair == os(&["-c", "web_search=\"live\""]))
    );
    assert_eq!(opts.thread_params["config"]["bypass_hook_trust"], true);
    assert!(
        options(&fixture.root, &[])
            .thread_params
            .get("config")
            .is_none()
    );
    assert!(options(&fixture.root, &[]).server_args.iter().all(|arg| {
        !arg.to_string_lossy().starts_with("web_search=")
            && !arg.to_string_lossy().starts_with("bypass_hook_trust=")
    }));
}

#[test]
fn permission_conflicts_and_invalid_modes_are_rejected_before_staging() {
    let fixture = Fixture::new("success");
    for args in [
        vec!["--yolo", "-a", "never"],
        vec!["--approve-for-me", "-a", "on-request"],
        vec!["--not-so-yolo", "-s", "read-only"],
        vec!["--approve-for-me", "--yolo"],
        vec!["-s", "invalid-sandbox"],
        vec!["-a", "untrusted"],
    ] {
        assert!(
            StagingOptions::from_args(&fixture.root, &os(&args)).is_err(),
            "accepted {args:?}"
        );
    }
}

#[test]
fn unsupported_session_routing_and_profile_flags_are_rejected() {
    let fixture = Fixture::new("success");
    for args in [
        vec!["--last"],
        vec!["--remote=ws://example.invalid"],
        vec!["--remote-auth-token-env", "TOKEN"],
        vec!["--worktree"],
        vec!["--oss"],
        vec!["--local-provider", "ollama"],
        vec!["-p", "work"],
        vec!["-pwork"],
        vec!["--profile=work"],
    ] {
        let err = StagingOptions::from_args(&fixture.root, &os(&args)).unwrap_err();
        assert_eq!(err.code, "UNSUPPORTED_LAUNCH_ARGUMENT", "{args:?}");
    }
}

#[test]
fn selected_workspace_cannot_be_redirected_by_codex_arguments() {
    let fixture = Fixture::new("success");
    options(&fixture.root, &["-C", "."]);
    options(&fixture.root, &["--cd", fixture.root.to_str().unwrap()]);
    for args in [
        vec!["--cd", "other"],
        vec!["-Cother"],
        vec!["--cd=missing-directory"],
    ] {
        let err = StagingOptions::from_args(&fixture.root, &os(&args)).unwrap_err();
        assert_eq!(err.code, "WORKSPACE_CONFLICT", "{args:?}");
    }
    for config in [
        "cwd=\"other\"",
        "profile=\"work\"",
        "sqlite_home=\"other\"",
        "codex_home=\"other\"",
    ] {
        let err = StagingOptions::from_args(&fixture.root, &os(&["-c", config])).unwrap_err();
        assert_eq!(err.code, "UNSUPPORTED_LAUNCH_ARGUMENT", "{config}");
    }
}

#[test]
fn literal_separator_and_opaque_prompt_bytes_are_not_interpreted_as_settings() {
    let fixture = Fixture::new("success");
    let mut args = os(&["--", "--last", "--yolo", "--profile=work", "-m=other"]);
    args.push(OsString::from_vec(vec![b'p', 0xff]));
    let original = args.clone();
    let opts = StagingOptions::from_args(&fixture.root, &args).unwrap();
    assert_eq!(args, original);
    assert_eq!(
        opts.server_args,
        os(&["app-server", "--listen", "stdio://"])
    );
    for field in ["model", "sandbox", "approvalPolicy"] {
        assert!(opts.thread_params.get(field).is_none());
    }
}

#[test]
fn value_taking_options_cannot_consume_a_literal_separator() {
    let fixture = Fixture::new("success");
    for arg in ["-c", "--enable", "-m", "-s", "-a", "-C", "-i"] {
        for args in [vec![arg], vec![arg, "--", "value"]] {
            assert!(
                StagingOptions::from_args(&fixture.root, &os(&args)).is_err(),
                "accepted {args:?}"
            );
        }
    }
}

#[test]
fn initialization_rejects_extra_prompts_and_subcommands() {
    let fixture = Fixture::new("success");
    for args in [
        vec!["an additional user prompt"],
        vec!["exec", "a different task"],
        vec!["resume", THREAD_ID, "a different task"],
        vec!["fork", THREAD_ID],
        vec!["review"],
        vec!["--model", "fixture-model", "exec"],
        vec!["--", "exec"],
        vec!["--", "--yolo"],
    ] {
        let err = StagingOptions::validate_initialization(&fixture.root, &os(&args)).unwrap_err();
        assert_eq!(err.code, "CLI_USAGE", "{args:?}");
    }
    assert!(!fixture.root.join("requests.jsonl").exists());
}

#[test]
fn initialization_accepts_direct_runtime_choices_and_consumes_option_values() {
    let fixture = Fixture::new("success");
    for args in [
        vec!["-p", "work"],
        vec!["-pwork"],
        vec!["--profile=work"],
        vec!["--oss", "--local-provider", "ollama"],
        vec!["--add-dir", "other"],
        vec!["--model", "exec"],
        vec!["--output-last-message", "resume"],
        vec!["-o=review", "--output-schema", "fork"],
        vec!["--color", "never", "--thread-source", "user"],
        vec![
            "--image",
            "first.png",
            "second.png",
            "--model",
            "fixture-model",
        ],
        vec!["--"],
    ] {
        StagingOptions::validate_initialization(&fixture.root, &os(&args))
            .unwrap_or_else(|err| panic!("rejected option values {args:?}: {err:?}"));
    }
}

#[test]
fn boolean_flag_values_cannot_be_misread_as_enabled_flags() {
    let fixture = Fixture::new("success");
    for flag in [
        "--strict-config",
        "--dangerously-bypass-approvals-and-sandbox",
        "--yolo",
        "--approve-for-me",
        "--not-so-yolo",
        "--search",
        "--dangerously-bypass-hook-trust",
    ] {
        for value in ["false", "true", ""] {
            let args = os(&[&format!("{flag}={value}")]);
            assert_eq!(
                StagingOptions::from_args(&fixture.root, &args)
                    .unwrap_err()
                    .code,
                "CLI_USAGE",
                "{args:?}"
            );
            assert_eq!(
                StagingOptions::validate_initialization(&fixture.root, &args)
                    .unwrap_err()
                    .code,
                "CLI_USAGE",
                "{args:?}"
            );
        }
    }
}

#[test]
fn attached_and_separate_image_paths_preserve_opaque_os_bytes() {
    let fixture = Fixture::new("success");
    let opaque = vec![b'i', b'm', b'g', 0xff, b'.', b'p', b'n', b'g'];
    let mut cases = vec![vec!["--image".into(), OsString::from_vec(opaque.clone())]];
    for prefix in [b"-i".as_slice(), b"-i=".as_slice(), b"--image=".as_slice()] {
        let mut argument = prefix.to_vec();
        argument.extend(&opaque);
        cases.push(vec![OsString::from_vec(argument)]);
    }
    for args in cases {
        let original = args.clone();
        let opts = StagingOptions::from_args(&fixture.root, &args)
            .unwrap_or_else(|err| panic!("rejected opaque image argument {args:?}: {err:?}"));
        StagingOptions::validate_initialization(&fixture.root, &args)
            .unwrap_or_else(|err| panic!("rejected opaque initializer image {args:?}: {err:?}"));
        assert_eq!(args, original);
        assert_eq!(
            opts.server_args,
            os(&["app-server", "--listen", "stdio://"])
        );
        assert!(opts.thread_params.get("model").is_none());
        assert!(opts.thread_params.get("approvalPolicy").is_none());
    }
}
