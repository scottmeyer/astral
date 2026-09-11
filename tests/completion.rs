#![cfg(unix)]
use ostk_gpt_cache::{
    completion::{self, FinishRequest, FinishState},
    project::Project,
    workspace::{WorkerMetadata, WorktreeBinding},
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const WORK: &str = "AST-001";
const THREAD: &str = "01a10000-1234-7000-8000-000000000001";
fn put(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_AUTHOR_NAME", "Completion Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Completion Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim_end().into()
}
fn commit(root: &Path) {
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "fixture"]);
}
fn record(root: &Path, title: &str) {
    put(
        root,
        ".astral/work/items.jsonl",
        format!(
            "{}\n",
            json!({"schema_version":1,"id":WORK,"title":title,"status":"in_progress","acceptance":["Review completed work"],"depends_on":[]})
        ),
    );
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    worker: PathBuf,
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
description = "Completion fixture"
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
            put(&root, &format!(".astral/core/{name}"), "Fixture docs.\n");
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
            "Readable seed.\n",
        );
        record(&root, "Original task");
        put(&root, "code.txt", "base\n");
        git(&root, &["init", "--initial-branch=main"]);
        commit(&root);
        let mut binding = WorktreeBinding::acquire(&root, "fixture", "web", WORK).unwrap();
        let worker = binding.root().to_owned();
        let digest = Project::load(&worker)
            .unwrap()
            .launch_context("web", Some(WORK))
            .unwrap()
            .current_context
            .selection_digest;
        binding
            .update_worker_metadata(WorkerMetadata {
                context_initialized: true,
                thread_id: Some(THREAD.into()),
                model: Some("gpt-6-astra".into()),
                provider: Some("openai".into()),
                selection_digest: Some(digest),
                selected_bundle_recorded: true,
                ..Default::default()
            })
            .unwrap();
        drop(binding);
        Self {
            _temp: temp,
            root,
            worker,
        }
    }
    fn request(&self) -> FinishRequest {
        FinishRequest {
            root: self.root.clone(),
            work: WORK.into(),
            into: "main".into(),
            context: None,
        }
    }
    fn advance(&self) {
        put(&self.worker, "code.txt", "worker\n");
        put(
            &self.worker,
            ".astral/core/web/README.md",
            "Reviewed current decisions.\n",
        );
        commit(&self.worker);
    }
}

#[test]
fn preview_is_read_only_and_reviewed_apply_is_idempotent() {
    let f = Fixture::new();
    f.advance();
    let target = git(&f.root, &["rev-parse", "HEAD"]);
    let before = git(&f.root, &["status", "--porcelain=v1"]);
    let plan = completion::plan(&f.request()).unwrap();
    assert_eq!(plan.state, FinishState::FastForwardReady);
    assert_eq!(
        plan.plan_sha256,
        completion::plan(&f.request()).unwrap().plan_sha256
    );
    assert_eq!(target, git(&f.root, &["rev-parse", "HEAD"]));
    assert_eq!(before, git(&f.root, &["status", "--porcelain=v1"]));
    assert!(plan.changes.iter().any(|p| p.category == "code_or_other"));
    assert!(
        plan.changes
            .iter()
            .any(|p| p.category == "readable_context_or_manifest")
    );
    let applied = completion::apply(&f.request(), &plan.plan_sha256).unwrap();
    assert!(applied.applied);
    assert_eq!(applied.plan.state, FinishState::AlreadyIntegrated);
    assert_eq!(fs::read(f.root.join("code.txt")).unwrap(), b"worker\n");
    assert!(!applied.work_status_updated);
    assert_eq!(applied.plan.records.worker_status, "in_progress");
    assert!(f.worker.exists());
    let retried = completion::apply(&f.request(), &plan.plan_sha256).unwrap();
    assert!(!retried.applied);
    assert_eq!(retried.plan.state, FinishState::AlreadyIntegrated);
}

#[test]
fn advanced_evidence_and_wrong_target_cannot_apply() {
    let f = Fixture::new();
    f.advance();
    let plan = completion::plan(&f.request()).unwrap();
    put(&f.worker, "another.txt", "new evidence\n");
    commit(&f.worker);
    assert_eq!(
        completion::apply(&f.request(), &plan.plan_sha256)
            .unwrap_err()
            .code,
        "FINISH_PLAN_CHANGED"
    );
    let mut req = f.request();
    req.into = "astral/AST-001".into();
    assert_eq!(completion::plan(&req).unwrap_err().code, "FINISH_TARGET");
}

#[test]
fn dirty_owned_and_uncertain_workers_are_blocked() {
    let f = Fixture::new();
    f.advance();
    put(&f.root, "untracked.txt", "preserve me");
    assert_eq!(
        completion::plan(&f.request()).unwrap_err().code,
        "FINISH_DIRTY"
    );
    git(&f.root, &["add", "untracked.txt"]);
    git(&f.root, &["commit", "-m", "preserve"]);
    let mut binding = WorktreeBinding::open_existing(&f.root, "fixture", "web", WORK).unwrap();
    assert!(completion::plan(&f.request()).is_err());
    let mut metadata = binding.worker_metadata().clone();
    metadata.staging_in_progress = true;
    binding.update_worker_metadata(metadata).unwrap();
    drop(binding);
    assert_eq!(
        completion::plan(&f.request()).unwrap_err().code,
        "FINISH_INCOMPLETE_WORKER"
    );
}

#[test]
fn record_conflicts_and_divergent_code_require_manual_integration() {
    let f = Fixture::new();
    record(&f.root, "Target edit");
    commit(&f.root);
    record(&f.worker, "Worker edit");
    commit(&f.worker);
    let plan = completion::plan(&f.request()).unwrap();
    assert_eq!(plan.state, FinishState::ManualIntegrationRequired);
    assert_eq!(plan.records.outcome, "conflicts");
    assert_eq!(plan.records.conflicts[0].id, WORK);
    assert!(plan.blockers.contains(&"DIVERGED_GIT_HISTORY"));
    assert_eq!(
        completion::apply(&f.request(), &plan.plan_sha256)
            .unwrap_err()
            .code,
        "FINISH_MANUAL_REQUIRED"
    );
}

#[test]
fn ordinary_merge_is_recognized_but_a_revert_is_not() {
    let f = Fixture::new();
    f.advance();
    put(&f.root, "target-only.txt", "target\n");
    commit(&f.root);
    assert_eq!(
        completion::plan(&f.request()).unwrap().state,
        FinishState::ManualIntegrationRequired
    );
    git(&f.root, &["merge", "--no-edit", "astral/AST-001"]);
    assert_eq!(
        completion::plan(&f.request()).unwrap().state,
        FinishState::AlreadyIntegrated
    );
    put(&f.root, "code.txt", "base\n");
    commit(&f.root);
    let plan = completion::plan(&f.request()).unwrap();
    assert!(
        plan.blockers
            .contains(&"WORKER_CHANGES_NOT_RETAINED_EXACTLY")
    );
}

#[test]
fn policy_filters_attributes_and_ignored_data_are_never_executed_or_overwritten() {
    for (key, value, code) in [
        (
            "filter.evil.clean",
            "touch SHOULD_NOT_RUN",
            "FINISH_EXTERNAL_FILTER",
        ),
        ("branch.main.mergeOptions", "--squash", "FINISH_GIT_POLICY"),
        ("merge.verifySignatures", "true", "FINISH_GIT_POLICY"),
        ("core.autocrlf", "true", "FINISH_CHECKOUT_CONVERSION"),
    ] {
        let f = Fixture::new();
        f.advance();
        git(&f.root, &["config", key, value]);
        assert_eq!(completion::plan(&f.request()).unwrap_err().code, code);
        assert!(!f.root.join("SHOULD_NOT_RUN").exists());
    }
    let f = Fixture::new();
    put(&f.worker, ".gitattributes", "*.txt ident\n");
    commit(&f.worker);
    assert_eq!(
        completion::plan(&f.request()).unwrap_err().code,
        "FINISH_CHECKOUT_CONVERSION"
    );
    let f = Fixture::new();
    put(&f.root, ".git/info/exclude", "ignored.txt\n");
    put(&f.root, "ignored.txt", "private ignored bytes");
    put(&f.worker, "ignored.txt", "incoming bytes");
    git(&f.worker, &["add", "-f", "ignored.txt"]);
    git(&f.worker, &["commit", "-m", "incoming"]);
    assert!(completion::plan(&f.request()).is_err());
    assert_eq!(
        fs::read(f.root.join("ignored.txt")).unwrap(),
        b"private ignored bytes"
    );
}

fn native(root: &Path, id: &str, opaque: &str) {
    let payload =
        serde_json::to_vec(&json!([{"type":"compaction","encrypted_content":opaque}])).unwrap();
    let manifest=serde_json::to_vec(&json!({"schema_version":1,"format":"astral-codex-native",
        "payload":{"file":"window.json","sha256":ostk_gpt_cache::hash(&payload),"bytes":payload.len(),"item_count":1},
        "compatibility":{"runtime":"codex","runtime_version":"0.154.0","protocol":"openai-responses-lite","provider":"openai","model":"gpt-6-astra","requires_tool_rebinding":true,"identity_scope":"same-account"},
        "source":{"project_id":"fixture","revision":null,"dirty":false,"selection_sha256":"a".repeat(64),"history_sha256":"b".repeat(64)},
        "capture":{"boundary":"completed-turn","history_complete":true,"last_checkpoint_index":0},"parents":[]})).unwrap();
    let digest = ostk_gpt_cache::hash(&manifest);
    put(
        root,
        &format!(".astral/projections/{id}/bundles/{digest}/manifest.json"),
        manifest,
    );
    put(
        root,
        &format!(".astral/projections/{id}/bundles/{digest}/window.json"),
        payload,
    );
    put(
        root,
        &format!(".astral/projections/{id}/handoff.md"),
        "Native fixture.\n",
    );
    put(
        root,
        &format!(".astral/projections/{id}/projection.toml"),
        format!(
            "schema_version = 1\nid = {id:?}\nkind = \"native-checkpoint\"\nsubsystems = [\"web\"]\nhandoff = \"handoff.md\"\nnative_payload_in_repository = true\nsources = []\n[native_bundle]\nmanifest = \"bundles/{digest}/manifest.json\"\nsha256 = {digest:?}\n"
        ),
    );
}

#[test]
fn distinct_native_histories_require_choice_and_both_artifacts_survive() {
    let f = Fixture::new();
    native(&f.root, "target-save", "TARGET_OPAQUE_SENTINEL");
    commit(&f.root);
    native(&f.worker, "worker-save", "WORKER_OPAQUE_SENTINEL");
    commit(&f.worker);
    let plan = completion::plan(&f.request()).unwrap();
    assert!(plan.context.explicit_choice_required);
    assert!(
        plan.blockers
            .contains(&"NATIVE_ARTIFACT_RETENTION_REQUIRED")
    );
    git(&f.worker, &["merge", "--no-edit", "main"]);
    let plan = completion::plan(&f.request()).unwrap();
    assert!(plan.context.explicit_choice_required);
    let mut req = f.request();
    req.context = Some("projection:worker-save".into());
    let plan = completion::plan(&req).unwrap();
    assert_eq!(plan.state, FinishState::FastForwardReady);
    assert!(plan.artifacts.iter().all(|a| a.retained_in_result));
    let serialized = serde_json::to_string(&plan).unwrap();
    assert!(!serialized.contains("OPAQUE_SENTINEL"));
    let result = completion::apply(&req, &plan.plan_sha256).unwrap();
    assert_eq!(result.plan.state, FinishState::AlreadyIntegrated);
    assert_eq!(
        result
            .plan
            .next_context_argv
            .as_ref()
            .unwrap()
            .last()
            .unwrap(),
        "--proxy"
    );
    assert_eq!(
        Project::load(&f.root).unwrap().validate().unwrap()["native_artifacts"],
        2
    );
}

#[test]
fn superseded_unselected_bundle_bytes_cannot_be_overwritten_by_apply() {
    let f = Fixture::new();
    let path = format!(
        ".astral/projections/seed/bundles/{}/window.json",
        "a".repeat(64)
    );
    put(&f.root, &path, "superseded immutable bytes");
    commit(&f.root);
    git(&f.worker, &["merge", "--ff-only", "main"]);
    put(&f.worker, &path, "replacement bytes");
    commit(&f.worker);
    let plan = completion::plan(&f.request()).unwrap();
    assert!(
        plan.blockers
            .contains(&"NATIVE_ARTIFACT_RETENTION_REQUIRED")
    );
    assert_eq!(
        completion::apply(&f.request(), &plan.plan_sha256)
            .unwrap_err()
            .code,
        "FINISH_MANUAL_REQUIRED"
    );
    assert_eq!(
        fs::read(f.root.join(path)).unwrap(),
        b"superseded immutable bytes"
    );
}

#[test]
fn ignored_unselected_declared_bundle_is_rejected_before_integration() {
    let f = Fixture::new();
    native(&f.worker, "unselected", "UNSELECTED_OPAQUE");
    let projection = ".astral/projections/unselected";
    put(&f.worker, ".gitignore", format!("{projection}/bundles/\n"));
    git(
        &f.worker,
        &[
            "add",
            ".gitignore",
            &format!("{projection}/projection.toml"),
            &format!("{projection}/handoff.md"),
        ],
    );
    git(&f.worker, &["commit", "-m", "untracked declared bundle"]);
    assert!(Project::load(&f.worker).is_ok());
    let before = git(&f.root, &["rev-parse", "HEAD"]);
    assert!(completion::plan(&f.request()).is_err());
    assert_eq!(before, git(&f.root, &["rev-parse", "HEAD"]));
}

#[test]
fn payload_larger_than_default_git_output_limit_is_checked_without_printing() {
    let f = Fixture::new();
    native(&f.worker, "large", &"opaque".repeat(220_000));
    commit(&f.worker);
    let plan = completion::plan(&f.request()).unwrap();
    assert_eq!(plan.state, FinishState::FastForwardReady);
    assert!(serde_json::to_vec(&plan).unwrap().len() < 32_000);
    let result = completion::apply(&f.request(), &plan.plan_sha256).unwrap();
    assert!(result.applied);
    assert!(Project::load(&f.root).is_ok());
}

#[test]
fn committed_source_bytes_are_checked_even_when_git_stat_cache_says_clean() {
    let f = Fixture::new();
    f.advance();
    git(&f.worker, &["config", "core.trustctime", "false"]);
    git(&f.worker, &["config", "core.checkstat", "minimal"]);
    let path = f.worker.join(".astral/core/RUN.md");
    let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old_time))
        .unwrap();
    git(&f.worker, &["update-index", "--refresh"]);
    let before = fs::metadata(&path).unwrap();
    fs::write(&path, "Changed docs.\n").unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), before.len());
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(before.modified().unwrap())
                .set_accessed(before.accessed().unwrap()),
        )
        .unwrap();
    assert_eq!(git(&f.worker, &["status", "--porcelain=v1"]), "");
    assert_eq!(
        completion::plan(&f.request()).unwrap_err().code,
        "FINISH_SOURCE_MISMATCH"
    );
}

#[test]
fn post_integration_work_status_update_does_not_undo_completion() {
    let f = Fixture::new();
    f.advance();
    record(&f.worker, "Worker reviewed task");
    commit(&f.worker);
    let plan = completion::plan(&f.request()).unwrap();
    completion::apply(&f.request(), &plan.plan_sha256).unwrap();
    let path = f.root.join(".astral/work/items.jsonl");
    let mut item: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    item["status"] = json!("complete");
    item["evidence"] = json!(["docs/verified.md"]);
    put(&f.root, "docs/verified.md", "User-reviewed evidence.\n");
    fs::write(path, format!("{}\n", item)).unwrap();
    commit(&f.root);
    let after = completion::plan(&f.request()).unwrap();
    assert_eq!(after.state, FinishState::AlreadyIntegrated);
    assert!(after.ancestry_integrated);
    assert_eq!(after.records.target_status.as_deref(), Some("complete"));
    let retry = completion::apply(&f.request(), &plan.plan_sha256).unwrap();
    assert!(!retry.applied);
}

#[test]
fn unrelated_ignored_build_outputs_survive_fast_forward() {
    let f = Fixture::new();
    f.advance();
    put(&f.root, ".git/info/exclude", "target/\n");
    put(
        &f.root,
        "target/local-build.bin",
        "private ignored build bytes",
    );
    let plan = completion::plan(&f.request()).unwrap();
    assert_eq!(plan.state, FinishState::FastForwardReady);
    completion::apply(&f.request(), &plan.plan_sha256).unwrap();
    assert_eq!(
        fs::read(f.root.join("target/local-build.bin")).unwrap(),
        b"private ignored build bytes"
    );
}

#[test]
fn tracked_deletions_require_manual_git_integration() {
    let f = Fixture::new();
    fs::remove_file(f.worker.join("code.txt")).unwrap();
    commit(&f.worker);
    let plan = completion::plan(&f.request()).unwrap();
    assert!(
        plan.blockers
            .contains(&"TRACKED_DELETIONS_REQUIRE_MANUAL_INTEGRATION")
    );
    assert_eq!(
        completion::apply(&f.request(), &plan.plan_sha256)
            .unwrap_err()
            .code,
        "FINISH_MANUAL_REQUIRED"
    );
    assert_eq!(fs::read(f.root.join("code.txt")).unwrap(), b"base\n");
}

#[test]
fn ignored_file_directory_boundaries_are_plan_collisions() {
    for ignored_directory in [true, false] {
        let f = Fixture::new();
        put(&f.root, ".git/info/exclude", "cache\n");
        let (private_path, incoming_path) = if ignored_directory {
            ("cache/private.txt", "cache")
        } else {
            ("cache", "cache/incoming.txt")
        };
        put(&f.root, private_path, "private data");
        put(&f.worker, incoming_path, "incoming data");
        git(&f.worker, &["add", "-f", incoming_path]);
        git(&f.worker, &["commit", "-m", "incoming shape"]);
        assert_eq!(
            completion::plan(&f.request()).unwrap_err().code,
            "FINISH_IGNORED_COLLISION"
        );
        assert_eq!(
            fs::read(f.root.join(private_path)).unwrap(),
            b"private data"
        );
    }
}

#[test]
fn global_signature_and_checkout_policies_are_seen_before_any_apply() {
    for (policy, expected) in [
        ("[merge]\nverifySignatures = true\n", "FINISH_GIT_POLICY"),
        ("[core]\nautocrlf = true\n", "FINISH_CHECKOUT_CONVERSION"),
    ] {
        let f = Fixture::new();
        f.advance();
        let reviewed = completion::plan(&f.request()).unwrap();
        let policy_path = f._temp.path().join("completion-policy.gitconfig");
        fs::write(&policy_path, policy).unwrap();
        let before = git(&f.root, &["rev-parse", "HEAD"]);
        let output = Command::new(env!("CARGO_BIN_EXE_astral"))
            .current_dir(&f.root)
            .args([
                "finish",
                "--work",
                WORK,
                "--into",
                "main",
                "--apply",
                &reviewed.plan_sha256,
            ])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &policy_path)
            .env("GIT_CONFIG_COUNT", "0")
            .env("ASTRAL_CODEX_BIN", f.root.join("must-not-run-codex"))
            .output()
            .unwrap();
        assert!(!output.status.success());
        let diagnostic = String::from_utf8(output.stderr).unwrap();
        assert!(diagnostic.contains(expected), "{diagnostic}");
        assert_eq!(before, git(&f.root, &["rev-parse", "HEAD"]));
        assert_eq!(fs::read(f.root.join("code.txt")).unwrap(), b"base\n");
    }
}
