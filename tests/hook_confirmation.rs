#![cfg(unix)]

use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{fs::MetadataExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const PROMPT: &str = "Apply this hook setup? [y/N] ";
const DEADLINE: Duration = Duration::from_secs(8);

fn environment(command: &mut Command, home: &Path) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("CODEX_HOME", home.join(".codex"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env_remove("ASTRAL_HOOK_DEPTH")
        .env_remove("ASTRAL_HOOK_CHILD");
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let root = base.join("repo");
        let home = base.join("home");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&home).unwrap();
        let mut git = Command::new("git");
        environment(&mut git, &home);
        let out = git
            .current_dir(&root)
            .args(["init", "--template=", "--initial-branch=main"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Self {
            _temp: temp,
            root,
            home,
        }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_astral"));
        environment(&mut command, &self.home);
        command
            .current_dir(&self.root)
            .arg("--root")
            .arg(&self.root)
            .args(args);
        command
    }
    fn run(&self, args: &[&str], input: &[u8]) -> Output {
        let mut command = self.command(args);
        let mut child = OwnedChild(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap(),
        );
        child.0.stdin.take().unwrap().write_all(input).unwrap();
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                started.elapsed() < DEADLINE,
                "nonterminal command timed out"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        child
            .0
            .stdout
            .take()
            .unwrap()
            .take(65536)
            .read_to_end(&mut stdout)
            .unwrap();
        child
            .0
            .stderr
            .take()
            .unwrap()
            .take(65536)
            .read_to_end(&mut stderr)
            .unwrap();
        Output {
            status,
            stdout,
            stderr,
        }
    }
    fn put(&self, name: &str, bytes: &[u8]) {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn status(&self) -> Value {
        let out = self.run(&["hooks", "status", "--json"], b"");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn installed(&self, target: &str) -> bool {
        let state = self.status();
        if target == "git" {
            state["git"]["enabled"] == true
        } else {
            state["codex"]["configured"] == true && state["codex"]["registration_enabled"] == true
        }
    }
    fn snapshot(&self) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
        fn walk(base: &Path, path: &Path, rows: &mut BTreeMap<PathBuf, (u32, Vec<u8>)>) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).unwrap();
                let bytes = if metadata.is_dir() {
                    Vec::new()
                } else if metadata.file_type().is_symlink() {
                    fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .as_bytes()
                        .to_vec()
                } else {
                    fs::read(&path).unwrap()
                };
                rows.insert(
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    (metadata.mode(), bytes),
                );
                if metadata.is_dir() {
                    walk(base, &path, rows);
                }
            }
        }
        let base = self.root.parent().unwrap();
        let mut rows = BTreeMap::new();
        walk(base, base, &mut rows);
        rows
    }
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            // SAFETY: this test child starts its own process group.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

struct Pty {
    child: OwnedChild,
    master: File,
    transcript: Vec<u8>,
    started: Instant,
}
impl Pty {
    fn start(fixture: &Fixture, args: &[&str], terminal_stderr: bool) -> Self {
        let mut master = -1;
        let mut slave = -1;
        // SAFETY: openpty initializes the two descriptor outputs. No terminal
        // name, size, or termios pointers are supplied.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0,
            "{}",
            io::Error::last_os_error()
        );
        // SAFETY: successful openpty transfers each distinct descriptor once.
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        for file in [&master, &slave] {
            // Do not leak the master into the child; Stdio duplicates the slave.
            assert_eq!(
                unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
                0
            );
        }
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        let stderr = if terminal_stderr {
            Stdio::from(slave.try_clone().unwrap())
        } else {
            Stdio::piped()
        };
        let mut command = fixture.command(args);
        let child = command
            .stdin(Stdio::from(slave))
            .stderr(stderr)
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        Self {
            child: OwnedChild(child),
            master,
            transcript: Vec::new(),
            started: Instant::now(),
        }
    }
    fn drain(&mut self) {
        let mut buffer = [0; 4096];
        loop {
            match self.master.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    self.transcript.extend_from_slice(&buffer[..n]);
                    assert!(
                        self.transcript.len() <= 65536,
                        "terminal output exceeds test bound"
                    );
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.raw_os_error() == Some(libc::EIO) =>
                {
                    break;
                }
                Err(e) => panic!("read terminal: {e}"),
            }
        }
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.transcript).into_owned()
    }
    fn prompt(&mut self) {
        loop {
            self.drain();
            if self.text().contains(PROMPT) {
                return;
            }
            if let Some(status) = self.child.0.try_wait().unwrap() {
                panic!("exited before confirmation {status}: {}", self.text());
            }
            assert!(
                self.started.elapsed() < DEADLINE,
                "confirmation prompt timed out: {}",
                self.text()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }
    fn finish(mut self) -> (ExitStatus, Vec<u8>, String) {
        let status = loop {
            self.drain();
            if let Some(status) = self.child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                self.started.elapsed() < DEADLINE,
                "terminal command timed out: {}",
                self.text()
            );
            thread::sleep(Duration::from_millis(10));
        };
        self.drain();
        let mut stdout = Vec::new();
        self.child
            .0
            .stdout
            .take()
            .unwrap()
            .take(65536)
            .read_to_end(&mut stdout)
            .unwrap();
        if let Some(stderr) = self.child.0.stderr.take() {
            stderr
                .take(65536)
                .read_to_end(&mut self.transcript)
                .unwrap();
        }
        (status, stdout, self.text())
    }
}

#[test]
fn interactive_yes_installs_and_uninstalls_both_targets_after_review() {
    for (target, answer) in [("git", b"y\n".as_slice()), ("codex", b"YES\n".as_slice())] {
        let fixture = Fixture::new();
        for action in ["install", "uninstall"] {
            let before = fixture.snapshot();
            let mut child = Pty::start(&fixture, &["hooks", action, target], true);
            child.prompt();
            assert_eq!(
                fixture.snapshot(),
                before,
                "displaying the plan must not write"
            );
            child.send(answer);
            let (status, stdout, transcript) = child.finish();
            assert!(status.success(), "{transcript}");
            assert!(
                stdout.is_empty(),
                "interactive output belongs on the terminal"
            );
            assert_eq!(transcript.matches(PROMPT).count(), 1);
            assert_eq!(fixture.installed(target), action == "install");
        }
    }
}

#[test]
fn negative_blank_unknown_and_eof_answers_decline_without_any_writes() {
    for answer in [
        b"n\n".as_slice(),
        b"\n",
        b"no\n",
        b"maybe\n",
        b"\x04",
        b"yes\x04\x04",
    ] {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        let mut child = Pty::start(&fixture, &["hooks", "install", "git"], true);
        child.prompt();
        child.send(answer);
        let (status, stdout, transcript) = child.finish();
        assert!(status.success(), "{transcript}");
        assert!(stdout.is_empty());
        assert_eq!(
            fixture.snapshot(),
            before,
            "declining must preserve every fixture file"
        );
    }
}

#[test]
fn yes_requires_a_line_ending_before_it_authorizes() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let mut child = Pty::start(&fixture, &["hooks", "install", "git"], true);
    child.prompt();
    child.send(b"Yes");
    thread::sleep(Duration::from_millis(100));
    assert!(child.child.0.try_wait().unwrap().is_none());
    assert_eq!(fixture.snapshot(), before);
    child.send(b"\n");
    let (status, _, transcript) = child.finish();
    assert!(status.success(), "{transcript}");
    assert!(fixture.installed("git"));
}

#[test]
fn files_changed_after_prompt_are_preserved_and_invalidate_confirmation() {
    for (target, path, bytes) in [
        (
            "git",
            ".git/hooks/pre-commit",
            b"foreign hook arrived during review\n".as_slice(),
        ),
        (
            "codex",
            ".codex/hooks.json",
            b"{\"hooks\":{},\"foreign\":true}\n".as_slice(),
        ),
    ] {
        let fixture = Fixture::new();
        let mut child = Pty::start(&fixture, &["hooks", "install", target], true);
        child.prompt();
        fixture.put(path, bytes);
        let changed = fixture.snapshot();
        child.send(b"yes\n");
        let (status, _, transcript) = child.finish();
        assert!(!status.success());
        assert!(transcript.contains("HOOK_PLAN_CHANGED"), "{transcript}");
        assert_eq!(fixture.snapshot(), changed);
        assert_eq!(fs::read(fixture.root.join(path)).unwrap(), bytes);
    }
}

#[test]
fn blockers_report_without_prompt_or_mutation() {
    let fixture = Fixture::new();
    fixture.put(".git/hooks/pre-commit", b"foreign hook\n");
    let before = fixture.snapshot();
    let child = Pty::start(&fixture, &["hooks", "install", "git"], true);
    let (_, _, transcript) = child.finish();
    assert!(!transcript.contains(PROMPT), "{transcript}");
    assert!(!transcript.is_empty());
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn piped_yes_is_only_a_human_preview_with_a_hint() {
    for target in ["git", "codex"] {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        let out = fixture.run(&["hooks", "install", target], b"yes\n");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let plan = String::from_utf8(out.stdout).unwrap();
        assert!(plan.to_lowercase().contains(target), "{plan}");
        assert!(plan.to_lowercase().contains("install"), "{plan}");
        assert!(plan.contains("--apply"), "{plan}");
        assert!(!out.stderr.is_empty());
        assert!(!String::from_utf8_lossy(&out.stderr).contains(PROMPT));
        assert_eq!(fixture.snapshot(), before);
    }
}

#[test]
fn terminal_stdin_with_redirected_stderr_is_still_a_preview() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let child = Pty::start(&fixture, &["hooks", "install", "git"], false);
    let (status, stdout, transcript) = child.finish();
    assert!(status.success(), "{transcript}");
    let plan = String::from_utf8(stdout).unwrap();
    assert!(plan.to_lowercase().contains("git"), "{plan}");
    assert!(!transcript.contains(PROMPT));
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn explicit_yes_installs_and_uninstalls_both_targets_without_prompt() {
    for target in ["git", "codex"] {
        let fixture = Fixture::new();
        for action in ["install", "uninstall"] {
            let out = fixture.run(&["hooks", action, target, "--yes", "--json"], b"");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let result: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(result["operation"], "hooks_apply");
            assert_eq!(result["action"], action);
            assert!(!String::from_utf8_lossy(&out.stderr).contains(PROMPT));
            assert_eq!(fixture.installed(target), action == "install");
        }
    }
}

#[test]
fn explicit_json_in_a_terminal_never_prompts_or_writes() {
    for target in ["git", "codex"] {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        let child = Pty::start(&fixture, &["hooks", "install", target, "--json"], true);
        let (status, stdout, transcript) = child.finish();
        assert!(status.success(), "{transcript}");
        let plan: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(plan["target"], target);
        assert!(!transcript.contains(PROMPT));
        assert_eq!(fixture.snapshot(), before);
    }
}

#[test]
fn confirmation_flags_are_mutually_exclusive_without_side_effects() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let hash = "a".repeat(64);
    for action in ["install", "uninstall"] {
        for flags in [
            vec!["--yes", "--dry-run"],
            vec!["--yes", "--apply", &hash],
            vec!["--dry-run", "--apply", &hash],
        ] {
            let mut args = vec!["hooks", action, "git"];
            args.extend(flags);
            let out = fixture.run(&args, b"yes\n");
            assert_eq!(out.status.code(), Some(2));
            assert!(!String::from_utf8_lossy(&out.stderr).contains(PROMPT));
            assert_eq!(fixture.snapshot(), before);
        }
    }
}

#[test]
fn legacy_exact_hash_apply_still_works_without_confirmation() {
    for target in ["git", "codex"] {
        let fixture = Fixture::new();
        for action in ["install", "uninstall"] {
            let preview = fixture.run(&["hooks", action, target, "--dry-run", "--json"], b"");
            assert!(preview.status.success());
            let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();
            let hash = plan["plan_sha256"].as_str().unwrap();
            let out = fixture.run(&["hooks", action, target, "--apply", hash, "--json"], b"");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let applied: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(applied["plan_sha256"], hash);
            assert_eq!(fixture.installed(target), action == "install");
            assert!(!String::from_utf8_lossy(&out.stderr).contains(PROMPT));
        }
    }
}

#[test]
fn declined_uninstall_preserves_existing_hooks_and_registration() {
    for target in ["git", "codex"] {
        let fixture = Fixture::new();
        let install = fixture.run(&["hooks", "install", target, "--yes"], b"");
        assert!(install.status.success());
        assert!(fixture.installed(target));
        let before = fixture.snapshot();
        for answer in [b"no\n".as_slice(), b"\n", b"\x04"] {
            let mut child = Pty::start(&fixture, &["hooks", "uninstall", target], true);
            child.prompt();
            child.send(answer);
            let (status, stdout, transcript) = child.finish();
            assert!(status.success(), "{transcript}");
            assert!(stdout.is_empty());
            assert_eq!(fixture.snapshot(), before);
            assert!(fixture.installed(target));
        }
    }
}
