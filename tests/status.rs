#![cfg(unix)]

use astral::{
    project::Project,
    status,
    workspace::{WorkerMetadata, WorktreeBinding},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const WORK: &str = "AST-001";
const THREAD: &str = "01a10000-1234-7000-8000-000000000001";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn put(root: &Path, name: &str, body: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_AUTHOR_NAME", "Status Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Status Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("repo");
        fs::create_dir(&root).unwrap();
        put(
            &root,
            ".astral/project.toml",
            r#"schema_version = 1
id = "fixture"
name = "Fixture"
description = "Status test"
core = "core"
projections = "projections"
work_items = "work/items.jsonl"
[identity]
scope = "repository"
runtime_bindings = "private"
[subsystems]
web = "core/web"
"#,
        );
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            put(
                &root,
                &format!(".astral/core/{name}"),
                "Fixture documents.\n",
            );
        }
        put(
            &root,
            ".astral/core/web/subsystem.toml",
            r#"schema_version = 1
id = "web"
purpose = "Fixture"
readme = "README.md"
rules = []
decisions = []
work_items = []
projection = "seed"
depends_on = []
"#,
        );
        put(&root, ".astral/core/web/README.md", "Web context.\n");
        put(
            &root,
            ".astral/projections/seed/projection.toml",
            r#"schema_version = 1
id = "seed"
kind = "fresh-context"
subsystems = ["web"]
handoff = "handoff.md"
native_payload_in_repository = false
sources = []
"#,
        );
        put(
            &root,
            ".astral/projections/seed/handoff.md",
            "Fixture handoff.\n",
        );
        let records: String = [WORK,"AST-002","AST-003"].iter().map(|id| format!("{}\n", json!({"schema_version":1,"id":id,"title":format!("Work {id}"),"status":"open","acceptance":["Observe only"],"depends_on":[]}))).collect();
        put(&root, ".astral/work/items.jsonl", &records);
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", ".astral"]);
        git(&root, &["commit", "-m", "fixture"]);
        Self { _temp: temp, root }
    }
    fn bind(&self) -> WorktreeBinding {
        let mut binding = WorktreeBinding::acquire(&self.root, "fixture", "web", WORK).unwrap();
        let context = Project::load(binding.root())
            .unwrap()
            .launch_context("web", Some(WORK))
            .unwrap();
        binding
            .update_worker_metadata(WorkerMetadata {
                context_initialized: true,
                thread_id: Some(THREAD.into()),
                model: Some("gpt-6-astra".into()),
                provider: Some("openai".into()),
                selection_digest: Some(context.current_context.selection_digest),
                selected_bundle_recorded: true,
                ..Default::default()
            })
            .unwrap();
        binding
    }
    fn report(&self) -> status::StatusReport {
        status::collect(&self.root, Some(WORK), 0, 32).unwrap()
    }
    fn cli(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_astral"))
            .current_dir(&self.root)
            .args(args)
            .env("ASTRAL_CODEX_BIN", self.root.join("must-not-run-codex"))
            .env_remove("ASTRAL_WORKTREE_ROOT")
            .output()
            .unwrap()
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), out);
            } else {
                out.insert(
                    entry.path().strip_prefix(root).unwrap().into(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

#[test]
fn unbound_status_is_read_only_and_cli_json_is_formatted() {
    let f = Fixture::new();
    let before = snapshot(&f.root);
    let output = f.cli(&["status", "--json"]);
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["total_work_items"], 3);
    assert_eq!(value["workers"][0]["state"], "unbound");
    assert!(value["workers"][0]["resume_argv"].is_null());
    assert!(String::from_utf8(output.stdout).unwrap().contains("\n  \""));
    assert_eq!(before, snapshot(&f.root));
    assert!(!f.root.join(".git/astral").exists());
    let human = f.cli(&["status", "--work", WORK]);
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .contains("open / unbound")
    );
    assert!(f.cli(&["doctor", "--work", WORK]).status.success());
}

#[test]
fn live_owner_is_not_an_uncertain_staging_failure() {
    let f = Fixture::new();
    let mut binding = f.bind();
    let mut metadata = binding.worker_metadata().clone();
    metadata.staging_in_progress = true;
    binding.update_worker_metadata(metadata).unwrap();
    let report = f.report();
    assert!(!report.needs_attention());
    assert_eq!(report.workers[0].state, "owned");
    assert!(report.workers[0].context.is_none());
    assert!(report.workers[0].resume_argv.is_none());
    drop(binding);
    let report = f.report();
    assert!(report.needs_attention());
    assert!(
        report.workers[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "WORKSPACE_STAGE_INCOMPLETE")
    );
    assert!(report.workers[0].resume_argv.is_none());
}

#[test]
fn recorded_thread_context_changes_and_literal_resume_suggestion() {
    let f = Fixture::new();
    let binding = f.bind();
    let worker_root = binding.root().to_owned();
    drop(binding);
    let before = snapshot(&f.root);
    let report = f.report();
    assert!(!report.needs_attention());
    let worker = &report.workers[0];
    assert_eq!(worker.state, "recorded");
    assert_eq!(
        worker.context.as_ref().unwrap().changed_since_staging,
        Some(false)
    );
    assert_eq!(
        worker.resume_argv.as_ref().unwrap(),
        &vec![
            "astral",
            "--root",
            worker_root.to_str().unwrap(),
            "project",
            "web",
            "--work",
            WORK
        ]
    );
    assert_eq!(before, snapshot(&f.root));
    put(
        &worker_root,
        ".astral/core/web/README.md",
        "Current changed web context.\n",
    );
    let report = f.report();
    assert!(!report.needs_attention());
    assert_eq!(
        report.workers[0]
            .context
            .as_ref()
            .unwrap()
            .changed_since_staging,
        Some(true)
    );
    // Changes in the invoking checkout do not replace the bound worker's context.
    put(
        &f.root,
        ".astral/core/web/README.md",
        "Different source context.\n",
    );
    assert_eq!(
        report.workers[0]
            .context
            .as_ref()
            .unwrap()
            .current_selection_digest,
        f.report().workers[0]
            .context
            .as_ref()
            .unwrap()
            .current_selection_digest
    );
}

#[test]
fn observed_native_requirement_is_carried_into_the_suggestion() {
    let f = Fixture::new();
    let mut binding = f.bind();
    let mut metadata = binding.worker_metadata().clone();
    metadata.requires_tool_rebinding = true;
    metadata.saved_bundle_sha256 = Some("a".repeat(64));
    binding.update_worker_metadata(metadata).unwrap();
    drop(binding);
    let report = f.report();
    assert!(report.workers[0].context.as_ref().unwrap().requires_proxy);
    assert_eq!(
        report.workers[0]
            .resume_argv
            .as_ref()
            .unwrap()
            .last()
            .unwrap(),
        "--proxy"
    );
    assert_eq!(
        report.workers[0]
            .context
            .as_ref()
            .unwrap()
            .recorded_saved_bundle_sha256,
        Some("a".repeat(64))
    );
}

#[test]
fn changed_selected_native_anchor_blocks_a_continuation_suggestion() {
    let f = Fixture::new();
    let mut binding = f.bind();
    let mut metadata = binding.worker_metadata().clone();
    // The selected context is fresh, while the receipt anchors another history.
    metadata.selected_bundle_sha256 = Some("b".repeat(64));
    binding.update_worker_metadata(metadata).unwrap();
    drop(binding);
    let report = f.report();
    assert!(report.needs_attention());
    assert!(
        report.workers[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "WORKSPACE_SEED_CHANGED")
    );
    assert!(report.workers[0].resume_argv.is_none());
}

#[test]
fn doctor_reports_context_and_git_binding_failures_without_repair() {
    let f = Fixture::new();
    let binding = f.bind();
    let worker_root = binding.root().to_owned();
    drop(binding);
    put(&worker_root, ".astral/project.toml", "malformed = [");
    let before = snapshot(&f.root);
    let output = f.cli(&["doctor", "--work", WORK, "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["workers"][0]["state"], "attention");
    assert!(value["workers"][0]["resume_argv"].is_null());
    assert_eq!(before, snapshot(&f.root));
    git(&worker_root, &["switch", "-c", "other"]);
    let report = f.report();
    assert!(
        report.workers[0]
            .diagnostics
            .iter()
            .any(|d| d.code == "WORKSPACE_BRANCH_MISMATCH")
    );
}

#[test]
fn pagination_unknown_work_and_invalid_project_are_explicit() {
    let f = Fixture::new();
    let page = status::collect(&f.root, None, 0, 2).unwrap();
    assert_eq!(page.workers.len(), 2);
    assert_eq!(page.next_offset, Some(2));
    let last = status::collect(&f.root, None, 2, 2).unwrap();
    assert_eq!(last.workers[0].work_id, "AST-003");
    assert_eq!(last.next_offset, None);
    assert!(
        status::collect(&f.root, None, 99, 2)
            .unwrap()
            .workers
            .is_empty()
    );
    for limit in [0, 65, usize::MAX] {
        assert!(status::collect(&f.root, None, 0, limit).is_err());
    }
    assert!(status::collect(&f.root, Some(WORK), 1, 32).is_err());
    assert_eq!(
        f.cli(&["doctor", "--work", "missing", "--json"])
            .status
            .code(),
        Some(1)
    );
    put(&f.root, ".astral/project.toml", "bad TOML");
    let output = f.cli(&["doctor", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!value["diagnostics"].as_array().unwrap().is_empty());
    assert!(value["workers"].as_array().unwrap().is_empty());
}

#[test]
fn human_output_escapes_terminal_control_characters() {
    let f = Fixture::new();
    let mut report = f.report();
    report.workers[0].title = "\x1b[2J\nforged terminal line".into();
    report.workers[0].resume_argv = Some(vec!["astral".into(), "path\u{202e}\u{0085}".into()]);
    let text = status::render(&report);
    assert!(!text.contains('\x1b'));
    assert!(!text.contains("\nforged"));
    assert!(text.contains("\\nforged"));
    assert!(!text.contains('\u{202e}') && !text.contains('\u{0085}'));
}

#[test]
fn no_staging_baseline_is_unknown_and_native_runtime_metadata_is_required() {
    let f = Fixture::new();
    let mut binding = f.bind();
    let mut metadata = binding.worker_metadata().clone();
    metadata.thread_id = None;
    metadata.selection_digest = None;
    metadata.model = None;
    metadata.provider = None;
    binding.update_worker_metadata(metadata.clone()).unwrap();
    drop(binding);
    let report = f.report();
    assert!(!report.needs_attention());
    assert_eq!(report.workers[0].state, "not_staged");
    assert_eq!(
        report.workers[0]
            .context
            .as_ref()
            .unwrap()
            .changed_since_staging,
        None
    );
    assert!(status::render(&report).contains("context change is unknown"));
    for missing_model in [false, true] {
        let mut binding = WorktreeBinding::open_existing(&f.root, "fixture", "web", WORK).unwrap();
        metadata.thread_id = Some(THREAD.into());
        metadata.requires_tool_rebinding = true;
        metadata.model = if missing_model {
            None
        } else {
            Some("gpt-6-astra".into())
        };
        metadata.provider = if missing_model {
            Some("openai".into())
        } else {
            None
        };
        binding.update_worker_metadata(metadata.clone()).unwrap();
        drop(binding);
        let report = f.report();
        assert!(report.needs_attention());
        assert!(
            report.workers[0]
                .diagnostics
                .iter()
                .any(|d| d.code == "WORKSPACE_METADATA")
        );
        assert!(report.workers[0].resume_argv.is_none());
    }
}
