use super::*;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::process::Command;

fn repository() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .current_dir(temp.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    temp
}

#[test]
fn notices_are_bounded_hashed_and_scoped_without_worker_locks() {
    let root = repository();
    let digest = crate::hash(b"observation");
    assert!(notify_once(root.path(), "git", "private session text", &digest).unwrap());
    assert!(!notify_once(root.path(), "git", "private session text", &digest).unwrap());
    assert!(notify_once(root.path(), "codex", "private session text", &digest).unwrap());
    assert!(
        notify_once(
            root.path(),
            "git",
            "private session text",
            &crate::hash(b"changed")
        )
        .unwrap()
    );
    for n in 0..70 {
        assert!(notify_once(root.path(), "git", &format!("session-{n}"), &digest).unwrap());
    }
    let store = Storage::open(root.path(), false).unwrap().unwrap();
    let bytes = store.read("notices.json", 16_384).unwrap().unwrap();
    let parsed: Notices = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.entries.len(), 64);
    assert!(bytes.len() < 16_384);
    assert!(!String::from_utf8(bytes).unwrap().contains("session"));
    let lock = store.try_lock().unwrap();
    assert_eq!(
        notify_once(root.path(), "git", "another", &digest)
            .unwrap_err()
            .code,
        "HOOK_STORAGE_BUSY"
    );
    drop(lock);
    assert!(notify_once(root.path(), "git", "another", &digest).unwrap());
    assert_eq!(
        fs::metadata(&store.directory.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(store.directory.path.join("notices.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn private_storage_rejects_links_modes_and_oversized_notices() {
    let root = repository();
    let git = root.path().join(".git");
    let other = root.path().join("foreign");
    fs::create_dir(&other).unwrap();
    symlink(&other, git.join("astral-hooks")).unwrap();
    assert!(Storage::open(root.path(), false).is_err());
    let root = repository();
    let store = Storage::open(root.path(), true).unwrap().unwrap();
    let outside = root.path().join("outside");
    fs::write(&outside, b"untouched").unwrap();
    symlink(&outside, store.directory.path.join("notices.json")).unwrap();
    assert!(notify_once(root.path(), "git", "session", &crate::hash(b"value")).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"untouched");
    let root = repository();
    let store = Storage::open(root.path(), true).unwrap().unwrap();
    let file = store.directory.path.join("notices.json");
    fs::write(&file, vec![b' '; 16_385]).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        notify_once(root.path(), "git", "session", &crate::hash(b"value"))
            .unwrap_err()
            .code,
        "HOOK_LIMIT"
    );
    fs::write(&file, b"{\"entries\":[]}").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        notify_once(root.path(), "git", "session", &crate::hash(b"value"))
            .unwrap_err()
            .code,
        "HOOK_STORAGE_UNSAFE"
    );
}
