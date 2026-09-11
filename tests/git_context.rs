#![cfg(unix)]
use ostk_gpt_cache::git_context::{require_committed_context, revision};
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) {
    let result = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test]
async fn committed_context_detects_ignored_and_hidden_changes_without_running_fsmonitor() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-b", "main"]);
    fs::create_dir(root.join(".astral")).unwrap();
    fs::write(root.join(".astral/document.md"), "committed").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "fixture"]);
    let hook = root.join("monitor");
    fs::write(&hook, "#!/bin/sh\nprintf invoked > fsmonitor-ran\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    git(root, &["config", "core.fsmonitor", hook.to_str().unwrap()]);
    require_committed_context(root, ".astral/work.jsonl")
        .await
        .unwrap();
    revision(root).await.unwrap();
    assert!(!root.join("fsmonitor-ran").exists());
    fs::write(root.join(".gitignore"), ".astral/ignored.md\n").unwrap();
    fs::write(root.join(".astral/ignored.md"), "ignored context").unwrap();
    assert_eq!(
        require_committed_context(root, ".astral/work.jsonl")
            .await
            .unwrap_err()
            .code,
        "WORKSPACE_UNCOMMITTED_CONTEXT"
    );
    // Track the formerly ignored file to isolate the next case without deleting it.
    git(root, &["add", "-f", ".astral/ignored.md", ".gitignore"]);
    git(root, &["commit", "-m", "track context"]);
    for flag in ["--assume-unchanged", "--skip-worktree"] {
        git(root, &["update-index", flag, ".astral/document.md"]);
        assert_eq!(
            require_committed_context(root, ".astral/work.jsonl")
                .await
                .unwrap_err()
                .code,
            "WORKSPACE_UNCOMMITTED_CONTEXT"
        );
        git(
            root,
            &[
                "update-index",
                "--no-assume-unchanged",
                ".astral/document.md",
            ],
        );
        git(
            root,
            &["update-index", "--no-skip-worktree", ".astral/document.md"],
        );
    }
    fs::write(root.join(".astral/work.jsonl"), "new work").unwrap();
    require_committed_context(root, ".astral/work.jsonl")
        .await
        .unwrap();
}
