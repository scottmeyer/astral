#![cfg(unix)]

use ostk_gpt_cache::{
    hash,
    native_bundle::NativeBundle,
    project::Project,
    projection_save,
    recovery::{self, PendingSave, StagedWorker},
    workspace::{WorkerMetadata, WorktreeBinding},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const WORK: &str = "AST-recover";
const THREAD: &str = "01234567-89ab-cdef-0123-456789abcdef";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    target: PathBuf,
}

fn put(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
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

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("repo");
        fs::create_dir(&root).unwrap();
        put(
            &root,
            ".astral/project.toml",
            "schema_version=1\nid='fixture'\nname='Fixture'\ndescription='Recovery fixture'\ncore='core'\nprojections='projections'\nwork_items='work/items.jsonl'\n[identity]\nscope='local'\nruntime_bindings='external'\n[subsystems]\nweb='core/web'\n",
        );
        for doc in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            put(&root, &format!(".astral/core/{doc}"), "Fixture context.\n");
        }
        put(
            &root,
            ".astral/core/web/subsystem.toml",
            "schema_version=1\nid='web'\npurpose='Fixture'\nreadme='README.md'\nrules=[]\ndecisions=[]\nwork_items=[]\nprojection='seed'\ndepends_on=[]\n",
        );
        put(&root, ".astral/core/web/README.md", "Web context.\n");
        put(
            &root,
            ".astral/projections/seed/projection.toml",
            "schema_version=1\nid='seed'\nkind='fresh-context'\nsubsystems=['web']\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n",
        );
        put(
            &root,
            ".astral/projections/seed/handoff.md",
            "Fixture handoff.\n",
        );
        put(
            &root,
            ".astral/work/items.jsonl",
            &format!(
                "{}\n",
                json!({"schema_version":1,"id":WORK,"title":"Recover","status":"in_progress","depends_on":[],"acceptance":["Retain evidence"]})
            ),
        );
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "fixture"]);
        let binding = WorktreeBinding::acquire(&root, "fixture", "web", WORK).unwrap();
        let target = binding.root().to_owned();
        drop(binding);
        Self {
            _temp: temp,
            root,
            target,
        }
    }
    fn open(&self) -> WorktreeBinding {
        WorktreeBinding::open_existing(&self.root, "fixture", "web", WORK).unwrap()
    }
    fn set(&self, metadata: WorkerMetadata) {
        self.open().update_worker_metadata(metadata).unwrap();
    }
    fn receipt(&self) -> Vec<u8> {
        fs::read(
            self.root
                .join(format!(".git/astral/work-bindings/{WORK}/receipt.json")),
        )
        .unwrap()
    }
    fn digest(&self) -> String {
        Project::load(&self.target)
            .unwrap()
            .launch_context("web", Some(WORK))
            .unwrap()
            .current_context
            .selection_digest
    }
    fn ready(&self) -> WorkerMetadata {
        WorkerMetadata {
            context_initialized: true,
            thread_id: Some(THREAD.into()),
            model: Some("gpt-6-astra".into()),
            provider: Some("openai".into()),
            selection_digest: Some(self.digest()),
            selected_bundle_recorded: true,
            ..Default::default()
        }
    }
    fn bundle(&self, source_digest: &str) -> NativeBundle {
        let payload=serde_json::to_vec(&json!([{"type":"compaction","encrypted_content":"synthetic fixture, not provider memory"}])).unwrap();
        let manifest=serde_json::to_vec(&json!({"schema_version":1,"format":"astral-codex-native",
            "payload":{"file":"window.json","sha256":hash(&payload),"bytes":payload.len(),"item_count":1},
            "compatibility":{"runtime":"codex","runtime_version":"0.154.0","protocol":"openai-responses-lite","provider":"openai","model":"gpt-6-astra","requires_tool_rebinding":true,"identity_scope":"same-account"},
            "source":{"project_id":"fixture","revision":null,"dirty":true,"selection_sha256":source_digest,"history_sha256":"b".repeat(64)},
            "capture":{"boundary":"completed-compaction","history_complete":true,"last_checkpoint_index":0},"parents":[]})).unwrap();
        NativeBundle::validate(&manifest, &payload).unwrap()
    }
}

#[test]
fn context_acceptance_is_explicit_hash_checked_and_idempotent() {
    let f = Fixture::new();
    let before = f.receipt();
    put(&f.target, "keep.txt", "Uncommitted work.\n");
    let plan = recovery::plan(&f.root, WORK).unwrap();
    assert_eq!(plan.classification, "context_ready_for_review");
    assert_eq!(f.receipt(), before);
    assert_eq!(
        recovery::apply(&f.root, WORK, &"a".repeat(64))
            .unwrap_err()
            .code,
        "RECOVERY_STALE"
    );
    recovery::apply(&f.root, WORK, &plan.plan_sha256).unwrap();
    assert!(f.open().worker_metadata().context_initialized);
    let applied = f.receipt();
    assert_eq!(
        recovery::apply(&f.root, WORK, &plan.plan_sha256).unwrap()["outcome"],
        "already_recorded"
    );
    assert_eq!(f.receipt(), applied);
    assert_eq!(
        fs::read_to_string(f.target.join("keep.txt")).unwrap(),
        "Uncommitted work.\n"
    );
}

#[test]
fn changed_context_or_receipt_invalidates_review() {
    for receipt in [false, true] {
        let f = Fixture::new();
        let plan = recovery::plan(&f.root, WORK).unwrap();
        if receipt {
            let mut metadata = f.open().worker_metadata().clone();
            metadata.requires_tool_rebinding = true;
            f.set(metadata);
        } else {
            put(
                &f.target,
                ".astral/core/web/README.md",
                "Changed context.\n",
            );
        }
        let before = f.receipt();
        assert_eq!(
            recovery::apply(&f.root, WORK, &plan.plan_sha256)
                .unwrap_err()
                .code,
            "RECOVERY_STALE"
        );
        assert_eq!(f.receipt(), before);
    }
}

#[test]
fn unknown_staging_is_never_cleared_and_active_owner_blocks_repair() {
    let f = Fixture::new();
    f.set(WorkerMetadata {
        context_initialized: true,
        staging_in_progress: true,
        ..Default::default()
    });
    let plan = recovery::plan(&f.root, WORK).unwrap();
    assert_eq!(plan.classification, "staging_unknown");
    assert!(plan.repair.is_none());
    let before = f.receipt();
    assert_eq!(
        recovery::apply(&f.root, WORK, &plan.plan_sha256)
            .unwrap_err()
            .code,
        "RECOVERY_UNPROVEN"
    );
    let owner = f.open();
    assert_eq!(
        recovery::apply(&f.root, WORK, &plan.plan_sha256)
            .unwrap_err()
            .code,
        "RECOVERY_OWNED"
    );
    assert_eq!(f.receipt(), before);
    drop(owner);
}

#[test]
fn acknowledged_staging_restores_exact_worker_without_reinjection() {
    let f = Fixture::new();
    let original_digest = f.digest();
    f.set(WorkerMetadata {
        context_initialized: true,
        staging_in_progress: true,
        staged_worker: Some(StagedWorker {
            thread_id: THREAD.into(),
            model: "gpt-6-astra".into(),
            provider: "openai".into(),
            selection_digest: original_digest.clone(),
            seed_bundle_sha256: None,
            selected_bundle_sha256: None,
        }),
        ..Default::default()
    });
    put(
        &f.target,
        ".astral/core/web/README.md",
        "Later readable changes must be sent on real resume.\n",
    );
    let plan = recovery::plan(&f.root, WORK).unwrap();
    assert_eq!(plan.classification, "staging_acknowledged");
    recovery::apply(&f.root, WORK, &plan.plan_sha256).unwrap();
    let owner = f.open();
    let metadata = owner.worker_metadata();
    assert_eq!(metadata.thread_id.as_deref(), Some(THREAD));
    assert_eq!(
        metadata.selection_digest.as_deref(),
        Some(original_digest.as_str())
    );
    assert!(metadata.staged_worker.is_none());
    assert!(!metadata.staging_in_progress);
}

#[test]
fn published_save_repairs_bookkeeping_and_preserves_exact_native_bytes() {
    for projection in ["seed", "new-export"] {
        let f = Fixture::new();
        let source_digest = f.digest();
        let bundle = f.bundle(&source_digest);
        let bundle_hash = bundle.summary().manifest_sha256;
        let mut metadata = f.ready();
        metadata.requires_tool_rebinding = true;
        metadata.pending_save = Some(PendingSave {
            projection: projection.into(),
            bundle_sha256: bundle_hash.clone(),
            selection_digest: source_digest.clone(),
            selected_bundle_sha256: None,
        });
        f.set(metadata);
        let publication =
            projection_save::publish(&f.target, projection, &["web".into()], &bundle, None)
                .unwrap();
        let payload = fs::read(f.target.join(&publication.payload.path)).unwrap();
        let plan = recovery::plan(&f.root, WORK).unwrap();
        assert_eq!(plan.classification, "publication_validated");
        recovery::apply(&f.root, WORK, &plan.plan_sha256).unwrap();
        let owner = f.open();
        let metadata = owner.worker_metadata();
        assert_eq!(
            metadata.saved_bundle_sha256.as_deref(),
            Some(bundle_hash.as_str())
        );
        assert_eq!(
            metadata.selected_bundle_sha256.is_some(),
            projection == "seed"
        );
        assert_eq!(
            metadata.selection_digest.as_deref(),
            Some(source_digest.as_str())
        );
        assert!(metadata.requires_tool_rebinding);
        assert!(metadata.pending_save.is_none());
        assert_eq!(
            fs::read(f.target.join(&publication.payload.path)).unwrap(),
            payload
        );
    }
}

#[test]
fn absent_or_different_publication_cannot_clear_save_attempt() {
    for wrong_digest in [false, true] {
        let f = Fixture::new();
        let source_digest = f.digest();
        let bundle = f.bundle(&source_digest);
        let mut metadata = f.ready();
        metadata.requires_tool_rebinding = true;
        metadata.pending_save = Some(PendingSave {
            projection: "export".into(),
            bundle_sha256: "a".repeat(64),
            selection_digest: source_digest,
            selected_bundle_sha256: None,
        });
        f.set(metadata);
        let before = f.receipt();
        if wrong_digest {
            projection_save::publish(&f.target, "export", &["web".into()], &bundle, None).unwrap();
        }
        let plan = recovery::plan(&f.root, WORK).unwrap();
        assert!(plan.repair.is_none());
        assert_eq!(
            recovery::apply(&f.root, WORK, &plan.plan_sha256)
                .unwrap_err()
                .code,
            "RECOVERY_UNPROVEN"
        );
        assert_eq!(f.receipt(), before);
    }
}

#[test]
fn inventory_includes_orphan_binding_without_repair_or_home_scan() {
    let f = Fixture::new();
    put(&f.root, ".astral/work/items.jsonl", "");
    let before = f.receipt();
    let inventory = recovery::inventory(&f.root, None, 0, 32).unwrap();
    assert_eq!(inventory["bindings"][0]["work"], WORK);
    assert_eq!(inventory["bindings"][0]["in_current_register"], false);
    assert!(inventory["launches"].is_null());
    assert_eq!(f.receipt(), before);
    assert!(recovery::inventory(&f.root, None, 0, 65).is_err());
}

#[test]
fn invalid_context_and_foreign_project_are_not_acknowledged() {
    let f = Fixture::new();
    put(&f.target, ".astral/work/items.jsonl", "");
    assert!(recovery::plan(&f.root, WORK).unwrap().repair.is_none());
    let file = f.target.join(".astral/project.toml");
    let content = fs::read_to_string(&file)
        .unwrap()
        .replace("id='fixture'", "id='another'");
    fs::write(file, content).unwrap();
    assert!(recovery::plan(&f.root, WORK).unwrap().repair.is_none());
}

#[test]
fn recovery_cli_emits_formatted_plan_and_rejects_invalid_apply() {
    let f = Fixture::new();
    let result = Command::new(env!("CARGO_BIN_EXE_astral"))
        .args(["--root"])
        .arg(&f.root)
        .args(["recover", "--work", WORK])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(result.stdout.starts_with(b"{\n  "));
    let plan: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(plan["classification"], "context_ready_for_review");
    let failed = Command::new(env!("CARGO_BIN_EXE_astral"))
        .args(["--root"])
        .arg(&f.root)
        .args(["recover", "--work", WORK, "--apply", "invalid"])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(2));
}
