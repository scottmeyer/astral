#![cfg(unix)]

use ostk_gpt_cache::project::Result;
use ostk_gpt_cache::workspace::{
    BindingStatus, MAX_BINDING_BYTES, WorkerMetadata, WorktreeBinding,
};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const WORK: &str = "AST-0123456789ab";
const OTHER: &str = "AST-0123456789ac";
const THREAD: &str = "01a08e8f-c4af-77a3-9643-7c474187c260";

struct Fixture {
    _temp: TempDir,
    base: PathBuf,
    root: PathBuf,
}

fn git(root: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Workspace Test")
        .env("GIT_AUTHOR_EMAIL", "workspace@example.invalid")
        .env("GIT_COMMITTER_NAME", "Workspace Test")
        .env("GIT_COMMITTER_EMAIL", "workspace@example.invalid");
    for key in [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
    ] {
        cmd.env_remove(key);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .trim_end_matches('\n')
        .into()
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("source");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "--initial-branch=main"]);
        fs::write(root.join("tracked.txt"), "committed\n").unwrap();
        git(&root, &["add", "tracked.txt"]);
        git(&root, &["commit", "-m", "initial"]);
        Self {
            _temp: temp,
            base,
            root,
        }
    }
    fn acquire(&self, work: &str) -> Result<WorktreeBinding> {
        WorktreeBinding::acquire_with_root(&self.root, "fixture", "project-context", work, None)
    }
    fn open(&self, work: &str) -> Result<WorktreeBinding> {
        WorktreeBinding::open_existing_with_root(
            &self.root,
            "fixture",
            "project-context",
            work,
            None,
        )
    }
    fn inspect(
        &self,
        work: &str,
    ) -> ostk_gpt_cache::project::Result<ostk_gpt_cache::workspace::WorkspacePlan> {
        WorktreeBinding::inspect_with_root(&self.root, "fixture", "project-context", work, None)
    }
    fn state_dir(&self) -> PathBuf {
        self.root.join(".git/astral/work-bindings")
    }
    fn receipt_path(&self, work: &str) -> PathBuf {
        self.state_dir().join(work).join("receipt.json")
    }
    fn receipt(&self, work: &str) -> Value {
        serde_json::from_slice(&fs::read(self.receipt_path(work)).unwrap()).unwrap()
    }
    fn write_receipt(&self, work: &str, value: &Value) {
        fs::write(self.receipt_path(work), serde_json::to_vec(value).unwrap()).unwrap();
    }
    fn child(&self, work: &str, mode: &str) -> std::process::Output {
        self.child_command(work, mode).output().unwrap()
    }
    fn child_command(&self, work: &str, mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "workspace_child_fixture", "--nocapture"])
            .env("ASTRAL_WORKSPACE_TEST_ROOT", &self.root)
            .env("ASTRAL_WORKSPACE_TEST_WORK", work)
            .env("ASTRAL_WORKSPACE_TEST_MODE", mode)
            .env_remove("ASTRAL_WORKTREE_ROOT");
        cmd
    }
}

fn code<T>(result: Result<T>) -> &'static str {
    result.err().expect("operation must fail").code
}
fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().mode() & 0o7777
}

fn metadata() -> WorkerMetadata {
    WorkerMetadata {
        context_initialized: true,
        staging_in_progress: false,
        requires_tool_rebinding: true,
        launch_id: Some("1".repeat(32)),
        thread_id: Some(THREAD.into()),
        model: Some("gpt-6-astra".into()),
        provider: Some("openai".into()),
        selection_digest: Some("a".repeat(64)),
        seed_bundle_sha256: Some("b".repeat(64)),
        saved_bundle_sha256: Some("c".repeat(64)),
        selected_bundle_sha256: Some("b".repeat(64)),
        selected_bundle_recorded: true,
    }
}

#[test]
fn selected_bundle_anchor_is_independent_of_latest_export_and_upgrades_old_receipts() {
    let mut worker = metadata();
    assert!(worker.accepts_selected_bundle(Some(&"b".repeat(64))));
    assert!(!worker.accepts_selected_bundle(Some(&"c".repeat(64))));
    assert!(!worker.accepts_selected_bundle(None));
    worker.selected_bundle_sha256 = None;
    assert!(worker.accepts_selected_bundle(None));
    assert!(!worker.accepts_selected_bundle(Some(&"c".repeat(64))));
    worker.selected_bundle_recorded = false;
    assert!(worker.accepts_selected_bundle(Some(&"b".repeat(64))));
    assert!(worker.accepts_selected_bundle(Some(&"c".repeat(64))));
    assert!(!worker.accepts_selected_bundle(Some(&"d".repeat(64))));
    worker.saved_bundle_sha256 = None;
    assert!(!worker.accepts_selected_bundle(None));
    worker.seed_bundle_sha256 = None;
    assert!(worker.accepts_selected_bundle(None));
    let mut old = serde_json::to_value(&worker).unwrap();
    old.as_object_mut()
        .unwrap()
        .remove("selected_bundle_sha256");
    old.as_object_mut()
        .unwrap()
        .remove("selected_bundle_recorded");
    let old: WorkerMetadata = serde_json::from_value(old).unwrap();
    old.validate().unwrap();
    assert!(!old.selected_bundle_recorded);
}

#[test]
fn inspect_and_open_unbound_do_not_write_any_state_or_worktree() {
    let f = Fixture::new();
    let before = git(
        &f.root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );
    let plan = f.inspect(WORK).unwrap();
    assert_eq!(plan.status, BindingStatus::Planned);
    assert!(!plan.existing);
    assert_eq!(plan.branch, format!("astral/{WORK}"));
    assert_eq!(
        plan.root,
        f.base
            .join(".astral-worktrees")
            .join(&plan.repository_id)
            .join(WORK)
    );
    assert_eq!(plan.base_commit, git(&f.root, &["rev-parse", "HEAD"]));
    assert_eq!(code(f.open(WORK)), "WORKSPACE_NOT_BOUND");
    assert!(!f.root.join(".git/astral").exists());
    assert!(!f.base.join(".astral-worktrees").exists());
    assert_eq!(
        git(
            &f.root,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        before
    );
}

#[test]
fn creation_preserves_dirty_source_and_reuses_dirty_worktree_and_metadata() {
    let f = Fixture::new();
    fs::write(f.root.join("tracked.txt"), "staged source\n").unwrap();
    git(&f.root, &["add", "tracked.txt"]);
    fs::write(f.root.join("tracked.txt"), "unstaged source\n").unwrap();
    fs::write(f.root.join("untracked.txt"), "private source\n").unwrap();
    let before = git(
        &f.root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    );
    let index = fs::read(f.root.join(".git/index")).unwrap();
    let source_head = git(&f.root, &["rev-parse", "HEAD"]);
    let mut binding = f.acquire(WORK).unwrap();
    let root = binding.root().to_path_buf();
    assert_eq!(binding.base_commit(), source_head);
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "committed\n"
    );
    assert!(!root.join("untracked.txt").exists());
    assert_eq!(
        fs::read_to_string(f.root.join("tracked.txt")).unwrap(),
        "unstaged source\n"
    );
    assert_eq!(
        fs::read_to_string(f.root.join("untracked.txt")).unwrap(),
        "private source\n"
    );
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), index);
    assert_eq!(
        git(
            &f.root,
            &["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        before
    );
    binding.update_worker_metadata(metadata()).unwrap();
    fs::write(root.join("tracked.txt"), "worker changes\n").unwrap();
    fs::write(root.join("worker-untracked"), "keep\n").unwrap();
    assert_eq!(mode(&f.root.join(".git/astral")), 0o700);
    assert_eq!(mode(&f.state_dir()), 0o700);
    assert_eq!(mode(&f.state_dir().join(WORK)), 0o700);
    assert_eq!(mode(&f.receipt_path(WORK)), 0o600);
    assert_eq!(mode(&f.state_dir().join(WORK).join("owner.lock")), 0o600);
    let receipt_bytes = fs::read(f.receipt_path(WORK)).unwrap();
    let observed = f.inspect(WORK).unwrap();
    assert!(observed.existing);
    assert_eq!(observed.worker_metadata, metadata());
    assert_eq!(fs::read(f.receipt_path(WORK)).unwrap(), receipt_bytes);
    assert_eq!(code(f.acquire(WORK)), "WORKSPACE_OWNED");
    drop(binding);
    let resumed = f.acquire(WORK).unwrap();
    assert_eq!(resumed.root(), root);
    assert_eq!(resumed.worker_metadata(), &metadata());
    assert_eq!(
        fs::read_to_string(root.join("tracked.txt")).unwrap(),
        "worker changes\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("worker-untracked")).unwrap(),
        "keep\n"
    );
    assert_eq!(
        git(&f.root, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        2
    );
}

#[test]
fn work_items_can_be_owned_concurrently_and_repositories_are_isolated() {
    let f = Fixture::new();
    let other_repo = Fixture::new();
    let first = f.acquire(WORK).unwrap();
    let second = f.acquire(OTHER).unwrap();
    let other = other_repo.acquire(WORK).unwrap();
    assert_ne!(first.root(), second.root());
    assert_eq!(first.repository_id(), second.repository_id());
    assert_ne!(first.repository_id(), other.repository_id());
    assert_ne!(first.root(), other.root());
    assert_eq!(first.branch(), other.branch());
    let nested =
        WorktreeBinding::inspect_with_root(first.root(), "fixture", "project-context", WORK, None)
            .unwrap();
    assert_eq!(nested.root, first.root());
}

#[test]
fn branches_and_even_empty_paths_without_bindings_are_conflicts() {
    let f = Fixture::new();
    let planned = f.inspect(WORK).unwrap();
    git(&f.root, &["branch", &planned.branch]);
    assert_eq!(code(f.acquire(WORK)), "WORKSPACE_CONFLICT");
    assert!(!f.receipt_path(WORK).exists());
    assert!(!planned.root.exists());
    let second = f.inspect(OTHER).unwrap();
    fs::create_dir_all(&second.root).unwrap();
    assert_eq!(code(f.inspect(OTHER)), "WORKSPACE_CONFLICT");
    assert_eq!(code(f.acquire(OTHER)), "WORKSPACE_CONFLICT");
    assert!(second.root.is_dir());
    assert!(!f.receipt_path(OTHER).exists());
    let third = "AST-0123456789ad";
    let third_plan = f.inspect(third).unwrap();
    symlink(f.base.join("missing"), &third_plan.root).unwrap();
    assert_eq!(code(f.acquire(third)), "WORKSPACE_CONFLICT");
    assert!(
        fs::symlink_metadata(third_plan.root)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn changed_branch_detached_head_and_non_descendant_ref_are_rejected() {
    for kind in ["different", "detached", "non-descendant"] {
        let f = Fixture::new();
        if kind == "non-descendant" {
            fs::write(f.root.join("next"), "second").unwrap();
            git(&f.root, &["add", "next"]);
            git(&f.root, &["commit", "-m", "second"]);
        }
        let bound = f.acquire(WORK).unwrap();
        let root = bound.root().to_path_buf();
        drop(bound);
        match kind {
            "different" => {
                git(&root, &["switch", "-c", "unrelated"]);
            }
            "detached" => {
                git(&root, &["switch", "--detach"]);
            }
            _ => {
                git(
                    &root,
                    &["update-ref", &format!("refs/heads/astral/{WORK}"), "HEAD^"],
                );
            }
        }
        let error = code(f.acquire(WORK));
        assert!(
            matches!(error, "WORKSPACE_BRANCH_MISMATCH" | "WORKSPACE_GIT_FAILED"),
            "{kind}: {error}"
        );
        assert!(root.exists());
        assert!(f.receipt_path(WORK).exists());
    }
}

#[test]
fn regular_worker_commits_are_preserved_and_base_remains_stable() {
    let f = Fixture::new();
    let bound = f.acquire(WORK).unwrap();
    let root = bound.root().to_path_buf();
    let base = bound.base_commit().to_owned();
    drop(bound);
    fs::write(root.join("new"), "worker commit").unwrap();
    git(&root, &["add", "new"]);
    git(&root, &["commit", "-m", "worker"]);
    let head = git(&root, &["rev-parse", "HEAD"]);
    let bound = f.open(WORK).unwrap();
    assert_eq!(bound.base_commit(), base);
    assert_ne!(head, base);
    assert_eq!(git(bound.root(), &["rev-parse", "HEAD"]), head);
    let next =
        WorktreeBinding::acquire_with_root(bound.root(), "fixture", "project-context", OTHER, None)
            .unwrap();
    assert_eq!(next.base_commit(), head);
    assert_eq!(next.root().parent(), bound.root().parent());
    assert_eq!(
        fs::read_to_string(next.root().join("new")).unwrap(),
        "worker commit"
    );
}

#[test]
fn metadata_is_bounded_validated_on_read_and_write_and_rejects_copies() {
    let f = Fixture::new();
    let mut bound = f.acquire(WORK).unwrap();
    let bad = WorkerMetadata {
        thread_id: Some("$(touch marker)".into()),
        ..WorkerMetadata::default()
    };
    assert_eq!(
        bound.update_worker_metadata(bad).unwrap_err().code,
        "WORKSPACE_METADATA"
    );
    bound.update_worker_metadata(metadata()).unwrap();
    drop(bound);
    let original = f.receipt(WORK);
    for (key, value) in [
        ("thread_id", json!("not-a-uuid")),
        ("thread_id", json!("00000000-0000-0000-0000-000000000000")),
        ("launch_id", json!("A".repeat(32))),
        ("selection_digest", json!("a".repeat(63))),
        ("seed_bundle_sha256", json!("z".repeat(64))),
        ("saved_bundle_sha256", json!("a".repeat(65))),
        ("model", json!("PRIVATE\nVALUE")),
        ("provider", json!("x".repeat(257))),
        (
            "arguments",
            json!(["--dangerously-bypass-approvals-and-sandbox"]),
        ),
    ] {
        let mut changed = original.clone();
        changed["worker_metadata"][key] = value;
        f.write_receipt(WORK, &changed);
        let err = f.open(WORK).err().unwrap();
        assert_eq!(err.code, "WORKSPACE_METADATA", "{key}");
        assert!(!err.message.contains("PRIVATE"));
    }
    for (key, value) in [
        ("repository_id", json!("a".repeat(64))),
        ("work_id", json!(OTHER)),
        ("root", json!("/private/tmp/copied")),
        ("branch", json!("main")),
        ("schema_version", json!(2)),
        ("common_identity", json!({"device": 0, "inode": 0})),
    ] {
        let mut changed = original.clone();
        changed[key] = value;
        f.write_receipt(WORK, &changed);
        assert_eq!(code(f.open(WORK)), "WORKSPACE_MISMATCH", "{key}");
    }
    for bytes in [b"not JSON".to_vec(), vec![b' '; MAX_BINDING_BYTES + 1]] {
        fs::write(f.receipt_path(WORK), bytes).unwrap();
        assert!(matches!(
            code(f.open(WORK)),
            "WORKSPACE_METADATA" | "WORKSPACE_METADATA_LIMIT"
        ));
    }
    f.write_receipt(WORK, &original);
    let other_repo = Fixture::new();
    drop(other_repo.acquire(WORK).unwrap());
    other_repo.write_receipt(WORK, &original);
    assert_eq!(code(other_repo.open(WORK)), "WORKSPACE_MISMATCH");
    drop(f.acquire(OTHER).unwrap());
    f.write_receipt(OTHER, &original);
    assert_eq!(code(f.open(OTHER)), "WORKSPACE_MISMATCH");
}

#[test]
fn symlinked_or_replaced_worktree_and_private_state_are_not_adopted() {
    for kind in [
        "missing", "symlink", "replaced", "receipt", "lock", "state", "dot-git", "gitdir",
    ] {
        let f = Fixture::new();
        let binding = f.acquire(WORK).unwrap();
        let root = binding.root().to_path_buf();
        drop(binding);
        let receipt = f.receipt(WORK);
        match kind {
            "missing" | "symlink" | "replaced" => {
                let moved = root.with_extension("retained");
                fs::rename(&root, &moved).unwrap();
                if kind == "symlink" {
                    symlink(moved, &root).unwrap();
                }
                if kind == "replaced" {
                    fs::create_dir(&root).unwrap();
                }
            }
            _ => {
                let path = match kind {
                    "receipt" => f.receipt_path(WORK),
                    "lock" => f.state_dir().join(WORK).join("owner.lock"),
                    "state" => f.state_dir().join(WORK),
                    "dot-git" => root.join(".git"),
                    _ => PathBuf::from(receipt["git_dir"].as_str().unwrap()),
                };
                let moved = path.with_extension("retained");
                fs::rename(&path, &moved).unwrap();
                symlink(moved, path).unwrap();
            }
        }
        let err = code(f.open(WORK));
        assert!(
            matches!(
                err,
                "WORKSPACE_MISMATCH"
                    | "WORKSPACE_UNSAFE_PATH"
                    | "WORKSPACE_MISSING"
                    | "WORKSPACE_GIT_FAILED"
            ),
            "{kind}: {err}"
        );
    }
}

#[test]
fn modified_receipt_is_not_overwritten_while_owned() {
    let f = Fixture::new();
    let mut bound = f.acquire(WORK).unwrap();
    fs::write(f.receipt_path(WORK), "external modification").unwrap();
    assert_eq!(
        bound.update_worker_metadata(metadata()).unwrap_err().code,
        "WORKSPACE_CHANGED"
    );
    assert_eq!(bound.worker_metadata(), &WorkerMetadata::default());
    assert_eq!(
        fs::read_to_string(f.receipt_path(WORK)).unwrap(),
        "external modification"
    );
}

#[test]
fn subprocess_ownership_and_drop_and_cold_reuse() {
    let f = Fixture::new();
    let mut bound = f.acquire(WORK).unwrap();
    bound.update_worker_metadata(metadata()).unwrap();
    let path = bound.root().to_path_buf();
    let blocked = f.child(WORK, "acquire");
    assert!(blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stdout).contains("WORKSPACE_CHILD_WORKSPACE_OWNED"));
    let distinct = f.child(OTHER, "acquire");
    assert!(distinct.status.success());
    assert!(String::from_utf8_lossy(&distinct.stdout).contains("WORKSPACE_CHILD_OK"));
    let save_blocked = f.child(WORK, "save");
    assert!(save_blocked.status.success());
    assert!(
        String::from_utf8_lossy(&save_blocked.stdout).contains("WORKSPACE_CHILD_WORKSPACE_OWNED")
    );
    drop(bound);
    let resumed = f.child(WORK, "metadata");
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("WORKSPACE_CHILD_OK"));
    assert_eq!(f.open(WORK).unwrap().root(), path);
}

#[test]
fn workspace_child_fixture() {
    let Some(mode) = std::env::var_os("ASTRAL_WORKSPACE_TEST_MODE") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("ASTRAL_WORKSPACE_TEST_ROOT").unwrap());
    let work = std::env::var("ASTRAL_WORKSPACE_TEST_WORK").unwrap();
    let result = if mode == "save" {
        WorktreeBinding::open_existing(&root, "fixture", "project-context", &work)
    } else if mode == "inspect" {
        match WorktreeBinding::inspect(&root, "fixture", "project-context", &work) {
            Ok(plan) => {
                println!("WORKSPACE_PLAN_{}", plan.root.display());
                return;
            }
            Err(e) => {
                println!("WORKSPACE_CHILD_{}", e.code);
                return;
            }
        }
    } else {
        WorktreeBinding::acquire(&root, "fixture", "project-context", &work)
    };
    match result {
        Ok(binding) => {
            if mode == "metadata" {
                assert_eq!(binding.worker_metadata(), &metadata());
            }
            println!("WORKSPACE_CHILD_OK");
        }
        Err(e) => println!("WORKSPACE_CHILD_{}", e.code),
    }
}

#[test]
fn trusted_external_root_is_absolute_confined_and_isolated() {
    let f = Fixture::new();
    let custom = f.base.join("external with spaces");
    let plan = WorktreeBinding::inspect_with_root(
        &f.root,
        "fixture",
        "project-context",
        WORK,
        Some(&custom),
    )
    .unwrap();
    assert!(!custom.exists());
    let binding = WorktreeBinding::acquire_with_root(
        &f.root,
        "fixture",
        "project-context",
        WORK,
        Some(&custom),
    )
    .unwrap();
    assert_eq!(binding.root(), plan.root);
    drop(binding);
    assert_eq!(code(f.open(WORK)), "WORKSPACE_MISMATCH");
    for path in [
        PathBuf::from("relative"),
        f.base.join("../escape"),
        f.root.join("managed"),
        f.root.join(".git/managed"),
    ] {
        assert_eq!(
            code(WorktreeBinding::inspect_with_root(
                &f.root,
                "fixture",
                "project-context",
                OTHER,
                Some(&path)
            )),
            "WORKSPACE_PATH"
        );
    }
    let alias = f.base.join("alias");
    symlink(&custom, &alias).unwrap();
    assert_eq!(
        code(WorktreeBinding::inspect_with_root(
            &f.root,
            "fixture",
            "project-context",
            OTHER,
            Some(&alias)
        )),
        "WORKSPACE_UNSAFE_PATH"
    );
    let output = f
        .child_command(WORK, "inspect")
        .env("ASTRAL_WORKTREE_ROOT", &custom)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains(&format!("WORKSPACE_PLAN_{}", plan.root.display()))
    );
}

#[test]
fn hostile_inputs_are_not_shell_expanded_and_git_redirection_is_removed() {
    let f = Fixture::new();
    for work in [
        "../escape",
        "x;touch marker",
        "$(touch marker)",
        "`touch marker`",
        "x\n--force",
        "x.lock",
        "x..y",
        ".hidden",
    ] {
        assert_eq!(code(f.acquire(work)), "WORKSPACE_IDENTITY");
    }
    let weird_path = f.base.join("$(touch marker);literal");
    let binding = WorktreeBinding::acquire_with_root(
        &f.root,
        "fixture",
        "project-context",
        WORK,
        Some(&weird_path),
    )
    .unwrap();
    assert!(binding.root().starts_with(&weird_path));
    assert!(!f.root.join("marker").exists());
    assert!(!f.base.join("marker").exists());
    let other = Fixture::new();
    let output = f
        .child_command(OTHER, "acquire")
        .env("GIT_DIR", other.root.join(".git"))
        .env("GIT_COMMON_DIR", other.root.join(".git"))
        .env("GIT_WORK_TREE", &other.root)
        .env("GIT_INDEX_FILE", other.root.join("injected-index"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.worktree")
        .env("GIT_CONFIG_VALUE_0", &other.root)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("WORKSPACE_CHILD_OK"));
    assert!(f.receipt_path(OTHER).exists());
    assert!(!other.state_dir().exists());
    assert!(!other.root.join("injected-index").exists());
}

#[test]
fn managed_creation_does_not_execute_hooks_or_write_git_config() {
    let f = Fixture::new();
    let marker = f.base.join("hook-ran");
    let hook = f.root.join(".git/hooks/post-checkout");
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    let before = fs::read(f.root.join(".git/config")).unwrap();
    let bound = f.acquire(WORK).unwrap();
    assert!(!marker.exists());
    assert_eq!(fs::read(f.root.join(".git/config")).unwrap(), before);
    assert_eq!(
        git(bound.root(), &["symbolic-ref", "HEAD"]),
        format!("refs/heads/astral/{WORK}")
    );
}

#[test]
fn failed_creation_is_retained_and_requires_explicit_manual_recovery() {
    let f = Fixture::new();
    let plan = f.inspect(WORK).unwrap();
    // refs/heads/astral blocks creation of refs/heads/astral/WORK without an exact ref collision.
    git(&f.root, &["branch", "astral"]);
    assert_eq!(code(f.acquire(WORK)), "WORKSPACE_GIT_FAILED");
    let value = f.receipt(WORK);
    assert_eq!(value["status"], "failed");
    assert_eq!(value["last_error"], "WORKSPACE_GIT_FAILED");
    assert_eq!(code(f.acquire(WORK)), "WORKSPACE_INCOMPLETE");
    assert_eq!(code(f.open(WORK)), "WORKSPACE_INCOMPLETE");
    let observed = f.inspect(WORK).unwrap();
    assert!(observed.existing);
    assert_eq!(observed.status, BindingStatus::Failed);
    assert_eq!(observed.root, plan.root);
    assert_eq!(f.receipt(WORK), value);
    assert!(f.state_dir().join(WORK).join("owner.lock").exists());
    assert_eq!(
        git(&f.root, &["branch", "--list", "astral"]).trim(),
        "astral"
    );
}

#[test]
fn checkout_failure_retains_registered_worktree_branch_and_partial_checkout() {
    let f = Fixture::new();
    fs::write(
        f.root.join(".gitattributes"),
        "tracked.txt filter=workspace-test\n",
    )
    .unwrap();
    git(&f.root, &["add", ".gitattributes"]);
    git(&f.root, &["commit", "-m", "checkout filter fixture"]);
    git(
        &f.root,
        &["config", "filter.workspace-test.smudge", "false"],
    );
    git(
        &f.root,
        &["config", "filter.workspace-test.required", "true"],
    );
    let plan = f.inspect(WORK).unwrap();
    assert_eq!(code(f.acquire(WORK)), "WORKSPACE_GIT_FAILED");
    assert!(plan.root.join(".git").is_file());
    assert!(plan.root.join(".gitattributes").is_file());
    assert_eq!(
        git(
            &f.root,
            &["rev-parse", &format!("refs/heads/{}", plan.branch)]
        ),
        plan.base_commit
    );
    assert_eq!(f.inspect(WORK).unwrap().status, BindingStatus::Failed);
    assert_eq!(code(f.acquire(WORK)), "WORKSPACE_INCOMPLETE");
}

#[test]
fn git_output_and_lingering_pipe_writers_are_bounded() {
    let f = Fixture::new();
    let fake_bin = f.base.join("fake-bin");
    fs::create_dir(&fake_bin).unwrap();
    let fake_git = fake_bin.join("git");
    for (script, expected) in [
        (
            "#!/bin/sh\nwhile :; do printf 'oversized-git-output\\n'; done\n",
            "WORKSPACE_GIT_LIMIT",
        ),
        (
            "#!/bin/sh\n/bin/sleep 60 &\nexit 0\n",
            "WORKSPACE_GIT_TIMEOUT",
        ),
    ] {
        fs::write(&fake_git, script).unwrap();
        fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();
        let started = std::time::Instant::now();
        let output = f
            .child_command(WORK, "inspect")
            .env("PATH", &fake_bin)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("WORKSPACE_CHILD_{expected}"))
        );
        assert!(started.elapsed() < std::time::Duration::from_secs(40));
        assert!(!f.root.join(".git/astral").exists());
    }
}

#[test]
fn missing_lock_and_insecure_or_hardlinked_receipt_fail_closed() {
    for kind in ["missing-lock", "permissions", "hardlink"] {
        let f = Fixture::new();
        drop(f.acquire(WORK).unwrap());
        match kind {
            "missing-lock" => {
                fs::rename(
                    f.state_dir().join(WORK).join("owner.lock"),
                    f.state_dir().join(WORK).join("old.lock"),
                )
                .unwrap();
                assert_eq!(code(f.open(WORK)), "WORKSPACE_INCOMPLETE");
                assert!(!f.state_dir().join(WORK).join("owner.lock").exists());
            }
            "permissions" => {
                fs::set_permissions(f.receipt_path(WORK), fs::Permissions::from_mode(0o644))
                    .unwrap();
                assert_eq!(code(f.open(WORK)), "WORKSPACE_PRIVATE_STATE");
            }
            _ => {
                fs::hard_link(f.receipt_path(WORK), f.base.join("copied-receipt")).unwrap();
                assert_eq!(code(f.open(WORK)), "WORKSPACE_PRIVATE_STATE");
            }
        }
    }
}
