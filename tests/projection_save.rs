#![cfg(unix)]

use astral::native_bundle::NativeBundle;
use astral::project::Project;
use astral::projection_save::{carry_work_file, publish, validate_target};
use serde_json::json;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn put(root: &Path, path: &str, content: impl AsRef<[u8]>) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, content).unwrap();
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn fixture() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]);
    put(
        root.path(),
        ".astral/project.toml",
        "schema_version = 1\nid = \"save-fixture\"\nname = \"Save fixture\"\ndescription = \"Synthetic project\"\ncore = \"core\"\nprojections = \"projections\"\nwork_items = \"work/items.jsonl\"\n[identity]\nscope = \"local\"\nruntime_bindings = \"external\"\n[subsystems]\nweb = \"core/web\"\nother = \"core/other\"\n",
    );
    for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
        put(
            root.path(),
            &format!(".astral/core/{name}"),
            "Current fixture document.\n",
        );
    }
    for id in ["web", "other"] {
        put(
            root.path(),
            &format!(".astral/core/{id}/subsystem.toml"),
            format!(
                "schema_version = 1\nid = {id:?}\npurpose = \"Fixture\"\nreadme = \"README.md\"\nrules = []\ndecisions = []\nwork_items = []\nprojection = \"start\"\ndepends_on = []\n"
            ),
        );
        put(
            root.path(),
            &format!(".astral/core/{id}/README.md"),
            "Current scoped instructions.\n",
        );
    }
    put(
        root.path(),
        ".astral/projections/start/projection.toml",
        "schema_version = 1\nschema_status = \"Keep this note\"\nid = \"start\"\nkind = \"fresh-context\"\nsubsystems = [\"web\"]\nhandoff = \"notes/handoff.md\"\nnative_payload_in_repository = false\n[[sources]]\nid = \"historic\"\nkind = \"design\"\nscope = \"Keep source metadata\"\navailability = \"Local\"\n",
    );
    put(
        root.path(),
        ".astral/projections/start/notes/handoff.md",
        "Preserve this existing handoff exactly.\n",
    );
    put(root.path(), ".astral/work/items.jsonl", "");
    Project::load(root.path()).unwrap();
    root
}

fn bundle(opaque: &str) -> NativeBundle {
    let payload =
        serde_json::to_vec(&json!([{ "type": "compaction", "encrypted_content": opaque }]))
            .unwrap();
    let manifest = serde_json::to_vec(&json!({
        "schema_version":1,"format":"astral-codex-native",
        "payload":{"file":"window.json","sha256":astral::hash(&payload),"bytes":payload.len(),"item_count":1},
        "compatibility":{"runtime":"codex","runtime_version":"0.154.0","protocol":"openai-responses-lite","provider":"openai","model":"gpt-6-astra","requires_tool_rebinding":true,"identity_scope":"same-account"},
        "source":{"project_id":"save-fixture","revision":null,"dirty":true,"selection_sha256":"a".repeat(64),"history_sha256":"b".repeat(64)},
        "capture":{"boundary":"completed-turn","history_complete":true,"last_checkpoint_index":0},"parents":[]
    })).unwrap();
    NativeBundle::validate(&manifest, &payload).unwrap()
}

fn scope() -> Vec<String> {
    vec!["web".into()]
}

fn dependency_fixture() -> TempDir {
    let root = fixture();
    let project = fs::read_to_string(root.path().join(".astral/project.toml")).unwrap();
    put(
        root.path(),
        ".astral/project.toml",
        format!("{project}common = \"core/common\"\nfoundation = \"core/foundation\"\n"),
    );
    let web_path = ".astral/core/web/subsystem.toml";
    let web = fs::read_to_string(root.path().join(web_path)).unwrap();
    put(
        root.path(),
        web_path,
        web.replace("depends_on = []", "depends_on = [\"common\"]"),
    );
    for (id, dependencies) in [("common", vec!["foundation"]), ("foundation", vec![])] {
        put(
            root.path(),
            &format!(".astral/core/{id}/subsystem.toml"),
            format!(
                "schema_version = 1\nid = {id:?}\npurpose = \"Dependency fixture\"\nreadme = \"README.md\"\nrules = []\ndecisions = []\nwork_items = []\nprojection = \"start\"\ndepends_on = {}\n",
                serde_json::to_string(&dependencies).unwrap()
            ),
        );
        put(
            root.path(),
            &format!(".astral/core/{id}/README.md"),
            "Declared dependency.\n",
        );
    }
    Project::load(root.path()).unwrap();
    root
}

#[test]
fn dependency_expanded_save_scope_matches_and_preserves_authored_projection_scope() {
    let root = dependency_fixture();
    let expanded = vec!["common".into(), "foundation".into(), "web".into()];
    let initial = Project::load(root.path())
        .unwrap()
        .inspect("projection:start", None)
        .unwrap();
    assert_eq!(initial["selection"]["subsystems"], json!(expanded));
    let handoff = fs::read(
        root.path()
            .join(".astral/projections/start/notes/handoff.md"),
    )
    .unwrap();
    // Both the authored shorthand and the launcher's full dependency closure
    // select the same project scope, including a transitive dependency.
    for selected in [&expanded, &scope()] {
        validate_target(root.path(), "start", selected).unwrap();
        publish(
            root.path(),
            "start",
            selected,
            &bundle("dependency-scope"),
            None,
        )
        .unwrap();
        let inspection = Project::load(root.path())
            .unwrap()
            .inspect("projection:start", None)
            .unwrap();
        assert_eq!(
            inspection["metadata"]["projection"]["subsystems"],
            json!(["web"])
        );
        assert_eq!(inspection["selection"]["subsystems"], json!(expanded));
        assert_eq!(
            inspection["metadata"]["projection"]["schema_status"],
            "Keep this note"
        );
        assert_eq!(
            inspection["metadata"]["projection"]["sources"],
            initial["metadata"]["projection"]["sources"]
        );
    }
    assert_eq!(
        fs::read(
            root.path()
                .join(".astral/projections/start/notes/handoff.md")
        )
        .unwrap(),
        handoff
    );
}

#[test]
fn dependency_resolution_still_rejects_missing_or_additional_save_scope() {
    let root = dependency_fixture();
    let manifest_path = root
        .path()
        .join(".astral/projections/start/projection.toml");
    let before = fs::read(&manifest_path).unwrap();
    for selected in [
        vec!["common".into(), "foundation".into()],
        vec![
            "common".into(),
            "foundation".into(),
            "web".into(),
            "other".into(),
        ],
    ] {
        assert_eq!(
            validate_target(root.path(), "start", &selected)
                .unwrap_err()
                .code,
            "PUBLICATION_CONFLICT"
        );
        assert_eq!(
            publish(
                root.path(),
                "start",
                &selected,
                &bundle("rejected-scope"),
                None
            )
            .unwrap_err()
            .code,
            "PUBLICATION_CONFLICT"
        );
        assert_eq!(fs::read(&manifest_path).unwrap(), before);
    }
    assert!(
        !root
            .path()
            .join(".astral/projections/start/bundles")
            .exists()
    );
}

#[test]
fn new_projection_publishes_complete_native_artifacts_and_keeps_preview_outside_git_status() {
    let root = fixture();
    validate_target(root.path(), "saved", &scope()).unwrap();
    let native = bundle("SYNTHETIC_PRIVATE_CHECKPOINT");
    let receipt = publish(
        root.path(),
        "saved",
        &scope(),
        &native,
        Some("Current handoff.\n"),
    )
    .unwrap();
    assert_eq!(
        fs::read(root.path().join(&receipt.payload.path)).unwrap(),
        native.payload_bytes()
    );
    assert_eq!(receipt.payload.sha256, astral::hash(native.payload_bytes()));
    assert_eq!(
        receipt.bundle_manifest.sha256,
        astral::hash(native.manifest_bytes())
    );
    assert!(receipt.bundle_manifest.path.contains(&format!(
        "bundles/{}/manifest.json",
        receipt.bundle_manifest.sha256
    )));
    let project = Project::load(root.path()).unwrap();
    assert_eq!(project.validate().unwrap()["native_artifacts"], 1);
    assert!(
        project
            .launch_context("projection:saved", None)
            .unwrap()
            .native
            .is_some()
    );
    assert!(
        !project
            .inspect("projection:saved", None)
            .unwrap()
            .to_string()
            .contains("SYNTHETIC_PRIVATE_CHECKPOINT")
    );
    assert!(
        !git(
            root.path(),
            &["status", "--porcelain", "--untracked-files=all"]
        )
        .contains("publication-staging")
    );
    assert!(root.path().join(".git/astral/publication-staging").is_dir());
    assert_eq!(
        fs::metadata(root.path().join(".git/astral/publication-staging"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn advancing_an_existing_projection_preserves_metadata_handoff_and_old_bundles() {
    let root = fixture();
    let original_handoff = fs::read(
        root.path()
            .join(".astral/projections/start/notes/handoff.md"),
    )
    .unwrap();
    let first = bundle("first-synthetic-checkpoint");
    let second = bundle("second-synthetic-checkpoint");
    let old = publish(root.path(), "start", &scope(), &first, None).unwrap();
    let next = publish(root.path(), "start", &scope(), &second, None).unwrap();
    assert_ne!(old.bundle_manifest.sha256, next.bundle_manifest.sha256);
    assert_eq!(
        fs::read(root.path().join(old.payload.path)).unwrap(),
        first.payload_bytes()
    );
    assert_eq!(
        fs::read(root.path().join(next.payload.path)).unwrap(),
        second.payload_bytes()
    );
    assert_eq!(
        fs::read(
            root.path()
                .join(".astral/projections/start/notes/handoff.md")
        )
        .unwrap(),
        original_handoff
    );
    let metadata = Project::load(root.path())
        .unwrap()
        .inspect("projection:start", None)
        .unwrap();
    assert_eq!(
        metadata["metadata"]["projection"]["schema_status"],
        "Keep this note"
    );
    assert_eq!(
        metadata["metadata"]["projection"]["sources"][0]["scope"],
        "Keep source metadata"
    );
    assert_eq!(
        metadata["metadata"]["projection"]["handoff"],
        "notes/handoff.md"
    );
    assert_eq!(
        metadata["native_artifacts"][0]["bundle"]["manifest_sha256"],
        next.bundle_manifest.sha256
    );
    // Repeated publication verifies immutable contents instead of replacing them.
    let repeated = publish(root.path(), "start", &scope(), &second, None).unwrap();
    assert_eq!(repeated.bundle_manifest.sha256, next.bundle_manifest.sha256);
}

#[test]
fn configured_projection_directory_is_respected() {
    let root = fixture();
    fs::rename(
        root.path().join(".astral/projections"),
        root.path().join(".astral/saved-contexts"),
    )
    .unwrap();
    let manifest = fs::read_to_string(root.path().join(".astral/project.toml"))
        .unwrap()
        .replace(
            "projections = \"projections\"",
            "projections = \"saved-contexts\"",
        );
    put(root.path(), ".astral/project.toml", manifest);
    let receipt = publish(
        root.path(),
        "saved",
        &scope(),
        &bundle("configured-path"),
        None,
    )
    .unwrap();
    assert!(
        receipt
            .payload
            .path
            .starts_with(".astral/saved-contexts/saved/")
    );
    assert!(!root.path().join(".astral/projections").exists());
    Project::load(root.path()).unwrap().validate().unwrap();
}

#[test]
fn conflicting_scope_invalid_names_and_handoff_replacements_fail_before_publication() {
    let root = fixture();
    let before = fs::read(
        root.path()
            .join(".astral/projections/start/projection.toml"),
    )
    .unwrap();
    for name in ["../escape", "/absolute", "", "..", "bad/name"] {
        assert_eq!(
            validate_target(root.path(), name, &scope())
                .unwrap_err()
                .code,
            "INVALID_ID"
        );
    }
    assert_eq!(
        validate_target(root.path(), "saved", &["unknown".into()])
            .unwrap_err()
            .code,
        "MISSING_REFERENCE"
    );
    assert_eq!(
        validate_target(root.path(), "start", &["other".into()])
            .unwrap_err()
            .code,
        "PUBLICATION_CONFLICT"
    );
    assert_eq!(
        publish(
            root.path(),
            "start",
            &scope(),
            &bundle("rejected"),
            Some("replace?")
        )
        .unwrap_err()
        .code,
        "PUBLICATION_CONFLICT"
    );
    assert_eq!(
        fs::read(
            root.path()
                .join(".astral/projections/start/projection.toml")
        )
        .unwrap(),
        before
    );
}

#[test]
fn immutable_conflict_retains_the_old_reference_and_private_failed_preview() {
    let root = fixture();
    let old = publish(root.path(), "start", &scope(), &bundle("old"), None).unwrap();
    let before = fs::read(root.path().join(&old.projection_manifest.path)).unwrap();
    let candidate = bundle("candidate");
    let digest = astral::hash(candidate.manifest_bytes());
    let dir = format!(".astral/projections/start/bundles/{digest}");
    put(
        root.path(),
        &format!("{dir}/manifest.json"),
        "existing foreign bytes",
    );
    put(
        root.path(),
        &format!("{dir}/window.json"),
        candidate.payload_bytes(),
    );
    let retained_before = fs::read_dir(root.path().join(".git/astral/publication-staging"))
        .unwrap()
        .count();
    assert_eq!(
        publish(root.path(), "start", &scope(), &candidate, None)
            .unwrap_err()
            .code,
        "IMMUTABLE_BUNDLE_CONFLICT"
    );
    assert_eq!(
        fs::read(root.path().join(old.projection_manifest.path)).unwrap(),
        before
    );
    assert_eq!(
        fs::read(root.path().join(format!("{dir}/manifest.json"))).unwrap(),
        b"existing foreign bytes"
    );
    assert_eq!(
        fs::read_dir(root.path().join(".git/astral/publication-staging"))
            .unwrap()
            .count(),
        retained_before + 1
    );
    Project::load(root.path()).unwrap();
}

#[test]
fn symlink_and_hardlink_destinations_never_receive_native_bytes() {
    let root = fixture();
    let outside = tempfile::tempdir().unwrap();
    symlink(
        outside.path(),
        root.path().join(".astral/projections/saved"),
    )
    .unwrap();
    assert!(publish(root.path(), "saved", &scope(), &bundle("secret"), None).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);

    let root = fixture();
    fs::hard_link(
        root.path().join(".astral/core/RUN.md"),
        root.path().join("hardlinked-document"),
    )
    .unwrap();
    assert_eq!(
        publish(root.path(), "saved", &scope(), &bundle("secret"), None)
            .unwrap_err()
            .code,
        "PUBLICATION_INSECURE"
    );
    assert!(!root.path().join(".astral/projections/saved").exists());
}

#[test]
fn cooperating_writers_and_insecure_staging_are_rejected() {
    let root = fixture();
    let lock = fs::File::open(root.path().join(".astral")).unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    assert_eq!(
        publish(root.path(), "saved", &scope(), &bundle("locked"), None)
            .unwrap_err()
            .code,
        "PUBLICATION_BUSY"
    );
    fs2::FileExt::unlock(&lock).unwrap();
    drop(lock);
    validate_target(root.path(), "saved", &scope()).unwrap();
    fs::set_permissions(
        root.path().join(".git/astral/publication-staging"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert_eq!(
        validate_target(root.path(), "saved", &scope())
            .unwrap_err()
            .code,
        "PUBLICATION_INSECURE"
    );
}

#[test]
fn foreign_project_bundle_and_native_document_overlap_fail_without_new_reference() {
    let root = fixture();
    let native = bundle("valid-synthetic-payload");
    let mut manifest: serde_json::Value = serde_json::from_slice(native.manifest_bytes()).unwrap();
    manifest["source"]["project_id"] = json!("another-project");
    let foreign = NativeBundle::validate(
        &serde_json::to_vec(&manifest).unwrap(),
        native.payload_bytes(),
    )
    .unwrap();
    assert_eq!(
        publish(root.path(), "saved", &scope(), &foreign, None)
            .unwrap_err()
            .code,
        "NATIVE_PROJECT_MISMATCH"
    );
    let handoff = std::str::from_utf8(native.payload_bytes()).unwrap();
    assert_eq!(
        publish(root.path(), "saved", &scope(), &native, Some(handoff))
            .unwrap_err()
            .code,
        "NATIVE_DOCUMENT_CONFLICT"
    );
    assert!(!root.path().join(".astral/projections/saved").exists());
}

fn record(id: &str, dependencies: &[&str]) -> serde_json::Value {
    json!({"schema_version":1,"id":id,"title":"Current task","status":"open","depends_on":dependencies,"acceptance":["Keep exact source record bytes"]})
}

#[test]
fn whole_work_file_carry_preserves_bytes_dependencies_and_only_changes_work_register() {
    let source = fixture();
    git(source.path(), &["add", ".astral"]);
    git(source.path(), &["commit", "--quiet", "-m", "fixture"]);
    let parent = tempfile::tempdir().unwrap();
    let target = parent.path().join("worker");
    git(
        source.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "fixture-worker",
            target.to_str().unwrap(),
        ],
    );
    let original = fs::read(target.join(".astral/core/RUN.md")).unwrap();
    put(
        source.path(),
        ".astral/core/RUN.md",
        "Do not copy this source-only edit.\n",
    );
    let work = format!(
        "{}\r\n  {}\n",
        record("AST-A", &["AST-B"]),
        record("AST-B", &[])
    );
    put(source.path(), ".astral/work/items.jsonl", &work);
    carry_work_file(source.path(), &target).unwrap();
    assert_eq!(
        fs::read(target.join(".astral/work/items.jsonl")).unwrap(),
        work.as_bytes()
    );
    assert_eq!(
        fs::read(target.join(".astral/core/RUN.md")).unwrap(),
        original
    );
    Project::load(&target)
        .unwrap()
        .inspect("web", Some("AST-A"))
        .unwrap();
    assert!(
        !git(&target, &["status", "--porcelain", "--untracked-files=all"])
            .contains("publication-staging")
    );
}

#[test]
fn incompatible_work_layout_or_invalid_source_never_changes_target_work() {
    let source = fixture();
    let target = fixture();
    let before = fs::read(target.path().join(".astral/work/items.jsonl")).unwrap();
    put(
        source.path(),
        ".astral/work/items.jsonl",
        "malformed work\n",
    );
    assert!(carry_work_file(source.path(), target.path()).is_err());
    assert_eq!(
        fs::read(target.path().join(".astral/work/items.jsonl")).unwrap(),
        before
    );
    put(source.path(), ".astral/work/items.jsonl", "");
    put(source.path(), ".astral/work/alternate.jsonl", "");
    let config = fs::read_to_string(source.path().join(".astral/project.toml"))
        .unwrap()
        .replace("work/items.jsonl", "work/alternate.jsonl");
    put(source.path(), ".astral/project.toml", config);
    assert_eq!(
        carry_work_file(source.path(), target.path())
            .unwrap_err()
            .code,
        "WORK_CARRY_CONFLICT"
    );
    assert_eq!(
        fs::read(target.path().join(".astral/work/items.jsonl")).unwrap(),
        before
    );
}

#[test]
fn aggregate_native_limit_is_checked_before_new_reference_is_visible() {
    let root = fixture();
    for (name, marker) in [("first", "a"), ("second", "b")] {
        publish(
            root.path(),
            name,
            &scope(),
            &bundle(&marker.repeat(6 * 1024 * 1024)),
            None,
        )
        .unwrap();
    }
    let before = Project::load(root.path()).unwrap().validate().unwrap();
    let too_large = bundle(&"c".repeat(6 * 1024 * 1024));
    assert_eq!(
        publish(root.path(), "third", &scope(), &too_large, None)
            .unwrap_err()
            .code,
        "LIMIT_EXCEEDED"
    );
    assert!(!root.path().join(".astral/projections/third").exists());
    assert_eq!(
        Project::load(root.path()).unwrap().validate().unwrap(),
        before
    );
}

#[test]
fn git_common_directory_symlink_is_not_used_for_private_staging() {
    let root = fixture();
    let external = tempfile::tempdir().unwrap();
    fs::rename(root.path().join(".git"), external.path().join("git-data")).unwrap();
    symlink(external.path().join("git-data"), root.path().join(".git")).unwrap();
    assert!(validate_target(root.path(), "saved", &scope()).is_err());
    assert!(!external.path().join("git-data/astral").exists());
}
