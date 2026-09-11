#![cfg(unix)]
use ostk_gpt_cache::launch_state::{
    LaunchContinuation, LaunchInventory, LaunchObservation, LaunchOwnership, LaunchState,
    MAX_INVENTORY_ENTRIES, MAX_RECEIPT_BYTES,
};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
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
        let root = base.path().canonicalize().unwrap().join("workspace");
        fs::create_dir(&root).unwrap();
        let store = root.parent().unwrap().join("state");
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
            None,
            SELECTION,
            BUNDLE,
        )
        .unwrap()
    }
    fn staged(&self) -> String {
        let mut state = self.create();
        state.staged(THREAD, "gpt-6-astra", "openai").unwrap();
        state.id().to_owned()
    }
    fn path(&self, id: &str) -> PathBuf {
        self.store.join(id).join("receipt.json")
    }
    fn observe(&self, id: &str) -> LaunchObservation {
        LaunchState::observe_in(&self.store, id, &self.root, "fixture").unwrap()
    }
    fn inventory(&self, offset: usize, limit: usize) -> LaunchInventory {
        LaunchState::inventory_in(&self.store, &self.root, "fixture", offset, limit).unwrap()
    }
    fn change(&self, id: &str, f: impl FnOnce(&mut Value)) {
        let mut receipt: Value = serde_json::from_slice(&fs::read(self.path(id)).unwrap()).unwrap();
        f(&mut receipt);
        fs::write(self.path(id), serde_json::to_vec(&receipt).unwrap()).unwrap();
    }
}

fn assert_unknown(view: &LaunchObservation) {
    assert_eq!(view.ownership, LaunchOwnership::Unknown);
    assert_eq!(view.continuation, LaunchContinuation::Unknown);
    assert!(view.receipt.is_none());
    assert!(view.receipt_sha256.is_none());
    assert!(!view.issues.is_empty());
}

#[test]
fn live_owner_and_unknown_initial_outcome_are_distinct_and_observation_is_read_only() {
    let f = Fixture::new();
    let state = f.create();
    let id = state.id().to_owned();
    let before = fs::read(f.path(&id)).unwrap();
    let live = f.observe(&id);
    assert_eq!(live.ownership, LaunchOwnership::Busy);
    assert_eq!(live.continuation, LaunchContinuation::Owned);
    assert_eq!(live.receipt.as_ref().unwrap().status, "preparing");
    assert_eq!(live.receipt_sha256.as_deref(), state.receipt_sha256());
    assert_eq!(state.selector(), "projection:saved");
    assert_eq!(state.work_id(), None);
    assert_eq!(state.selection_digest(), SELECTION);
    assert_eq!(state.bundle_manifest_sha256(), BUNDLE);
    drop(state);
    let closed = f.observe(&id);
    assert_eq!(closed.ownership, LaunchOwnership::Available);
    assert_eq!(
        closed.continuation,
        LaunchContinuation::InitialOutcomeUnknown
    );
    assert_eq!(fs::read(f.path(&id)).unwrap(), before);
    assert_eq!(fs::read_dir(f.store.join(&id)).unwrap().count(), 3);
}

#[test]
fn recorded_thread_candidate_is_not_a_claim_of_execution_and_probe_releases_lock() {
    let f = Fixture::new();
    let id = f.staged();
    let before = fs::read(f.path(&id)).unwrap();
    let view = f.observe(&id);
    assert_eq!(
        view.continuation,
        LaunchContinuation::RecordedContinuationCandidate
    );
    let receipt = view.receipt.unwrap();
    assert_eq!(receipt.thread_id.as_deref(), Some(THREAD));
    assert_eq!(receipt.model.as_deref(), Some("gpt-6-astra"));
    assert_eq!(receipt.provider.as_deref(), Some("openai"));
    let mut owner = LaunchState::resume_in(
        &f.store,
        &id,
        &f.root,
        "fixture",
        "projection:saved",
        None,
        SELECTION,
        BUNDLE,
    )
    .unwrap();
    assert_eq!(f.observe(&id).ownership, LaunchOwnership::Busy);
    owner.failed("CONTINUATION_FAILED").unwrap();
    drop(owner);
    let view = f.observe(&id);
    assert_eq!(
        view.continuation,
        LaunchContinuation::RecordedContinuationCandidate
    );
    assert_eq!(
        view.receipt.unwrap().last_error.as_deref(),
        Some("CONTINUATION_FAILED")
    );
    assert_ne!(fs::read(f.path(&id)).unwrap(), before);
    let second = f.create();
    let second_id = second.id().to_owned();
    drop(second);
    assert_eq!(
        f.observe(&second_id).continuation,
        LaunchContinuation::InitialOutcomeUnknown
    );
}

#[test]
fn only_valid_matching_scope_is_exposed_and_malformed_scope_is_unknown() {
    let f = Fixture::new();
    let local = f.staged();
    let other_project = f.staged();
    f.change(&other_project, |v| {
        v["project_id"] = json!("foreign-project-secret")
    });
    let other_workspace = f.staged();
    f.change(&other_workspace, |v| {
        v["workspace"] = json!("/never-access-foreign-workspace")
    });
    let malformed = f.staged();
    f.change(&malformed, |v| {
        v["project_id"] = json!("foreign-project-secret");
        v["thread_id"] = json!("private malformed thread");
    });
    let report = f.inventory(0, 64);
    assert_eq!(report.total_entries, 4);
    assert_eq!(report.excluded_foreign_launches, 2);
    assert_eq!(report.total_observations, 2);
    assert!(
        report
            .launches
            .iter()
            .any(|v| v.launch_id == local && v.receipt.is_some())
    );
    assert_unknown(
        report
            .launches
            .iter()
            .find(|v| v.launch_id == malformed)
            .unwrap(),
    );
    let output = serde_json::to_string(&report).unwrap();
    for secret in [
        "foreign-project-secret",
        "never-access-foreign-workspace",
        "private malformed thread",
        &other_project,
        &other_workspace,
    ] {
        assert!(!output.contains(secret));
    }
    assert_unknown(&f.observe(&other_project));
}

#[test]
fn strict_receipt_errors_do_not_echo_untrusted_content() {
    let f = Fixture::new();
    let id = f.staged();
    let original = fs::read(f.path(&id)).unwrap();
    for change in [
        json!({"thread_id":"sensitive-bad-uuid"}),
        json!({"model":"unsafe\nmodel"}),
        json!({"provider":null}),
        json!({"status":"unexpected-status"}),
        json!({"status":"preparing"}),
        json!({"id":"00000000000000000000000000000000"}),
        json!({"schema_version":2}),
        json!({"selection_digest":"bad"}),
        json!({"bundle_manifest_sha256":"bad"}),
        json!({"selector":"../bad"}),
        json!({"exit_code":0}),
        json!({"last_error":"SECRET\nerror"}),
        json!({"new_field":"SECRET"}),
    ] {
        fs::write(f.path(&id), &original).unwrap();
        f.change(&id, |v| {
            for (key, value) in change.as_object().unwrap() {
                v[key] = value.clone();
            }
        });
        assert_unknown(&f.observe(&id));
    }
    for bytes in [
        b"SECRET MALFORMED JSON".to_vec(),
        vec![b' '; MAX_RECEIPT_BYTES + 1],
        original
            .iter()
            .copied()
            .take(original.len() - 1)
            .chain(b",\"schema_version\":1}".iter().copied())
            .collect(),
    ] {
        fs::write(f.path(&id), bytes).unwrap();
        let view = f.observe(&id);
        assert_unknown(&view);
        assert!(!serde_json::to_string(&view).unwrap().contains("SECRET"));
    }
    fs::write(f.path(&id), &original).unwrap();
    f.change(&id, |v| {
        v.as_object_mut().unwrap().remove("work_id");
    });
    assert_unknown(&f.observe(&id));
}

#[test]
fn explicit_root_is_required_and_missing_storage_is_not_created() {
    let f = Fixture::new();
    assert!(LaunchState::inventory_in(&f.store, &f.root, "fixture", 0, 64).is_err());
    assert!(!f.store.exists());
    fs::create_dir(&f.store).unwrap();
    fs::set_permissions(&f.store, fs::Permissions::from_mode(0o700)).unwrap();
    let empty = f.inventory(0, 64);
    assert_eq!(empty.total_entries, 0);
    assert_eq!(fs::read_dir(&f.store).unwrap().count(), 0);
    for limit in [0, 65, usize::MAX] {
        assert_eq!(
            LaunchState::inventory_in(&f.store, &f.root, "fixture", 0, limit)
                .unwrap_err()
                .code,
            "LAUNCH_STATE_PAGE"
        );
    }
    assert_eq!(
        LaunchState::inventory_in("relative".as_ref(), &f.root, "fixture", 0, 64)
            .unwrap_err()
            .code,
        "LAUNCH_STATE_PATH"
    );
    let inside = f.root.join("state");
    fs::create_dir(&inside).unwrap();
    fs::set_permissions(&inside, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        LaunchState::inventory_in(&inside, &f.root, "fixture", 0, 64)
            .unwrap_err()
            .code,
        "LAUNCH_STATE_IN_WORKSPACE"
    );
    let alias = f.store.with_file_name("alias");
    symlink(&f.store, &alias).unwrap();
    assert!(LaunchState::inventory_in(&alias, &f.root, "fixture", 0, 64).is_err());
    let opaque = f
        .store
        .with_file_name(std::ffi::OsString::from_vec(vec![0xff, b'x']));
    // Validation precedes filesystem access, including on filesystems that
    // cannot represent this native OS path.
    assert_eq!(
        LaunchState::inventory_in(&opaque, &f.root, "fixture", 0, 64)
            .unwrap_err()
            .code,
        "LAUNCH_STATE_PATH"
    );
    fs::set_permissions(&f.store, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        LaunchState::inventory_in(&f.store, &f.root, "fixture", 0, 64)
            .unwrap_err()
            .code,
        "LAUNCH_STATE_INSECURE"
    );
    assert_eq!(
        LaunchState::observe_in(&f.store, "../not-an-id", &f.root, "fixture")
            .unwrap_err()
            .code,
        "LAUNCH_STATE_ID"
    );
}

#[test]
fn unknown_names_are_not_opened_or_echoed_and_pagination_is_deterministic() {
    let f = Fixture::new();
    let mut ids: Vec<_> = (0..5).map(|_| f.staged()).collect();
    ids.sort();
    fs::write(f.store.join("secret\nunknown-name"), "SECRET PAYLOAD").unwrap();
    let opaque_entry = match fs::write(
        f.store.join(std::ffi::OsString::from_vec(vec![0xff, b'X'])),
        "SECRET PAYLOAD",
    ) {
        Ok(()) => 1,
        // APFS rejects invalid UTF-8 names before an entry can exist. Other
        // filesystems exercise enumeration of an actual opaque entry here.
        Err(e) if e.raw_os_error() == Some(libc::EILSEQ) => 0,
        Err(e) => panic!("cannot create opaque-name fixture: {e}"),
    };
    let first = f.inventory(0, 2);
    assert_eq!(first.total_entries, 6 + opaque_entry);
    assert_eq!(first.total_observations, 5);
    assert_eq!(first.unrecognized_entries, 1 + opaque_entry);
    assert_eq!(
        first
            .launches
            .iter()
            .map(|v| &v.launch_id)
            .collect::<Vec<_>>(),
        ids[..2].iter().collect::<Vec<_>>()
    );
    assert_eq!(first.next_offset, Some(2));
    assert_eq!(first.issues.len(), 1);
    let output = serde_json::to_string(&first).unwrap();
    assert!(!output.contains("unknown-name"));
    assert!(!output.contains("SECRET"));
    let second = f.inventory(2, 2);
    assert_eq!(second.launches[0].launch_id, ids[2]);
    assert_eq!(second.next_offset, Some(4));
    let last = f.inventory(4, 2);
    assert_eq!(last.launches.len(), 1);
    assert_eq!(last.next_offset, None);
    let beyond = f.inventory(usize::MAX, 2);
    assert!(beyond.launches.is_empty());
    assert_eq!(beyond.next_offset, None);
}

#[test]
fn entry_bound_applies_even_to_unknown_entries() {
    let f = Fixture::new();
    fs::create_dir(&f.store).unwrap();
    fs::set_permissions(&f.store, fs::Permissions::from_mode(0o700)).unwrap();
    for i in 0..=MAX_INVENTORY_ENTRIES {
        fs::write(f.store.join(format!("unknown-{i}")), []).unwrap();
    }
    assert_eq!(
        LaunchState::inventory_in(&f.store, &f.root, "fixture", 0, 1)
            .unwrap_err()
            .code,
        "LAUNCH_STATE_LIMIT"
    );
}

#[test]
fn unsafe_receipt_lock_proxy_and_launch_directory_are_unknown() {
    for target in ["receipt.json", "owner.lock", "proxy", "."] {
        let f = Fixture::new();
        let id = f.staged();
        let path = f.store.join(&id).join(target);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert_unknown(&f.observe(&id));
    }
    for target in ["receipt.json", "owner.lock", "proxy"] {
        let f = Fixture::new();
        let id = f.staged();
        let path = f.store.join(&id).join(target);
        let saved = f.store.with_file_name(format!("outside-{target}"));
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        assert_unknown(&f.observe(&id));
    }
    for target in ["receipt.json", "owner.lock"] {
        let f = Fixture::new();
        let id = f.staged();
        fs::hard_link(
            f.store.join(&id).join(target),
            f.store.with_file_name("outside-hardlink"),
        )
        .unwrap();
        assert_unknown(&f.observe(&id));
    }
    let f = Fixture::new();
    let id = f.staged();
    let path = f.store.join(&id);
    let saved = f.store.with_file_name("outside-launch");
    fs::rename(&path, &saved).unwrap();
    symlink(&saved, &path).unwrap();
    assert_unknown(&f.observe(&id));
}
