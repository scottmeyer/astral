#![cfg(unix)]
use astral::workspace::{BindingStatus, OwnershipObservation, WorktreeBinding};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
const WORK: &str = "AST-recovery";
const SELECTOR: &str = "projection:recovery-context";
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
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_AUTHOR_NAME", "Recovery Test")
        .env("GIT_AUTHOR_EMAIL", "recovery@example.invalid")
        .env("GIT_COMMITTER_NAME", "Recovery Test")
        .env("GIT_COMMITTER_EMAIL", "recovery@example.invalid");
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
        "git {args:?}: {}",
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
        fs::write(source.join("tracked.txt"), "base\n").unwrap();
        git(&source, &["add", "tracked.txt"]);
        git(&source, &["commit", "-m", "initial"]);
        Self {
            _temp: temp,
            base,
            source,
        }
    }
    fn directory(&self) -> PathBuf {
        self.source.join(".git/astral/work-bindings").join(WORK)
    }
    fn acquire(&self) -> WorktreeBinding {
        WorktreeBinding::acquire_with_root(&self.source, "fixture", SELECTOR, WORK, None).unwrap()
    }
    fn bytes(&self) -> Vec<u8> {
        fs::read(self.directory().join("receipt.json")).unwrap()
    }
    fn receipt(&self) -> Value {
        serde_json::from_slice(&self.bytes()).unwrap()
    }
    fn write(&self, value: &Value) {
        fs::write(
            self.directory().join("receipt.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    fn preview(&self) -> astral::workspace::RecoveryPreview {
        WorktreeBinding::recovery_preview_with_root(&self.source, "fixture", WORK, None).unwrap()
    }
    fn incomplete(&self, proof: bool, state: &str) -> PathBuf {
        let binding = self.acquire();
        let root = binding.root().to_path_buf();
        drop(binding);
        let mut receipt = self.receipt();
        assert!(
            receipt["creation_proof"].is_object(),
            "new creation must persist proof"
        );
        if !proof {
            receipt.as_object_mut().unwrap().remove("creation_proof");
        }
        receipt["status"] = json!(state);
        receipt["root_identity"] = Value::Null;
        receipt["git_dir"] = Value::Null;
        receipt["git_identity"] = Value::Null;
        receipt["last_error"] = if state == "failed" {
            json!("WORKSPACE_IO")
        } else {
            Value::Null
        };
        self.write(&receipt);
        root
    }
    fn recover(&self, expected: &str) -> astral::project::Result<WorktreeBinding> {
        WorktreeBinding::recover_creation_with_root(&self.source, "fixture", WORK, expected, None)
    }
}
fn code<T>(result: astral::project::Result<T>) -> &'static str {
    result.err().expect("must fail").code
}
#[test]
fn empty_inventory_and_preview_do_not_create_private_storage() {
    let fixture = Fixture::new();
    let inventory = WorktreeBinding::inventory_with_root(&fixture.source, "fixture", None).unwrap();
    assert!(inventory.work_ids.is_empty());
    assert!(inventory.issues.is_empty());
    let preview = fixture.preview();
    assert!(preview.receipt_sha256.is_none());
    assert!(preview.observation.plan.is_none());
    assert!(!preview.creation_recoverable);
    assert!(!fixture.source.join(".git/astral").exists());
    assert!(!fixture.base.join(".astral-worktrees").exists());
}
#[test]
fn inventory_includes_private_workers_without_a_work_register_and_reports_unsafe_entries() {
    let fixture = Fixture::new();
    drop(fixture.acquire());
    let before = fixture.bytes();
    let store = fixture.directory().parent().unwrap().to_path_buf();
    symlink(fixture.base.join("missing"), store.join("AST-orphan-link")).unwrap();
    let inventory = WorktreeBinding::inventory_with_root(&fixture.source, "fixture", None).unwrap();
    assert_eq!(inventory.work_ids, ["AST-orphan-link", WORK]);
    assert_eq!(inventory.issues[0].code, "WORKSPACE_UNSAFE_PATH");
    assert_eq!(fixture.bytes(), before);
    assert!(!fixture.source.join(".astral").exists());
}
#[test]
fn ready_preview_has_exact_digest_selector_and_is_not_a_reservation() {
    let fixture = Fixture::new();
    let binding = fixture.acquire();
    let preview = fixture.preview();
    assert_eq!(
        preview.receipt_sha256.as_deref(),
        Some(binding.receipt_digest().unwrap().as_str())
    );
    assert_eq!(preview.observation.selector.as_deref(), Some(SELECTOR));
    assert_eq!(preview.observation.ownership, OwnershipObservation::Busy);
    drop(binding);
    assert_eq!(
        fixture.preview().observation.ownership,
        OwnershipObservation::Available
    );
    drop(fixture.acquire());
}
#[test]
fn proven_preparing_and_failed_creation_recover_without_touching_dirty_checkout() {
    for state in ["preparing", "failed"] {
        let fixture = Fixture::new();
        let root = fixture.incomplete(true, state);
        fs::write(root.join("tracked.txt"), "local edits stay\n").unwrap();
        fs::write(root.join("untracked.txt"), "untracked stays\n").unwrap();
        let before = fixture.bytes();
        let hash = astral::hash(&before);
        let preview = fixture.preview();
        assert_eq!(preview.receipt_sha256.as_deref(), Some(hash.as_str()));
        assert!(preview.creation_recoverable, "{preview:?}");
        let binding = fixture.recover(&hash).unwrap();
        assert!(!binding.newly_created());
        assert_eq!(binding.root(), root);
        assert_eq!(fixture.receipt()["status"], "ready");
        assert_eq!(
            fs::read(
                fixture
                    .directory()
                    .join(format!("recovery-evidence/receipt-{hash}.json"))
            )
            .unwrap(),
            before
        );
        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "local edits stay\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("untracked.txt")).unwrap(),
            "untracked stays\n"
        );
        assert_eq!(
            fixture.preview().observation.ownership,
            OwnershipObservation::Busy
        );
        drop(binding);
        assert_eq!(
            fixture.preview().observation.plan.unwrap().status,
            BindingStatus::Ready
        );
        drop(fixture.acquire());
    }
}
#[test]
fn recorded_creation_promotion_records_plan_in_the_same_receipt_write() {
    let fixture = Fixture::new();
    fixture.incomplete(true, "preparing");
    let digest = astral::hash(&fixture.bytes());
    let plan = "a".repeat(64);
    let binding = WorktreeBinding::recover_creation_recorded_with_root(
        &fixture.source,
        "fixture",
        WORK,
        &digest,
        Some(&plan),
        None,
    )
    .unwrap();
    assert_eq!(
        binding.worker_metadata().last_recovery_sha256.as_deref(),
        Some(plan.as_str())
    );
    assert_eq!(
        fixture.receipt()["worker_metadata"]["last_recovery_sha256"],
        plan
    );
    assert_eq!(fixture.receipt()["status"], "ready");
}
#[test]
fn legacy_unproven_creation_is_diagnostic_only() {
    let fixture = Fixture::new();
    fixture.incomplete(false, "failed");
    let before = fixture.bytes();
    let preview = fixture.preview();
    assert!(!preview.creation_recoverable);
    assert!(
        preview
            .observation
            .issues
            .iter()
            .any(|e| e.code == "WORKSPACE_RECOVERY_UNPROVEN")
    );
    assert_eq!(
        code(fixture.recover(&astral::hash(&before))),
        "WORKSPACE_RECOVERY_UNPROVEN"
    );
    assert_eq!(fixture.bytes(), before);
    assert!(!fixture.directory().join("recovery-evidence").exists());
}
#[test]
fn changed_receipt_branch_or_checkout_identity_is_never_adopted() {
    for change in ["receipt", "branch", "directory"] {
        let fixture = Fixture::new();
        let root = fixture.incomplete(true, "preparing");
        let digest = astral::hash(&fixture.bytes());
        match change {
            "receipt" => {
                let mut receipt = fixture.receipt();
                receipt["status"] = json!("failed");
                receipt["last_error"] = json!("WORKSPACE_IO");
                fixture.write(&receipt);
            }
            "branch" => {
                git(&root, &["switch", "-c", "different"]);
            }
            _ => {
                fs::rename(&root, fixture.base.join("retained")).unwrap();
                fs::create_dir(&root).unwrap();
            }
        }
        let before = fixture.bytes();
        let error = code(fixture.recover(&digest));
        assert!(
            matches!(
                error,
                "WORKSPACE_CHANGED" | "WORKSPACE_BRANCH_MISMATCH" | "WORKSPACE_MISMATCH"
            ),
            "{error}"
        );
        assert_eq!(fixture.bytes(), before);
        assert!(!fixture.directory().join("recovery-evidence").exists());
    }
}
#[test]
fn ownership_blocks_apply_and_uncertain_staging_cannot_be_reset_as_creation() {
    let fixture = Fixture::new();
    let binding = fixture.acquire();
    let digest = binding.receipt_digest().unwrap();
    assert_eq!(code(fixture.recover(&digest)), "WORKSPACE_OWNED");
    drop(binding);
    let mut receipt = fixture.receipt();
    receipt["status"] = json!("preparing");
    receipt["root_identity"] = Value::Null;
    receipt["git_dir"] = Value::Null;
    receipt["git_identity"] = Value::Null;
    receipt["worker_metadata"]["staging_in_progress"] = json!(true);
    fixture.write(&receipt);
    let before = fixture.bytes();
    assert_eq!(
        code(fixture.recover(&astral::hash(&before))),
        "WORKSPACE_METADATA"
    );
    assert_eq!(fixture.bytes(), before);
}
#[test]
fn metadata_repair_archives_exact_old_bytes_and_enforces_both_cas_checks() {
    let fixture = Fixture::new();
    let mut binding = fixture.acquire();
    let before = fixture.bytes();
    let digest = binding.receipt_digest().unwrap();
    let mut metadata = binding.worker_metadata().clone();
    metadata.context_initialized = true;
    assert_eq!(
        code(binding.update_worker_metadata_expected(metadata.clone(), &"0".repeat(64))),
        "WORKSPACE_CHANGED"
    );
    assert!(!fixture.directory().join("recovery-evidence").exists());
    binding
        .update_worker_metadata_expected(metadata.clone(), &digest)
        .unwrap();
    assert_eq!(
        fs::read(
            fixture
                .directory()
                .join(format!("recovery-evidence/receipt-{digest}.json"))
        )
        .unwrap(),
        before
    );
    assert_ne!(binding.receipt_digest().unwrap(), digest);
    assert_eq!(
        fixture.receipt()["worker_metadata"]["context_initialized"],
        true
    );
    let current = binding.receipt_digest().unwrap();
    fs::write(
        fixture.directory().join("receipt.json"),
        b"external changed receipt",
    )
    .unwrap();
    assert_eq!(
        code(binding.update_worker_metadata_expected(metadata, &current)),
        "WORKSPACE_CHANGED"
    );
    assert_eq!(fixture.bytes(), b"external changed receipt");
}
#[test]
fn unsafe_receipt_and_archive_are_rejected_and_preserved() {
    for target in ["receipt", "archive"] {
        let fixture = Fixture::new();
        let mut binding = fixture.acquire();
        let digest = binding.receipt_digest().unwrap();
        let mut metadata = binding.worker_metadata().clone();
        metadata.context_initialized = true;
        if target == "receipt" {
            fs::set_permissions(
                fixture.directory().join("receipt.json"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        } else {
            symlink(&fixture.base, fixture.directory().join("recovery-evidence")).unwrap();
        }
        let before = fixture.bytes();
        let error = code(binding.update_worker_metadata_expected(metadata, &digest));
        assert!(matches!(
            error,
            "WORKSPACE_PRIVATE_STATE" | "WORKSPACE_UNSAFE_PATH"
        ));
        assert_eq!(fixture.bytes(), before);
    }
}

#[test]
fn legacy_receipt_digest_and_archive_preserve_original_formatting_and_omitted_defaults() {
    let fixture = Fixture::new();
    drop(fixture.acquire());
    let mut receipt = fixture.receipt();
    receipt.as_object_mut().unwrap().remove("creation_proof");
    for name in [
        "staged_worker",
        "pending_save",
        "last_recovery_sha256",
        "selected_bundle_recorded",
    ] {
        receipt["worker_metadata"]
            .as_object_mut()
            .unwrap()
            .remove(name);
    }
    let mut original = serde_json::to_vec_pretty(&receipt).unwrap();
    original.push(b'\n');
    fs::write(fixture.directory().join("receipt.json"), &original).unwrap();
    let mut binding = fixture.acquire();
    let digest = astral::hash(&original);
    assert_eq!(binding.receipt_digest().unwrap(), digest);
    let mut metadata = binding.worker_metadata().clone();
    metadata.context_initialized = true;
    binding
        .update_worker_metadata_expected(metadata, &digest)
        .unwrap();
    assert_eq!(
        fs::read(
            fixture
                .directory()
                .join(format!("recovery-evidence/receipt-{digest}.json"))
        )
        .unwrap(),
        original
    );
}

#[test]
fn inventory_is_bounded_without_creating_any_binding_or_inspecting_runtime_data() {
    let fixture = Fixture::new();
    drop(fixture.acquire());
    let store = fixture.directory().parent().unwrap().to_path_buf();
    for index in 0..astral::workspace::MAX_BINDING_INVENTORY {
        let directory = store.join(format!("AST-orphan-{index:04}"));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert_eq!(
        code(WorktreeBinding::inventory_with_root(
            &fixture.source,
            "fixture",
            None
        )),
        "WORKSPACE_INVENTORY_LIMIT"
    );
    assert_eq!(fixture.receipt()["status"], "ready");
}

#[test]
fn interrupted_pending_archive_is_retained_and_does_not_block_retry() {
    let fixture = Fixture::new();
    let mut binding = fixture.acquire();
    let before = fixture.bytes();
    let digest = binding.receipt_digest().unwrap();
    let directory = fixture.directory().join("recovery-evidence");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let pending = directory.join(format!("pending-{digest}-{}.json", "a".repeat(64)));
    fs::write(&pending, b"partial interrupted receipt").unwrap();
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600)).unwrap();
    let mut metadata = binding.worker_metadata().clone();
    metadata.context_initialized = true;
    binding
        .update_worker_metadata_expected(metadata, &digest)
        .unwrap();
    assert_eq!(fs::read(&pending).unwrap(), b"partial interrupted receipt");
    assert_eq!(
        fs::read(directory.join(format!("receipt-{digest}.json"))).unwrap(),
        before
    );
    assert_eq!(
        fixture.receipt()["worker_metadata"]["context_initialized"],
        true
    );
}
