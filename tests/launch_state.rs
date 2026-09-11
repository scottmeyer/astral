#![cfg(unix)]

use astral::launch_state::{LaunchState, MAX_RECEIPT_BYTES};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const THREAD: &str = "01a08e8f-c4af-77a3-9643-7c474187c260";
const SELECTION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BUNDLE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Fixture {
    _base: TempDir,
    root: PathBuf,
    store: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = tempfile::tempdir().unwrap();
        let canonical = base.path().canonicalize().unwrap();
        let root = canonical.join("workspace");
        fs::create_dir(&root).unwrap();
        let store = canonical.join("state");
        Self {
            _base: base,
            root,
            store,
        }
    }
    fn create(&self) -> LaunchState {
        LaunchState::create_in(
            &self.store,
            &self.root,
            "fixture",
            "projection:saved",
            Some("AST-001"),
            SELECTION,
            BUNDLE,
        )
        .unwrap()
    }
    fn resume(&self, id: &str) -> astral::project::Result<LaunchState> {
        LaunchState::resume_in(
            &self.store,
            id,
            &self.root,
            "fixture",
            "projection:saved",
            Some("AST-001"),
            SELECTION,
            BUNDLE,
        )
    }
    fn path(&self, id: &str) -> PathBuf {
        self.store.join(id).join("receipt.json")
    }
    fn receipt(&self, id: &str) -> Value {
        serde_json::from_slice(&fs::read(self.path(id)).unwrap()).unwrap()
    }
    fn staged(&self) -> String {
        let mut state = self.create();
        state.staged(THREAD, "gpt-6-astra", "openai").unwrap();
        state.id().to_owned()
    }
}

fn code(result: astral::project::Result<LaunchState>) -> &'static str {
    result.err().expect("operation must fail").code
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().mode() & 0o7777
}

#[test]
fn creation_is_private_random_bounded_and_contains_only_receipt_metadata() {
    let fixture = Fixture::new();
    let mut first = fixture.create();
    let second = fixture.create();
    assert_ne!(first.id(), second.id());
    assert_eq!(first.id().len(), 32);
    assert!(
        first
            .id()
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert_eq!(first.status(), "preparing");
    assert_eq!(first.thread_id(), None);
    assert_eq!(mode(&fixture.store), 0o700);
    assert_eq!(mode(&fixture.store.join(first.id())), 0o700);
    assert_eq!(mode(first.proxy_state_dir()), 0o700);
    assert_eq!(mode(&fixture.path(first.id())), 0o600);
    assert_eq!(
        mode(&fixture.store.join(first.id()).join("owner.lock")),
        0o600
    );
    assert_eq!(
        first.proxy_state_dir(),
        fixture.store.join(first.id()).join("proxy")
    );
    first.failed("PRIVATE STDERR BODY\nnot a code").unwrap();
    let bytes = fs::read(fixture.path(first.id())).unwrap();
    assert!(bytes.len() <= MAX_RECEIPT_BYTES);
    assert!(!String::from_utf8_lossy(&bytes).contains("PRIVATE"));
    assert_eq!(fixture.receipt(first.id())["last_error"], "LAUNCH_FAILED");
    assert_eq!(
        fixture.receipt(first.id())["workspace"],
        fixture.root.to_str().unwrap()
    );
    let keys = fixture
        .receipt(first.id())
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            "bundle_manifest_sha256",
            "exit_code",
            "id",
            "last_error",
            "model",
            "project_id",
            "provider",
            "schema_version",
            "selection_digest",
            "selector",
            "status",
            "thread_id",
            "work_id",
            "workspace"
        ]
    );
}

#[test]
fn cold_resume_reuses_thread_and_drop_releases_ownership() {
    let fixture = Fixture::new();
    let mut state = fixture.create();
    let id = state.id().to_owned();
    assert_eq!(code(fixture.resume(&id)), "LAUNCH_ACTIVE");
    state.staged(THREAD, "gpt-6-astra", "openai").unwrap();
    assert_eq!(
        state
            .staged(THREAD, "gpt-6-astra", "openai")
            .unwrap_err()
            .code,
        "LAUNCH_ALREADY_STAGED"
    );
    state.finished(Some(7)).unwrap();
    drop(state);
    let mut resumed = fixture.resume(&id).unwrap();
    assert_eq!(resumed.thread_id(), Some(THREAD));
    assert_eq!(resumed.model(), Some("gpt-6-astra"));
    assert_eq!(resumed.provider(), Some("openai"));
    assert_eq!(resumed.status(), "finished");
    resumed.failed("PROXY_START_FAILED").unwrap();
    drop(resumed);
    assert_eq!(fixture.receipt(&id)["last_error"], "PROXY_START_FAILED");
    let mut resumed = fixture.resume(&id).unwrap();
    resumed.finished(None).unwrap();
    assert_eq!(fixture.receipt(&id)["exit_code"], Value::Null);
    assert_eq!(fixture.receipt(&id)["last_error"], Value::Null);
}

#[test]
fn failure_before_staging_is_retained_and_cannot_resume_as_fresh() {
    let fixture = Fixture::new();
    let mut state = fixture.create();
    let id = state.id().to_owned();
    state.failed("NATIVE_IMPORT_FAILED").unwrap();
    assert_eq!(
        state.finished(Some(0)).unwrap_err().code,
        "LAUNCH_NOT_STAGED"
    );
    drop(state);
    let before = fs::read(fixture.path(&id)).unwrap();
    assert_eq!(code(fixture.resume(&id)), "LAUNCH_NOT_STAGED");
    assert_eq!(fs::read(fixture.path(&id)).unwrap(), before);
    assert_eq!(fixture.receipt(&id)["status"], "failed");
    assert!(fixture.store.join(id).join("owner.lock").exists());
}

#[test]
fn every_resume_identity_component_is_bound_to_the_receipt() {
    let fixture = Fixture::new();
    let id = fixture.staged();
    let other_root = fixture.root.parent().unwrap().join("other");
    fs::create_dir(&other_root).unwrap();
    for (root, project, selector, work, selection, bundle) in [
        (
            other_root.as_path(),
            "fixture",
            "projection:saved",
            Some("AST-001"),
            SELECTION,
            BUNDLE,
        ),
        (
            fixture.root.as_path(),
            "other",
            "projection:saved",
            Some("AST-001"),
            SELECTION,
            BUNDLE,
        ),
        (
            fixture.root.as_path(),
            "fixture",
            "subsystem:saved",
            Some("AST-001"),
            SELECTION,
            BUNDLE,
        ),
        (
            fixture.root.as_path(),
            "fixture",
            "projection:saved",
            None,
            SELECTION,
            BUNDLE,
        ),
        (
            fixture.root.as_path(),
            "fixture",
            "projection:saved",
            Some("AST-001"),
            BUNDLE,
            BUNDLE,
        ),
        (
            fixture.root.as_path(),
            "fixture",
            "projection:saved",
            Some("AST-001"),
            SELECTION,
            SELECTION,
        ),
    ] {
        assert_eq!(
            code(LaunchState::resume_in(
                &fixture.store,
                &id,
                root,
                project,
                selector,
                work,
                selection,
                bundle
            )),
            "LAUNCH_STATE_MISMATCH"
        );
    }
    let alias = fixture.root.parent().unwrap().join("workspace-alias");
    symlink(&fixture.root, &alias).unwrap();
    LaunchState::resume_in(
        &fixture.store,
        &id,
        &alias,
        "fixture",
        "projection:saved",
        Some("AST-001"),
        SELECTION,
        BUNDLE,
    )
    .unwrap();
}

#[test]
fn malformed_tampered_and_oversized_receipts_are_rejected_without_echoing_input() {
    let fixture = Fixture::new();
    let id = fixture.staged();
    let baseline = fixture.receipt(&id);
    for (key, value) in [
        ("thread_id", json!("not-a-uuid")),
        ("thread_id", json!("00000000-0000-0000-0000-000000000000")),
        ("model", Value::Null),
        ("provider", json!("PRIVATE\nVALUE")),
        ("status", json!("unknown")),
        ("last_error", json!("PRIVATE ERROR BODY")),
        ("exit_code", json!(0)),
        ("unknown", json!("PRIVATE VALUE")),
    ] {
        let mut changed = baseline.clone();
        changed[key] = value;
        fs::write(fixture.path(&id), serde_json::to_vec(&changed).unwrap()).unwrap();
        let error = fixture.resume(&id).err().unwrap();
        assert!(matches!(
            error.code,
            "LAUNCH_STATE_INVALID" | "LAUNCH_STATE_MISMATCH"
        ));
        assert!(!error.to_string().contains("PRIVATE"));
    }
    for key in baseline.as_object().unwrap().keys() {
        let mut changed = baseline.clone();
        changed.as_object_mut().unwrap().remove(key);
        fs::write(fixture.path(&id), serde_json::to_vec(&changed).unwrap()).unwrap();
        assert_eq!(code(fixture.resume(&id)), "LAUNCH_STATE_INVALID", "{key}");
    }
    let duplicate = serde_json::to_string(&baseline).unwrap().replacen(
        "\"schema_version\":1",
        "\"schema_version\":1,\"schema_version\":1",
        1,
    );
    fs::write(fixture.path(&id), duplicate).unwrap();
    assert_eq!(code(fixture.resume(&id)), "LAUNCH_STATE_INVALID");
    fs::write(fixture.path(&id), vec![b' '; MAX_RECEIPT_BYTES + 1]).unwrap();
    assert_eq!(code(fixture.resume(&id)), "LAUNCH_STATE_LIMIT");
}

#[test]
fn owned_receipt_tampering_is_not_silently_overwritten() {
    let fixture = Fixture::new();
    let mut state = fixture.create();
    let path = fixture.path(state.id());
    fs::write(&path, b"PRIVATE MODIFICATION").unwrap();
    assert_eq!(
        state
            .staged(THREAD, "gpt-6-astra", "openai")
            .unwrap_err()
            .code,
        "LAUNCH_STATE_CHANGED"
    );
    assert_eq!(state.thread_id(), None);
    assert_eq!(fs::read(path).unwrap(), b"PRIVATE MODIFICATION");
}

#[test]
fn unsafe_ids_paths_and_insecure_directories_are_rejected() {
    let fixture = Fixture::new();
    for id in [
        "",
        "../escape",
        "/etc/passwd",
        "AABB",
        "01a08e8f-c4af-77a3-9643-7c474187c260",
    ] {
        assert_eq!(code(fixture.resume(id)), "LAUNCH_STATE_ID");
    }
    for store in [
        PathBuf::from("relative"),
        fixture.store.join("../escape"),
        fixture.store.join("."),
        fixture.root.join(".astral/state"),
    ] {
        let error = code(LaunchState::create_in(
            &store,
            &fixture.root,
            "fixture",
            "projection:saved",
            None,
            SELECTION,
            BUNDLE,
        ));
        assert!(matches!(
            error,
            "LAUNCH_STATE_PATH" | "LAUNCH_STATE_IN_WORKSPACE"
        ));
    }
    fs::create_dir(&fixture.store).unwrap();
    fs::set_permissions(&fixture.store, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        code(LaunchState::create_in(
            &fixture.store,
            &fixture.root,
            "fixture",
            "projection:saved",
            None,
            SELECTION,
            BUNDLE
        )),
        "LAUNCH_STATE_INSECURE"
    );
}

#[test]
fn symlink_routes_and_nonprivate_or_hardlinked_files_fail_closed() {
    for entry in ["receipt.json", "owner.lock", "proxy"] {
        let fixture = Fixture::new();
        let id = fixture.staged();
        let path = fixture.store.join(&id).join(entry);
        let moved = fixture.store.join(&id).join("moved");
        fs::rename(&path, &moved).unwrap();
        symlink(&moved, &path).unwrap();
        assert_eq!(
            code(fixture.resume(&id)),
            "LAUNCH_STATE_UNAVAILABLE",
            "{entry}"
        );
    }
    for entry in ["receipt.json", "owner.lock", "proxy", ""] {
        let fixture = Fixture::new();
        let id = fixture.staged();
        let path = fixture.store.join(&id).join(entry);
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            code(fixture.resume(&id)),
            "LAUNCH_STATE_INSECURE",
            "{entry}"
        );
    }
    let fixture = Fixture::new();
    let id = fixture.staged();
    fs::hard_link(fixture.path(&id), fixture.store.join("linked.json")).unwrap();
    assert_eq!(code(fixture.resume(&id)), "LAUNCH_STATE_INSECURE");
    let alias = fixture.store.with_extension("alias");
    symlink(&fixture.store, &alias).unwrap();
    assert_eq!(
        code(LaunchState::resume_in(
            &alias,
            &id,
            &fixture.root,
            "fixture",
            "projection:saved",
            Some("AST-001"),
            SELECTION,
            BUNDLE
        )),
        "LAUNCH_STATE_UNAVAILABLE"
    );
}

#[test]
fn receipt_updates_are_atomic_for_concurrent_readers() {
    let fixture = Fixture::new();
    let mut state = fixture.create();
    state.staged(THREAD, "gpt-6-astra", "openai").unwrap();
    let path = fixture.path(state.id());
    let reader = std::thread::spawn(move || {
        for _ in 0..200 {
            let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(value["thread_id"], THREAD);
        }
    });
    for n in 0..20 {
        state.finished(Some(n)).unwrap();
    }
    reader.join().unwrap();
    assert_eq!(fixture.receipt(state.id())["exit_code"], 19);
    assert_eq!(
        fs::read_dir(fixture.store.join(state.id()))
            .unwrap()
            .count(),
        3
    );
}

#[test]
fn real_second_process_cannot_own_active_launch_and_can_resume_after_release() {
    let fixture = Fixture::new();
    let mut state = fixture.create();
    state.staged(THREAD, "gpt-6-astra", "openai").unwrap();
    let id = state.id().to_owned();
    let run = || {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "launch_child_fixture", "--nocapture"])
            .env("ASTRAL_STATE_CHILD", "1")
            .env("ASTRAL_LAUNCH_STATE_DIR", &fixture.store)
            .env("ASTRAL_STATE_CHILD_ROOT", &fixture.root)
            .env("ASTRAL_STATE_CHILD_ID", &id)
            .output()
            .unwrap()
    };
    let blocked = run();
    assert!(blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stdout).contains("STATE_CHILD_LAUNCH_ACTIVE"));
    drop(state);
    let resumed = run();
    assert!(resumed.status.success());
    assert!(String::from_utf8_lossy(&resumed.stdout).contains("STATE_CHILD_OK"));
}

#[test]
fn launch_child_fixture() {
    let Some(mode) = std::env::var_os("ASTRAL_STATE_CHILD") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("ASTRAL_STATE_CHILD_ROOT").unwrap());
    if mode == "create" {
        let state = LaunchState::create(
            &root,
            "fixture",
            "projection:saved",
            None,
            SELECTION,
            BUNDLE,
        )
        .unwrap();
        println!("STATE_CHILD_CREATED_{}", state.id());
        return;
    }
    let id = std::env::var("ASTRAL_STATE_CHILD_ID").unwrap();
    match LaunchState::resume(
        &id,
        &root,
        "fixture",
        "projection:saved",
        Some("AST-001"),
        SELECTION,
        BUNDLE,
    ) {
        Ok(state) => {
            assert_eq!(state.thread_id(), Some(THREAD));
            println!("STATE_CHILD_OK");
        }
        Err(error) => println!("STATE_CHILD_{}", error.code),
    }
}

#[test]
fn configured_default_uses_current_home_outside_workspace() {
    let fixture = Fixture::new();
    let home = fixture.root.parent().unwrap().join("home");
    fs::create_dir(&home).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "launch_child_fixture", "--nocapture"])
        .env_remove("ASTRAL_LAUNCH_STATE_DIR")
        .env("HOME", &home)
        .env("ASTRAL_STATE_CHILD", "create")
        .env("ASTRAL_STATE_CHILD_ROOT", &fixture.root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let id = stdout
        .lines()
        .find_map(|line| line.strip_prefix("STATE_CHILD_CREATED_"))
        .unwrap();
    let launches = home.join(".local/state/astral/launches");
    assert_eq!(mode(&launches), 0o700);
    assert!(launches.join(id).join("receipt.json").is_file());
    assert!(!fixture.root.join(".local").exists());
}
