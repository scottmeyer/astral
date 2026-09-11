#![cfg(unix)]

use astral::project::{Limits, Project};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const PROJECT_ID: &str = "native-project-fixture";
const OPAQUE_MARKER: &str = "SYNTHETIC_OPAQUE_NATIVE_MARKER_NOT_FOR_INSPECTION";
const TAIL_MARKER: &str = "SYNTHETIC_READABLE_TAIL_NOT_FOR_INSPECTION";

fn put(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, bytes).unwrap();
}

fn replace(root: &Path, path: &str, before: &str, after: &str) {
    let bytes = fs::read_to_string(root.join(path)).unwrap();
    assert!(
        bytes.contains(before),
        "missing replacement target {before:?}"
    );
    put(root, path, bytes.replace(before, after));
}

fn projection_path(id: &str) -> String {
    format!(".astral/projections/{id}/projection.toml")
}

fn manifest_path(id: &str) -> String {
    format!(".astral/projections/{id}/bundles/demo/manifest.json")
}

fn payload_path(id: &str) -> String {
    format!(".astral/projections/{id}/bundles/demo/window.json")
}

fn subsystem(root: &Path, id: &str, projection: &str, dependencies: &[&str]) {
    put(
        root,
        &format!(".astral/core/{id}/subsystem.toml"),
        format!(
            "schema_version = 1\nid = {id:?}\npurpose = \"Synthetic subsystem\"\nreadme = \"README.md\"\nrules = []\ndecisions = []\nwork_items = []\nprojection = {projection:?}\ndepends_on = {}\n",
            serde_json::to_string(dependencies).unwrap()
        ),
    );
    put(
        root,
        &format!(".astral/core/{id}/README.md"),
        "Fixture documentation.\n",
    );
}

fn write_bundle(root: &Path, id: &str, subsystem: &str, project: &str, opaque: &str) {
    let payload = serde_json::to_vec(&json!([
        {"type": "compaction", "encrypted_content": opaque},
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": TAIL_MARKER}]}
    ])).unwrap();
    let manifest = serde_json::to_vec(&json!({
        "schema_version": 1,
        "format": "astral-codex-native",
        "payload": {"file": "window.json", "sha256": astral::hash(&payload), "bytes": payload.len(), "item_count": 2},
        "compatibility": {"runtime": "codex", "runtime_version": "0.154.0", "protocol": "openai-responses-lite", "provider": "openai", "model": "gpt-6-astra", "requires_tool_rebinding": true, "identity_scope": "same-account"},
        "source": {"project_id": project, "revision": null, "dirty": false, "selection_sha256": "a".repeat(64), "history_sha256": "b".repeat(64)},
        "capture": {"boundary": "completed-turn", "history_complete": true, "last_checkpoint_index": 0},
        "parents": []
    })).unwrap();
    put(root, &payload_path(id), &payload);
    put(root, &manifest_path(id), &manifest);
    put(
        root,
        &projection_path(id),
        format!(
            "schema_version = 1\nid = {id:?}\nkind = \"native-checkpoint\"\nsubsystems = [{subsystem:?}]\nhandoff = \"handoff.md\"\nnative_payload_in_repository = true\nsources = []\n[native_bundle]\nmanifest = \"bundles/demo/manifest.json\"\nsha256 = {:?}\n",
            astral::hash(&manifest)
        ),
    );
    put(
        root,
        &format!(".astral/projections/{id}/handoff.md"),
        "Synthetic native artifact; no runtime bindings.\n",
    );
}

fn make_readable(root: &Path, id: &str) {
    let path = projection_path(id);
    let original = fs::read_to_string(root.join(&path)).unwrap();
    let readable = original
        .split("[native_bundle]")
        .next()
        .unwrap()
        .replace("kind = \"native-checkpoint\"", "kind = \"fresh-context\"")
        .replace(
            "native_payload_in_repository = true",
            "native_payload_in_repository = false",
        );
    put(root, &path, readable);
}

fn fixture() -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    put(
        root,
        ".astral/project.toml",
        format!(
            r#"schema_version = 1
id = "{PROJECT_ID}"
name = "Native project fixture"
description = "Synthetic native artifact integration coverage"
core = "core"
projections = "projections"
work_items = "work/items.jsonl"
[identity]
scope = "repository-local logical names"
runtime_bindings = "private and external to Git"
[subsystems]
web = "core/web"
authentication = "core/authentication"
other = "core/other"
"#
        ),
    );
    for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
        put(
            root,
            &format!(".astral/core/{name}"),
            "Synthetic repository documentation.\n",
        );
    }
    put(root, ".astral/work/items.jsonl", "");
    subsystem(root, "web", "web-native", &["authentication"]);
    subsystem(root, "authentication", "auth-native", &[]);
    subsystem(root, "other", "other-native", &[]);
    for (id, subsystem) in [
        ("web-native", "web"),
        ("auth-native", "authentication"),
        ("other-native", "other"),
    ] {
        write_bundle(root, id, subsystem, PROJECT_ID, OPAQUE_MARKER);
    }
    directory
}

fn inspect(root: &Path, selector: &str) -> Value {
    Project::load(root)
        .unwrap()
        .inspect(selector, None)
        .unwrap()
}

fn load_error(root: &Path) -> String {
    Project::load(root)
        .err()
        .expect("fixture must fail")
        .code
        .to_owned()
}

fn native_ids(inspection: &Value) -> Vec<&str> {
    inspection["native_artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|artifact| artifact["projection"].as_str().unwrap())
        .collect()
}

fn command(root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_astral"))
        .arg("--json")
        .arg("--root")
        .arg(root)
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn inspection_exposes_integrity_metadata_without_native_or_tail_contents() {
    let root = fixture();
    let selected = inspect(root.path(), "projection:web-native");
    assert_eq!(native_ids(&selected), ["auth-native", "web-native"]);
    assert_eq!(selected["native_binding"]["state"], "UNBOUND");
    assert_eq!(selected["native_binding"]["launch"], false);
    assert_eq!(selected["native_binding"]["plaintext_substitution"], false);
    for artifact in selected["native_artifacts"].as_array().unwrap() {
        let id = artifact["projection"].as_str().unwrap();
        assert_eq!(artifact["availability"], "validated");
        assert_eq!(artifact["runtime_binding"], "unbound");
        assert!(artifact["bundle"].is_object());
        for (field, path) in [
            ("manifest", manifest_path(id)),
            ("payload", payload_path(id)),
        ] {
            let bytes = fs::read(root.path().join(&path)).unwrap();
            assert_eq!(artifact[field]["path"], path);
            assert_eq!(artifact[field]["sha256"], astral::hash(&bytes));
            assert_eq!(artifact[field]["bytes"], bytes.len());
            assert!(
                selected["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|source| {
                        source["path"] == path && source["sha256"] == artifact[field]["sha256"]
                    })
            );
        }
    }
    let serialized = selected.to_string();
    assert!(!serialized.contains(OPAQUE_MARKER));
    assert!(!serialized.contains(TAIL_MARKER));
    assert!(!serialized.contains("encrypted_content"));

    let output = command(root.path(), &["project", "web", "--inspect"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let cli: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(native_ids(&cli), ["auth-native", "web-native"]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(OPAQUE_MARKER));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(TAIL_MARKER));

    let validated = command(root.path(), &["context", "validate"]);
    assert!(validated.status.success());
    let metadata: Value = serde_json::from_slice(&validated.stdout).unwrap();
    assert_eq!(metadata["native_artifacts"], 3);
    assert_eq!(metadata["status"], "valid");
}

#[test]
fn linked_dependency_bundles_are_selected_once_without_unrelated_artifacts() {
    let root = fixture();
    let linked = inspect(root.path(), "subsystem:web");
    assert_eq!(
        linked["selection"]["subsystems"],
        json!(["authentication", "web"])
    );
    assert_eq!(native_ids(&linked), ["auth-native", "web-native"]);
    assert!(
        !linked["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| { source["path"].as_str().unwrap().contains("other-native/") })
    );
    assert_eq!(native_ids(&inspect(root.path(), "other")), ["other-native"]);
    subsystem(root.path(), "authentication", "web-native", &[]);
    let duplicate = inspect(root.path(), "projection:web-native");
    assert_eq!(native_ids(&duplicate), ["web-native"]);
}

#[test]
fn valid_native_selection_is_never_substituted_with_readable_launch_context() {
    let root = fixture();
    let project = Project::load(root.path()).unwrap();
    let executable = root.path().join("unexpected-codex");
    let marker = root.path().join("codex-was-invoked");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf invoked > \"$ASTRAL_NATIVE_TEST_MARKER\"\nexit 91\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    for selector in ["web", "projection:web-native", "authentication"] {
        assert_eq!(
            project.fresh_context(selector, None).unwrap_err().code,
            "NATIVE_CONTEXT_REQUIRES_STAGING"
        );
        let output = Command::new(env!("CARGO_BIN_EXE_astral"))
            .arg("--json")
            .arg("--root")
            .arg(root.path())
            .args(["project", selector])
            .env("ASTRAL_CODEX_BIN", &executable)
            .env("ASTRAL_NATIVE_TEST_MARKER", &marker)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let diagnostic: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(diagnostic["error"]["code"], "NATIVE_PROXY_REQUIRED");
        assert!(!marker.exists());
    }
}

#[test]
fn launch_context_keeps_native_window_separate_and_deduplicates_identical_bundles() {
    let root = fixture();
    let project = Project::load(root.path()).unwrap();
    let context = project.launch_context("web", None).unwrap();
    let documents = serde_json::to_string(&context.current_context).unwrap();
    assert!(!documents.contains(OPAQUE_MARKER));
    assert!(!documents.contains(TAIL_MARKER));
    assert!(context.native.is_some());
    // These fixture projections pin byte-identical bundles. Different paths
    // do not create a second native history.
    assert_eq!(context.native.unwrap().manifest().payload.item_count, 2);
    write_bundle(
        root.path(),
        "auth-native",
        "authentication",
        PROJECT_ID,
        "distinct opaque history",
    );
    let project = Project::load(root.path()).unwrap();
    assert_eq!(
        project.launch_context("web", None).unwrap_err().code,
        "MULTIPLE_NATIVE_CONTEXTS"
    );
    let explicit = project
        .launch_context("projection:web-native", None)
        .unwrap();
    assert!(explicit.native.is_some());
    assert!(
        explicit
            .current_context
            .selection
            .subsystems
            .iter()
            .any(|id| id == "authentication")
    );
    assert_eq!(
        explicit.native.unwrap().summary().manifest_sha256,
        project
            .native_artifacts("projection:web-native", None)
            .unwrap()
            .into_iter()
            .find(|a| a.projection == "web-native")
            .unwrap()
            .bundle
            .manifest_sha256
    );
    assert!(
        project
            .launch_context("authentication", None)
            .unwrap()
            .native
            .is_some()
    );
    make_readable(root.path(), "web-native");
    make_readable(root.path(), "auth-native");
    let project = Project::load(root.path()).unwrap();
    assert!(
        project
            .launch_context("web", None)
            .unwrap()
            .native
            .is_none()
    );
}

#[test]
fn loading_validates_even_native_bundles_outside_requested_scope() {
    let root = fixture();
    put(
        root.path(),
        &payload_path("other-native"),
        "corrupted payload",
    );
    let result = command(root.path(), &["project", "web", "--inspect"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("corrupted payload"));
}

#[test]
fn native_reference_is_required_and_cannot_disagree_with_projection_kind() {
    for (kind, payload, reference, expected) in [
        ("native-checkpoint", true, false, "MISSING_NATIVE_BUNDLE"),
        ("native-checkpoint", false, false, "MISSING_NATIVE_BUNDLE"),
        ("native-checkpoint", false, true, "INVALID_NATIVE_REFERENCE"),
        ("reviewable", true, true, "INVALID_NATIVE_REFERENCE"),
        ("reviewable", false, true, "INVALID_NATIVE_REFERENCE"),
    ] {
        let root = fixture();
        let path = projection_path("web-native");
        let mut manifest = fs::read_to_string(root.path().join(&path)).unwrap();
        if !reference {
            manifest = manifest.split("[native_bundle]").next().unwrap().to_owned();
        }
        manifest = manifest.replace("kind = \"native-checkpoint\"", &format!("kind = {kind:?}"));
        manifest = manifest.replace(
            "native_payload_in_repository = true",
            &format!("native_payload_in_repository = {payload}"),
        );
        put(root.path(), &path, manifest);
        assert_eq!(
            load_error(root.path()),
            expected,
            "kind={kind}, payload={payload}, reference={reference}"
        );
    }
}

#[test]
fn missing_artifacts_and_noncanonical_manifest_references_are_rejected() {
    for path in [manifest_path("web-native"), payload_path("web-native")] {
        let root = fixture();
        fs::remove_file(root.path().join(path)).unwrap();
        assert_eq!(load_error(root.path()), "NATIVE_BUNDLE_UNAVAILABLE");
    }
    for path in [
        "../manifest.json",
        "/tmp/manifest.json",
        "bundles/../demo/manifest.json",
        "bundles//demo/manifest.json",
        "./bundles/demo/manifest.json",
        r"bundles\demo\manifest.json",
    ] {
        let root = fixture();
        replace(
            root.path(),
            &projection_path("web-native"),
            "manifest = \"bundles/demo/manifest.json\"",
            &format!("manifest = '{path}'"),
        );
        assert_eq!(load_error(root.path()), "UNSAFE_PATH", "{path}");
    }
}

#[test]
fn manifest_payload_and_intermediate_directory_symlinks_are_rejected() {
    for path in [
        manifest_path("web-native"),
        payload_path("web-native"),
        ".astral/projections/web-native/bundles/demo".to_owned(),
    ] {
        let root = fixture();
        let source = root.path().join(path);
        let moved = root.path().join("outside-projection");
        fs::rename(&source, &moved).unwrap();
        symlink(&moved, &source).unwrap();
        assert_eq!(load_error(root.path()), "SYMLINK_REJECTED");
    }
}

#[test]
fn inspections_use_the_validated_memory_snapshot_after_disk_changes() {
    let root = fixture();
    let original = Project::load(root.path()).unwrap();
    let before = original.inspect("web", None).unwrap();
    write_bundle(
        root.path(),
        "web-native",
        "web",
        PROJECT_ID,
        "SYNTHETIC_CHANGED_NATIVE_BYTES",
    );
    assert_eq!(before, original.inspect("web", None).unwrap());
    let changed = inspect(root.path(), "web");
    assert_ne!(before["selection_digest"], changed["selection_digest"]);
    put(
        root.path(),
        &payload_path("web-native"),
        "invalid after successful load",
    );
    assert_eq!(before, original.inspect("web", None).unwrap());
    assert!(Project::load(root.path()).is_err());
}

#[test]
fn native_digests_are_root_independent_scoped_and_sensitive_to_artifact_paths() {
    let left = fixture();
    let right = fixture();
    let before = inspect(left.path(), "web");
    assert_eq!(before, inspect(right.path(), "web"));
    write_bundle(
        right.path(),
        "other-native",
        "other",
        PROJECT_ID,
        "SYNTHETIC_UNRELATED_NATIVE_BYTES",
    );
    assert_eq!(before, inspect(right.path(), "web"));
    let directory = right.path().join(".astral/projections/web-native/bundles");
    fs::rename(directory.join("demo"), directory.join("relocated")).unwrap();
    replace(
        right.path(),
        &projection_path("web-native"),
        "bundles/demo/manifest.json",
        "bundles/relocated/manifest.json",
    );
    let relocated = inspect(right.path(), "web");
    assert_ne!(before["selection_digest"], relocated["selection_digest"]);
    let artifact = relocated["native_artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["projection"] == "web-native")
        .unwrap();
    assert_eq!(
        artifact["payload"]["path"],
        ".astral/projections/web-native/bundles/relocated/window.json"
    );
}

#[test]
fn pinned_manifest_and_payload_corruption_have_distinct_integrity_failures() {
    let root = fixture();
    let path = manifest_path("web-native");
    let mut bytes = fs::read(root.path().join(&path)).unwrap();
    // JSON whitespace is semantically harmless but changes the pinned bytes.
    bytes.push(b'\n');
    put(root.path(), &path, bytes);
    assert_eq!(load_error(root.path()), "NATIVE_BUNDLE_DIGEST_MISMATCH");

    let root = fixture();
    replace(
        root.path(),
        &payload_path("web-native"),
        OPAQUE_MARKER,
        "SYNTHETIC_TAMPERED_NATIVE_PAYLOAD",
    );
    assert_eq!(load_error(root.path()), "NATIVE_INTEGRITY");
}

#[test]
fn a_valid_bundle_cannot_claim_another_project_identity() {
    let root = fixture();
    write_bundle(
        root.path(),
        "web-native",
        "web",
        "another-project",
        OPAQUE_MARKER,
    );
    assert_eq!(load_error(root.path()), "NATIVE_PROJECT_MISMATCH");
}

#[test]
fn readable_only_inspection_has_no_native_artifacts_or_native_fresh_gate() {
    let root = fixture();
    for id in ["web-native", "auth-native", "other-native"] {
        make_readable(root.path(), id);
    }
    let project = Project::load(root.path()).unwrap();
    let selected = project.inspect("web", None).unwrap();
    assert_eq!(selected["native_artifacts"], json!([]));
    assert_eq!(project.validate().unwrap()["native_artifacts"], 0);
    let fresh = project.fresh_context("web", None).unwrap();
    assert!(
        !serde_json::to_string(&fresh)
            .unwrap()
            .contains(OPAQUE_MARKER)
    );
}

#[test]
fn native_payloads_have_separate_file_caps_and_share_total_and_output_budgets() {
    let root = fixture();
    let large = "S".repeat(Limits::default().file_bytes + 1);
    write_bundle(root.path(), "web-native", "web", PROJECT_ID, &large);
    let project = Project::load(root.path()).unwrap();
    assert_eq!(
        native_ids(&project.inspect("web", None).unwrap()),
        ["auth-native", "web-native"]
    );
    for limits in [
        Limits {
            native_payload_bytes: 1_024,
            ..Limits::default()
        },
        Limits {
            total_bytes: large.len(),
            ..Limits::default()
        },
    ] {
        assert_eq!(
            Project::load_with_limits(root.path(), limits)
                .err()
                .unwrap()
                .code,
            "LIMIT_EXCEEDED"
        );
    }
    let bounded = Project::load_with_limits(
        root.path(),
        Limits {
            output_bytes: 512,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(
        bounded.inspect("web", None).unwrap_err().code,
        "LIMIT_EXCEEDED"
    );
    let oversized = "S".repeat(Limits::default().native_payload_bytes);
    write_bundle(root.path(), "web-native", "web", PROJECT_ID, &oversized);
    assert_eq!(load_error(root.path()), "LIMIT_EXCEEDED");
}

#[test]
fn native_artifacts_cannot_enter_fresh_context_through_copied_readable_documents() {
    for artifact in [payload_path("other-native"), manifest_path("other-native")] {
        let root = fixture();
        make_readable(root.path(), "web-native");
        make_readable(root.path(), "auth-native");
        let bytes = fs::read(root.path().join(artifact)).unwrap();
        put(root.path(), ".astral/core/web/README.md", bytes);
        assert_eq!(load_error(root.path()), "NATIVE_DOCUMENT_CONFLICT");
    }
}

#[test]
fn native_payload_document_alias_is_rejected_even_with_a_fresh_linked_projection() {
    let root = fixture();
    make_readable(root.path(), "web-native");
    make_readable(root.path(), "auth-native");
    replace(
        root.path(),
        ".astral/project.toml",
        "web = \"core/web\"",
        "web = \"projections/other-native\"",
    );
    put(
        root.path(),
        ".astral/projections/other-native/subsystem.toml",
        "schema_version = 1\nid = \"web\"\npurpose = \"Document role alias fixture\"\nreadme = \"bundles/demo/window.json\"\nrules = []\ndecisions = []\nwork_items = []\nprojection = \"web-native\"\ndepends_on = []\n",
    );
    assert_eq!(load_error(root.path()), "NATIVE_DOCUMENT_CONFLICT");
}
