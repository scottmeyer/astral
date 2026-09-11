#![cfg(unix)]
use ostk_gpt_cache::hooks::install::{self, Action, GIT_EVENTS, Target};
use serde_json::Value;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::{fs, path::PathBuf, process::Command};
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    exe: PathBuf,
    log: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("repo");
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--initial-branch=main"])
                .current_dir(&root)
                .output()
                .unwrap()
                .status
                .success()
        );
        let exe = base.join("astral's executable");
        let log = base.join("argv.log");
        fs::write(
            &exe,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nexit 17\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _temp: temp,
            root,
            exe,
            log,
        }
    }
    fn apply(&self, target: Target, action: Action) -> Value {
        let plan = install::plan(&self.root, &self.exe, target, action).unwrap();
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        install::apply(&self.root, &self.exe, target, action, &plan.plan_sha256).unwrap()
    }
    fn hooks(&self) -> PathBuf {
        self.root.join(".git/hooks")
    }
    fn state(&self) -> PathBuf {
        self.root.join(".git/astral-hooks")
    }
    fn codex(&self) -> PathBuf {
        self.root.join(".codex/hooks.json")
    }
    fn config(&self, bytes: &[u8]) {
        fs::create_dir_all(self.root.join(".codex")).unwrap();
        fs::write(self.codex(), bytes).unwrap();
    }
}
#[test]
fn git_preview_install_disable_and_reenable_preserve_owned_bytes() {
    let f = Fixture::new();
    let before = fs::read_dir(f.hooks()).unwrap().count();
    let plan = install::plan(&f.root, &f.exe, Target::Git, Action::Install).unwrap();
    assert!(!f.state().exists());
    assert_eq!(fs::read_dir(f.hooks()).unwrap().count(), before);
    assert!(plan.blockers.is_empty());
    f.apply(Target::Git, Action::Install);
    assert!(install::git_enabled(&f.root, &f.exe).unwrap());
    let snapshots: Vec<_> = GIT_EVENTS
        .iter()
        .map(|e| fs::read(f.hooks().join(e)).unwrap())
        .collect();
    f.apply(Target::Git, Action::Install);
    f.apply(Target::Git, Action::Uninstall);
    assert!(!install::git_enabled(&f.root, &f.exe).unwrap());
    for (event, bytes) in GIT_EVENTS.iter().zip(&snapshots) {
        assert_eq!(&fs::read(f.hooks().join(event)).unwrap(), bytes);
    }
    f.apply(Target::Git, Action::Install);
    assert!(install::git_enabled(&f.root, &f.exe).unwrap());
}
#[test]
fn generated_shim_preserves_literal_arguments_and_always_exits_zero() {
    let f = Fixture::new();
    f.apply(Target::Git, Action::Install);
    let result = Command::new(f.hooks().join("post-checkout"))
        .args(["first argument", "$(touch SHOULD_NOT_EXIST)", "--flag"])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(
        fs::read_to_string(&f.log).unwrap(),
        "hook\ngit\npost-checkout\nfirst argument\n$(touch SHOULD_NOT_EXIST)\n--flag\n"
    );
    assert!(!f.root.join("SHOULD_NOT_EXIST").exists());
    fs::rename(&f.exe, f.exe.with_file_name("retained-executable")).unwrap();
    assert!(
        Command::new(f.hooks().join("pre-commit"))
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[test]
fn foreign_hooks_links_and_configured_managers_are_not_changed() {
    for kind in ["file", "link", "manager"] {
        let f = Fixture::new();
        match kind {
            "file" => fs::write(
                f.hooks().join("pre-commit"),
                b"existing non-executable hook",
            )
            .unwrap(),
            "link" => symlink(&f.exe, f.hooks().join("pre-commit")).unwrap(),
            _ => {
                assert!(
                    Command::new("git")
                        .args(["config", "core.hooksPath", "managed-hooks"])
                        .current_dir(&f.root)
                        .status()
                        .unwrap()
                        .success()
                );
            }
        }
        let plan = install::plan(&f.root, &f.exe, Target::Git, Action::Install).unwrap();
        assert!(!plan.blockers.is_empty());
        assert_eq!(
            install::apply(
                &f.root,
                &f.exe,
                Target::Git,
                Action::Install,
                &plan.plan_sha256
            )
            .unwrap_err()
            .code,
            "HOOK_INSTALL_BLOCKED"
        );
        assert!(!f.state().exists());
        if kind == "file" {
            assert_eq!(
                fs::read(f.hooks().join("pre-commit")).unwrap(),
                b"existing non-executable hook"
            );
        }
    }
}
#[test]
fn reviewed_plan_detects_changed_files_and_owned_edits_are_retained_on_disable() {
    let f = Fixture::new();
    let plan = install::plan(&f.root, &f.exe, Target::Git, Action::Install).unwrap();
    fs::write(f.hooks().join("pre-commit"), b"foreign change").unwrap();
    assert_eq!(
        install::apply(
            &f.root,
            &f.exe,
            Target::Git,
            Action::Install,
            &plan.plan_sha256
        )
        .unwrap_err()
        .code,
        "HOOK_PLAN_CHANGED"
    );
    let f = Fixture::new();
    f.apply(Target::Git, Action::Install);
    fs::write(f.hooks().join("pre-commit"), b"edited owned hook").unwrap();
    assert!(!install::git_enabled(&f.root, &f.exe).unwrap());
    assert!(
        !install::plan(&f.root, &f.exe, Target::Git, Action::Install)
            .unwrap()
            .blockers
            .is_empty()
    );
    f.apply(Target::Git, Action::Uninstall);
    assert_eq!(
        fs::read(f.hooks().join("pre-commit")).unwrap(),
        b"edited owned hook"
    );
}
#[test]
fn incomplete_git_install_stays_disabled_until_a_new_plan_completes_it() {
    if unsafe { libc::geteuid() } == 0 {
        // Root bypasses the directory permission failure used by this fixture.
        return;
    }
    let f = Fixture::new();
    fs::set_permissions(f.hooks(), fs::Permissions::from_mode(0o555)).unwrap();
    let plan = install::plan(&f.root, &f.exe, Target::Git, Action::Install).unwrap();
    assert!(
        install::apply(
            &f.root,
            &f.exe,
            Target::Git,
            Action::Install,
            &plan.plan_sha256
        )
        .is_err()
    );
    let reg: Value =
        serde_json::from_slice(&fs::read(f.state().join("git.json")).unwrap()).unwrap();
    assert_eq!(reg["enabled"], false);
    assert!(!install::git_enabled(&f.root, &f.exe).unwrap());
    fs::set_permissions(f.hooks(), fs::Permissions::from_mode(0o755)).unwrap();
    f.apply(Target::Git, Action::Install);
    assert!(install::git_enabled(&f.root, &f.exe).unwrap());
}
#[test]
fn uninstall_without_registration_does_not_create_ownership() {
    let f = Fixture::new();
    fs::write(f.hooks().join("pre-commit"), b"foreign").unwrap();
    f.apply(Target::Git, Action::Uninstall);
    f.apply(Target::Codex, Action::Uninstall);
    assert!(!f.state().exists());
    assert!(!f.codex().exists());
}
#[test]
fn codex_preserves_foreign_groups_raw_unknown_numbers_and_prior_bytes() {
    let f = Fixture::new();
    let original=br#"{ "description":"foreign", "future":123456789012345678901234567890, "hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"existing handler"}]}],"FutureEvent":[{"arbitrary":9.1234567890123456789012345}]}}"#;
    f.config(original);
    let parsed: Value = serde_json::from_slice(original).unwrap();
    f.apply(Target::Codex, Action::Install);
    let installed = fs::read(f.codex()).unwrap();
    let text = String::from_utf8(installed.clone()).unwrap();
    assert!(text.contains("123456789012345678901234567890"));
    assert!(text.contains("9.1234567890123456789012345"));
    let next: Value = serde_json::from_slice(&installed).unwrap();
    assert_eq!(
        next["hooks"]["UserPromptSubmit"][0],
        parsed["hooks"]["UserPromptSubmit"][0]
    );
    assert_eq!(next["hooks"]["FutureEvent"], parsed["hooks"]["FutureEvent"]);
    assert_eq!(
        next["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "astral hook codex"
    );
    let backup = f.state().join(format!(
        "codex-backup-{}.json",
        ostk_gpt_cache::hash(original)
    ));
    assert_eq!(fs::read(backup).unwrap(), original);
    f.apply(Target::Codex, Action::Install);
    assert_eq!(fs::read(f.codex()).unwrap(), installed);
    f.apply(Target::Codex, Action::Uninstall);
    let after = fs::read(f.codex()).unwrap();
    let next: Value = serde_json::from_slice(&after).unwrap();
    assert_eq!(
        next["hooks"]["UserPromptSubmit"],
        parsed["hooks"]["UserPromptSubmit"]
    );
    assert_eq!(next["hooks"]["FutureEvent"], parsed["hooks"]["FutureEvent"]);
    assert!(
        String::from_utf8(after)
            .unwrap()
            .contains("123456789012345678901234567890")
    );
    f.apply(Target::Codex, Action::Install);
}
#[test]
fn codex_edited_groups_are_preserved_and_duplicates_do_not_grant_ownership() {
    let f = Fixture::new();
    f.apply(Target::Codex, Action::Install);
    let current = String::from_utf8(fs::read(f.codex()).unwrap()).unwrap();
    let changed = current.replacen("astral hook codex", "user changed hook", 1);
    fs::write(f.codex(), changed.as_bytes()).unwrap();
    let plan = install::plan(&f.root, &f.exe, Target::Codex, Action::Install).unwrap();
    assert!(!plan.blockers.is_empty());
    let removal = f.apply(Target::Codex, Action::Uninstall);
    assert!(removal.get("enabled").is_none());
    assert_eq!(removal["registration_enabled"], false);
    assert_eq!(removal["removal_scope"], "exact_owned_groups_only");
    assert_eq!(
        removal["modified_or_unowned_callbacks"],
        "retained; execution is not observed"
    );
    assert!(
        fs::read_to_string(f.codex())
            .unwrap()
            .contains("user changed hook")
    );
    let other = Fixture::new();
    other.config(current.as_bytes());
    assert!(
        !install::plan(&other.root, &other.exe, Target::Codex, Action::Install)
            .unwrap()
            .blockers
            .is_empty()
    );
}
#[test]
fn codex_rejects_unsafe_files_malformed_json_and_stale_review() {
    for raw in [
        b"[]".as_slice(),
        br#"{"hooks":{},"hooks":{}}"#,
        br#"{"hooks":{"SessionStart":"wrong"}}"#,
    ] {
        let f = Fixture::new();
        f.config(raw);
        assert!(install::plan(&f.root, &f.exe, Target::Codex, Action::Install).is_err());
        assert_eq!(fs::read(f.codex()).unwrap(), raw);
    }
    let f = Fixture::new();
    fs::create_dir(f.root.join(".codex")).unwrap();
    symlink(&f.exe, f.codex()).unwrap();
    assert!(install::plan(&f.root, &f.exe, Target::Codex, Action::Install).is_err());
    let f = Fixture::new();
    f.config(b"{}");
    let plan = install::plan(&f.root, &f.exe, Target::Codex, Action::Install).unwrap();
    f.config(b"{\"description\":\"new\"}");
    assert_eq!(
        install::apply(
            &f.root,
            &f.exe,
            Target::Codex,
            Action::Install,
            &plan.plan_sha256
        )
        .unwrap_err()
        .code,
        "HOOK_PLAN_CHANGED"
    );
}
#[test]
fn shared_git_hooks_are_visible_from_linked_worktrees() {
    let f = Fixture::new();
    fs::write(f.root.join("file"), b"fixture").unwrap();
    for args in [
        vec!["add", "file"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-m",
            "fixture",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&f.root)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let linked = f.root.with_file_name("linked");
    assert!(
        Command::new("git")
            .args(["worktree", "add", "-b", "other"])
            .arg(&linked)
            .current_dir(&f.root)
            .output()
            .unwrap()
            .status
            .success()
    );
    f.apply(Target::Git, Action::Install);
    assert!(install::git_enabled(&linked, &f.exe).unwrap());
    let plan = install::plan(&linked, &f.exe, Target::Git, Action::Uninstall).unwrap();
    install::apply(
        &linked,
        &f.exe,
        Target::Git,
        Action::Uninstall,
        &plan.plan_sha256,
    )
    .unwrap();
    assert!(!install::git_enabled(&f.root, &f.exe).unwrap());
}

#[test]
fn codex_status_distinguishes_configuration_from_registration_and_runtime_trust() {
    let f = Fixture::new();
    let absent = install::status(&f.root).unwrap();
    assert_eq!(absent["codex"]["configured"], false);
    assert_eq!(absent["codex"]["registered"], false);
    assert!(!f.state().exists());
    let plan = install::plan(&f.root, &f.exe, Target::Codex, Action::Install).unwrap();
    assert!(plan.executable.is_none());
    assert_eq!(plan.command, "astral hook codex");
    assert_eq!(plan.executable_lookup, "destination_PATH");
    f.apply(Target::Codex, Action::Install);
    let installed = install::status(&f.root).unwrap();
    assert_eq!(installed["codex"]["configured"], true);
    assert_eq!(installed["codex"]["owned_configuration_intact"], true);
    assert_eq!(installed["codex"]["runtime_trust"], "not_observed");
    let bytes = fs::read(f.codex()).unwrap();
    let clone = Fixture::new();
    clone.config(&bytes);
    let observed = install::status(&clone.root).unwrap();
    assert_eq!(observed["codex"]["configured"], true);
    assert_eq!(observed["codex"]["registered"], false);
    assert_eq!(observed["codex"]["owned_configuration_intact"], false);
    assert!(!clone.state().exists());
    fs::rename(f.codex(), f.root.join("retained-hooks.json")).unwrap();
    let missing = install::status(&f.root).unwrap();
    assert_eq!(missing["codex"]["registration_enabled"], true);
    assert_eq!(missing["codex"]["configured"], false);
    assert_eq!(missing["codex"]["owned_configuration_intact"], false);
}

#[test]
fn codex_changed_prior_byte_backup_blocks_replacement_and_retains_files() {
    let f = Fixture::new();
    let prior = br#"{"description":"original"}"#;
    f.config(prior);
    f.apply(Target::Codex, Action::Install);
    let backup = fs::read_dir(f.state())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("codex-backup-")
        })
        .unwrap();
    f.apply(Target::Codex, Action::Uninstall);
    f.config(prior);
    fs::write(&backup, b"changed private backup").unwrap();
    let plan = install::plan(&f.root, &f.exe, Target::Codex, Action::Install).unwrap();
    assert_eq!(
        install::apply(
            &f.root,
            &f.exe,
            Target::Codex,
            Action::Install,
            &plan.plan_sha256
        )
        .unwrap_err()
        .code,
        "HOOK_BACKUP_CHANGED"
    );
    assert_eq!(fs::read(f.codex()).unwrap(), prior);
    assert_eq!(fs::read(backup).unwrap(), b"changed private backup");
}

#[test]
fn partial_codex_install_retains_backup_and_requires_explicit_review() {
    if unsafe { libc::geteuid() } == 0 {
        // Root bypasses the directory permission failure used by this fixture.
        return;
    }
    let f = Fixture::new();
    let original = b"{\"description\":\"existing user configuration\"}";
    f.config(original);
    fs::set_permissions(f.root.join(".codex"), fs::Permissions::from_mode(0o555)).unwrap();
    let plan = install::plan(&f.root, &f.exe, Target::Codex, Action::Install).unwrap();
    assert!(
        install::apply(
            &f.root,
            &f.exe,
            Target::Codex,
            Action::Install,
            &plan.plan_sha256
        )
        .is_err()
    );
    assert_eq!(fs::read(f.codex()).unwrap(), original);
    let observation = install::status(&f.root).unwrap();
    assert_eq!(observation["codex"]["registration_enabled"], false);
    assert_eq!(observation["codex"]["configured"], false);
    assert_eq!(
        install::apply(
            &f.root,
            &f.exe,
            Target::Codex,
            Action::Install,
            &plan.plan_sha256
        )
        .unwrap_err()
        .code,
        "HOOK_PLAN_CHANGED"
    );
    let backup = fs::read_dir(f.state())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("codex-backup-")
        })
        .unwrap();
    assert_eq!(fs::read(backup).unwrap(), original);
    fs::set_permissions(f.root.join(".codex"), fs::Permissions::from_mode(0o755)).unwrap();
    let retained = install::plan(&f.root, &f.exe, Target::Codex, Action::Install).unwrap();
    assert!(!retained.blockers.is_empty());
    // Review disabling the incomplete registration before a fresh installation.
    f.apply(Target::Codex, Action::Uninstall);
    assert_eq!(fs::read(f.codex()).unwrap(), original);
    f.apply(Target::Codex, Action::Install);
    assert_eq!(
        install::status(&f.root).unwrap()["codex"]["owned_configuration_intact"],
        true
    );
}
