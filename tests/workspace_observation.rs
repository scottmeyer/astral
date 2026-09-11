#![cfg(unix)]

use astral::workspace::{BindingObservation, BindingStatus, OwnershipObservation, WorktreeBinding};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const WORK: &str = "AST-observation";
const SELECTOR: &str = "projection:nondefault-context";

struct Fixture {
    _temp: TempDir,
    base: PathBuf,
    source: PathBuf,
}

fn git(root: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Observation Test")
        .env("GIT_AUTHOR_EMAIL", "observation@example.invalid")
        .env("GIT_COMMITTER_NAME", "Observation Test")
        .env("GIT_COMMITTER_EMAIL", "observation@example.invalid");
    for key in [
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
    ] {
        command.env_remove(key);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end_matches('\n')
        .into()
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let source = base.join("source");
        fs::create_dir(&source).unwrap();
        git(&source, &["init", "--initial-branch=main"]);
        fs::write(source.join("tracked.txt"), "committed\n").unwrap();
        git(&source, &["add", "tracked.txt"]);
        git(&source, &["commit", "-m", "initial"]);
        Self {
            _temp: temp,
            base,
            source,
        }
    }

    fn acquire(&self) -> WorktreeBinding {
        WorktreeBinding::acquire_with_root(&self.source, "fixture", SELECTOR, WORK, None).unwrap()
    }

    fn observe(&self) -> BindingObservation {
        WorktreeBinding::observe_with_root(&self.source, "fixture", WORK, None).unwrap()
    }

    fn directory(&self) -> PathBuf {
        self.source.join(".git/astral/work-bindings").join(WORK)
    }

    fn receipt(&self) -> Value {
        serde_json::from_slice(&fs::read(self.directory().join("receipt.json")).unwrap()).unwrap()
    }

    fn write_receipt(&self, value: &Value) {
        fs::write(
            self.directory().join("receipt.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
}

fn tree(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, entries: &mut BTreeMap<PathBuf, (u32, Vec<u8>)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let bytes = if metadata.is_file() {
                fs::read(&path).unwrap()
            } else {
                Vec::new()
            };
            entries.insert(
                path.strip_prefix(root).unwrap().into(),
                (metadata.mode(), bytes),
            );
            if metadata.is_dir() {
                visit(root, &path, entries);
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn issue(observed: &BindingObservation, code: &str) {
    assert!(
        observed.issues.iter().any(|error| error.code == code),
        "{observed:?}"
    );
}

#[test]
fn unbound_observation_has_no_plan_and_writes_nothing() {
    let fixture = Fixture::new();
    let before = tree(&fixture.base);
    let observed = fixture.observe();
    assert_eq!(observed.work_id, WORK);
    assert!(observed.selector.is_none());
    assert!(observed.plan.is_none());
    assert!(observed.issues.is_empty());
    assert_eq!(observed.ownership, OwnershipObservation::Unknown);
    assert_eq!(tree(&fixture.base), before);
    assert!(!fixture.source.join(".git/astral").exists());
    assert!(!fixture.base.join(".astral-worktrees").exists());
}

#[test]
fn orphan_artifacts_are_reported_without_adopting_or_creating_a_binding() {
    for binding_storage in [false, true] {
        let fixture = Fixture::new();
        if binding_storage {
            drop(fixture.acquire());
            fs::rename(fixture.directory(), fixture.base.join("retained-binding")).unwrap();
        } else {
            git(&fixture.source, &["branch", &format!("astral/{WORK}")]);
        }
        let before = tree(&fixture.base);
        let observed = fixture.observe();
        issue(&observed, "WORKSPACE_CONFLICT");
        assert!(observed.plan.is_none());
        assert!(observed.selector.is_none());
        assert_eq!(observed.ownership, OwnershipObservation::Unknown);
        assert_eq!(tree(&fixture.base), before);
    }
}

#[test]
fn busy_released_and_reacquired_observations_do_not_reserve_ownership() {
    let fixture = Fixture::new();
    let binding = fixture.acquire();
    let before = tree(&fixture.base);
    let observed = fixture.observe();
    assert_eq!(observed.selector.as_deref(), Some(SELECTOR));
    assert_eq!(observed.ownership, OwnershipObservation::Busy);
    assert!(observed.issues.is_empty());
    assert_eq!(observed.plan.as_ref().unwrap().root, binding.root());
    assert_eq!(observed.plan.as_ref().unwrap().status, BindingStatus::Ready);
    assert_eq!(
        serde_json::to_value(&observed).unwrap()["ownership"],
        "busy"
    );
    assert_eq!(tree(&fixture.base), before);
    drop(binding);
    for _ in 0..2 {
        let observed = fixture.observe();
        assert_eq!(observed.ownership, OwnershipObservation::Available);
        assert!(observed.issues.is_empty());
        let binding = fixture.acquire();
        assert_eq!(fixture.observe().ownership, OwnershipObservation::Busy);
        drop(binding);
    }
}

#[test]
fn explicit_managed_root_is_resolved_from_trusted_caller_configuration() {
    let fixture = Fixture::new();
    let managed = fixture.base.join("custom-managed");
    let binding = WorktreeBinding::acquire_with_root(
        &fixture.source,
        "fixture",
        SELECTOR,
        WORK,
        Some(&managed),
    )
    .unwrap();
    let observed =
        WorktreeBinding::observe_with_root(&fixture.source, "fixture", WORK, Some(&managed))
            .unwrap();
    assert_eq!(observed.ownership, OwnershipObservation::Busy);
    assert_eq!(observed.plan.unwrap().root, binding.root());
    drop(binding);
    let observed = fixture.observe();
    issue(&observed, "WORKSPACE_MISMATCH");
    assert!(observed.plan.is_none());
    assert_eq!(observed.ownership, OwnershipObservation::Unknown);
}

#[test]
fn active_preparation_and_failed_receipts_are_distinguished_from_unowned_incomplete_state() {
    for state in ["preparing", "failed"] {
        let fixture = Fixture::new();
        let binding = fixture.acquire();
        let mut receipt = fixture.receipt();
        receipt["status"] = json!(state);
        receipt["root_identity"] = Value::Null;
        receipt["git_dir"] = Value::Null;
        receipt["git_identity"] = Value::Null;
        receipt["last_error"] = if state == "failed" {
            json!("WORKSPACE_GIT_FAILED")
        } else {
            Value::Null
        };
        fixture.write_receipt(&receipt);
        // Preparation owns its lock before the proxy directory is created.
        fs::rename(
            fixture.directory().join("proxy"),
            fixture.base.join("retained-proxy"),
        )
        .unwrap();
        let before = tree(&fixture.base);
        let observed = fixture.observe();
        assert_eq!(observed.ownership, OwnershipObservation::Busy);
        assert!(observed.issues.is_empty(), "{observed:?}");
        assert!(observed.plan.is_some());
        assert_eq!(tree(&fixture.base), before);
        drop(binding);
        let observed = fixture.observe();
        assert_eq!(observed.ownership, OwnershipObservation::Available);
        issue(&observed, "WORKSPACE_INCOMPLETE");
        assert_eq!(
            observed.plan.unwrap().status,
            if state == "failed" {
                BindingStatus::Failed
            } else {
                BindingStatus::Preparing
            }
        );
        assert_eq!(fixture.receipt(), receipt);
    }
}

#[test]
fn missing_receipt_lock_or_proxy_is_reported_without_repair() {
    for (name, keeps_plan) in [
        ("receipt.json", false),
        ("owner.lock", true),
        ("proxy", true),
    ] {
        let fixture = Fixture::new();
        drop(fixture.acquire());
        fs::rename(
            fixture.directory().join(name),
            fixture.base.join(format!("retained-{name}")),
        )
        .unwrap();
        let before = tree(&fixture.base);
        let observed = fixture.observe();
        issue(&observed, "WORKSPACE_INCOMPLETE");
        assert_eq!(observed.plan.is_some(), keeps_plan);
        assert_eq!(observed.ownership, OwnershipObservation::Unknown);
        assert_eq!(tree(&fixture.base), before);
    }
}

#[test]
fn corrupt_or_mismatched_receipt_cannot_supply_a_plan_or_selector() {
    for change in ["json", "selector", "project", "root"] {
        let fixture = Fixture::new();
        drop(fixture.acquire());
        let mut receipt = fixture.receipt();
        let expected_code = match change {
            "json" => {
                fs::write(fixture.directory().join("receipt.json"), "{invalid").unwrap();
                "WORKSPACE_METADATA"
            }
            "selector" => {
                receipt["selector"] = json!("projection:../../unexpected");
                fixture.write_receipt(&receipt);
                "WORKSPACE_METADATA"
            }
            "project" => {
                receipt["project_id"] = json!("different-project");
                fixture.write_receipt(&receipt);
                "WORKSPACE_MISMATCH"
            }
            "root" => {
                receipt["root"] = json!(fixture.base.join("unchecked"));
                fixture.write_receipt(&receipt);
                "WORKSPACE_MISMATCH"
            }
            _ => unreachable!(),
        };
        let before = tree(&fixture.base);
        let observed = fixture.observe();
        issue(&observed, expected_code);
        assert!(observed.plan.is_none());
        assert!(observed.selector.is_none());
        assert_eq!(observed.ownership, OwnershipObservation::Unknown);
        assert_eq!(tree(&fixture.base), before);
    }
}

#[test]
fn changed_branch_and_missing_worktree_keep_validated_plan_but_not_availability() {
    for change in ["branch", "missing", "replacement"] {
        let fixture = Fixture::new();
        let binding = fixture.acquire();
        let root = binding.root().to_path_buf();
        drop(binding);
        match change {
            "branch" => {
                git(&root, &["switch", "-c", "different-branch"]);
            }
            "missing" | "replacement" => {
                fs::rename(&root, fixture.base.join("retained-worker")).unwrap();
                if change == "replacement" {
                    fs::create_dir(&root).unwrap();
                }
            }
            _ => unreachable!(),
        }
        let observed = fixture.observe();
        let expected = match change {
            "branch" => "WORKSPACE_BRANCH_MISMATCH",
            "missing" => "WORKSPACE_MISSING",
            _ => "WORKSPACE_MISMATCH",
        };
        issue(&observed, expected);
        assert_eq!(observed.plan.unwrap().root, root);
        assert_eq!(observed.ownership, OwnershipObservation::Unknown);
    }
}

#[test]
fn symlinks_private_permissions_and_hardlinks_are_rejected() {
    for target in ["receipt.json", "owner.lock", "proxy", "."] {
        for change in ["symlink", "permissions"] {
            let fixture = Fixture::new();
            drop(fixture.acquire());
            let path = if target == "." {
                fixture.directory()
            } else {
                fixture.directory().join(target)
            };
            if change == "symlink" {
                let retained = fixture.base.join("retained-entry");
                fs::rename(&path, &retained).unwrap();
                symlink(retained, &path).unwrap();
            } else {
                fs::set_permissions(
                    &path,
                    fs::Permissions::from_mode(if path.is_dir() { 0o755 } else { 0o644 }),
                )
                .unwrap();
            }
            let observed = fixture.observe();
            issue(
                &observed,
                if change == "symlink" {
                    "WORKSPACE_UNSAFE_PATH"
                } else {
                    "WORKSPACE_PRIVATE_STATE"
                },
            );
            assert_eq!(observed.ownership, OwnershipObservation::Unknown);
        }
    }
    for target in ["receipt.json", "owner.lock"] {
        let fixture = Fixture::new();
        drop(fixture.acquire());
        fs::hard_link(
            fixture.directory().join(target),
            fixture.base.join("hardlink"),
        )
        .unwrap();
        let observed = fixture.observe();
        issue(&observed, "WORKSPACE_PRIVATE_STATE");
        assert_eq!(observed.ownership, OwnershipObservation::Unknown);
    }
}

#[test]
fn invalid_caller_identity_or_repository_is_an_error() {
    let fixture = Fixture::new();
    for (project, work) in [
        ("../fixture", WORK),
        ("fixture", "../work"),
        ("fixture", "bad.lock"),
    ] {
        let error =
            WorktreeBinding::observe_with_root(&fixture.source, project, work, None).unwrap_err();
        assert_eq!(error.code, "WORKSPACE_IDENTITY");
    }
    assert!(WorktreeBinding::observe_with_root(&fixture.base, "fixture", WORK, None).is_err());
    assert!(
        WorktreeBinding::observe_with_root(
            &fixture.source,
            "fixture",
            WORK,
            Some(Path::new("relative"))
        )
        .is_err()
    );
    assert!(!fixture.source.join(".git/astral").exists());
}
