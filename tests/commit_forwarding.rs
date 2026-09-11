#![cfg(unix)]
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn environment(command: &mut Command) -> &mut Command {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Forwarding Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Forwarding Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_TERMINAL_PROMPT", "0")
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    log: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let f = Self {
            log: root.join("invocations"),
            root,
            _temp: temp,
        };
        f.git(&["init", "--initial-branch=main"]);
        fs::write(f.root.join("file"), b"commit fixture\n").unwrap();
        f.git(&["add", "file"]);
        f
    }
    fn git(&self, args: &[&str]) -> Output {
        let output = environment(Command::new("git").current_dir(&self.root))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    fn command(&self, wrapper: bool) -> Command {
        let mut command = Command::new(if wrapper {
            env!("CARGO_BIN_EXE_astral")
        } else {
            "git"
        });
        environment(&mut command)
            .current_dir(&self.root)
            .env("FORWARDING_LOG", &self.log)
            .env("FORWARDING_MARKER", "literal $(do-not-execute) value");
        command.arg("commit");
        if wrapper {
            command.arg("--");
        }
        command
    }
    fn script(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    fn configure(&self, name: &str, value: &Path) {
        let output = environment(Command::new("git").current_dir(&self.root))
            .arg("config")
            .arg(name)
            .arg(value)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
}

#[test]
fn actual_git_signing_program_and_failure_are_preserved_by_commit_wrapper() {
    let f = Fixture::new();
    let signer = f.script("signer with spaces", "printf '%s\\0' \"$@\" >> \"$FORWARDING_LOG\"\nprintf '%s\\0' \"$FORWARDING_MARKER\" >> \"$FORWARDING_LOG\"\ncat >/dev/null\nexit 23");
    f.configure("gpg.program", &signer);
    f.git(&["config", "gpg.format", "openpgp"]);
    f.git(&["config", "commit.gpgsign", "true"]);
    f.git(&["config", "user.signingkey", "fixture signing key"]);
    let plain = f
        .command(false)
        .args(["-m", "signed fixture"])
        .output()
        .unwrap();
    assert!(!plain.status.success());
    let first = fs::read(&f.log).unwrap();
    assert!(!first.is_empty());
    assert!(String::from_utf8_lossy(&first).contains("fixture signing key"));
    assert!(String::from_utf8_lossy(&first).contains("literal $(do-not-execute) value"));
    let wrapped = f
        .command(true)
        .args(["-m", "signed fixture"])
        .output()
        .unwrap();
    assert_eq!(wrapped.status.code(), plain.status.code());
    let all = fs::read(&f.log).unwrap();
    assert_eq!(all, [first.as_slice(), first.as_slice()].concat());
    assert!(String::from_utf8_lossy(&wrapped.stderr).contains("failed to sign"));
    let explicit = f
        .command(true)
        .args(["--no-gpg-sign", "-m", "explicit unsigned fixture"])
        .output()
        .unwrap();
    assert!(
        explicit.status.success(),
        "{}",
        String::from_utf8_lossy(&explicit.stderr)
    );
    assert_eq!(fs::read(&f.log).unwrap(), all);
}

#[test]
fn environment_selected_hook_manager_and_its_rejection_remain_authoritative() {
    let f = Fixture::new();
    f.git(&["config", "commit.gpgsign", "false"]);
    let manager = f.root.join("manager with spaces");
    f.script("manager with spaces/pre-commit", "printf '%s\\n' \"$FORWARDING_MARKER\" >> \"$FORWARDING_LOG\"\nprintf 'ENV_MANAGER_REJECTED\\n' >&2\nexit 19");
    // A different repository-local manager must not override the caller's env.
    f.git(&["config", "core.hooksPath", "different-local-manager"]);
    let mut codes = Vec::new();
    for wrapper in [false, true] {
        let result = f
            .command(wrapper)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .env("GIT_CONFIG_VALUE_0", &manager)
            .args(["-m", "rejected by configured manager"])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("ENV_MANAGER_REJECTED"));
        codes.push(result.status.code());
    }
    assert_eq!(codes[0], codes[1]);
    assert_eq!(
        fs::read_to_string(&f.log).unwrap(),
        "literal $(do-not-execute) value\nliteral $(do-not-execute) value\n"
    );
    let explicitly_skipped = f
        .command(true)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_0", &manager)
        .args(["--no-verify", "-m", "explicit user skip"])
        .output()
        .unwrap();
    assert!(explicitly_skipped.status.success());
    assert_eq!(fs::read_to_string(&f.log).unwrap().lines().count(), 2);
}

#[test]
fn environment_signing_overrides_reach_real_git_without_rewriting() {
    let f = Fixture::new();
    f.git(&["config", "commit.gpgsign", "false"]);
    f.git(&["config", "gpg.program", "must-not-run-repository-signer"]);
    let signer = f.script(
        "environment signer",
        "printf 'ENV_SIGNER\\n' >> \"$FORWARDING_LOG\"\ncat >/dev/null\nexit 29",
    );
    let mut codes = Vec::new();
    for wrapper in [false, true] {
        let result = f
            .command(wrapper)
            .env("GIT_CONFIG_COUNT", "3")
            .env("GIT_CONFIG_KEY_0", "gpg.program")
            .env("GIT_CONFIG_VALUE_0", &signer)
            .env("GIT_CONFIG_KEY_1", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_1", "true")
            .env("GIT_CONFIG_KEY_2", "gpg.format")
            .env("GIT_CONFIG_VALUE_2", "openpgp")
            .args(["-m", "signed through caller environment"])
            .output()
            .unwrap();
        assert!(!result.status.success());
        codes.push(result.status.code());
    }
    assert_eq!(codes[0], codes[1]);
    assert_eq!(
        fs::read_to_string(&f.log).unwrap(),
        "ENV_SIGNER\nENV_SIGNER\n"
    );
}
