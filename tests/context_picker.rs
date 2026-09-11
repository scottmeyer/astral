#![cfg(unix)]

use astral::{
    project::Project,
    workspace::{WorkerMetadata, WorktreeBinding},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{MetadataExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT: usize = 256 * 1024;
const CONTEXT: &str = "Choose a context";
const WORK: &str = "Choose a work item";
const REVIEW: &str = "Review launch";
const DOCUMENT: &str = "PRIVATE_DOCUMENT_BODY_MUST_NOT_APPEAR_IN_PICKER";

fn environment(command: &mut Command, home: &Path) {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") || key.to_string_lossy().starts_with("ASTRAL_")
        {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("CODEX_HOME", home.join(".codex"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "maintenance.auto")
        .env("GIT_CONFIG_VALUE_1", "false")
        .env("GIT_CONFIG_KEY_2", "gc.auto")
        .env("GIT_CONFIG_VALUE_2", "0")
        .env("TERM", "xterm-256color")
        .env("NO_COLOR", "1")
        .env("ASTRAL_CODEX_BIN", home.join("codex-must-not-run"));
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
        let fixture = Self {
            _temp: temp,
            root: base.join("repo"),
            home: base.join("home"),
        };
        fs::create_dir(&fixture.root).unwrap();
        fs::create_dir(&fixture.home).unwrap();
        fixture.put(
            ".astral/project.toml",
            b"schema_version=1\nid='picker-fixture'\nname='Picker fixture'\ndescription='Offline navigation'\ncore='core'\nprojections='projections'\nwork_items='work/items.jsonl'\n[identity]\nscope='local'\nruntime_bindings='external'\n[subsystems]\nproject-context='core/project-context'\nweb='core/web'\nstorage='core/storage'\n",
        );
        for file in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            fixture.put(&format!(".astral/core/{file}"), DOCUMENT.as_bytes());
        }
        for (id, projection) in [
            ("project-context", "default-seed"),
            ("web", "web-seed"),
            ("storage", "storage-seed"),
        ] {
            fixture.put(
                &format!(".astral/core/{id}/subsystem.toml"),
                format!("schema_version=1\nid='{id}'\npurpose='Picker subsystem {id}'\nreadme='README.md'\nrules=[]\ndecisions=[]\nwork_items=[]\nprojection='{projection}'\n").as_bytes(),
            );
            fixture.put(&format!(".astral/core/{id}/README.md"), DOCUMENT.as_bytes());
            fixture.put(
                &format!(".astral/projections/{projection}/projection.toml"),
                format!("schema_version=1\nid='{projection}'\nkind='fresh-context'\nsubsystems=['{id}']\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n").as_bytes(),
            );
            fixture.put(
                &format!(".astral/projections/{projection}/handoff.md"),
                DOCUMENT.as_bytes(),
            );
        }
        let items = [
            ("AST-complete", "Finished task", "complete"),
            ("AST-open", "Needle review", "open"),
            ("AST-blocked", "Blocked task", "blocked"),
            ("AST-active", "Active task", "in_progress"),
        ];
        let jsonl: String = items.into_iter().map(|(id, title, status)| {
            format!("{}\n", json!({"schema_version":1,"id":id,"title":title,"status":status,"depends_on":[],"acceptance":["Review without inference"]}))
        }).collect();
        fixture.put(".astral/work/items.jsonl", jsonl.as_bytes());
        fixture.git(&["init", "--template=", "--initial-branch=main"]);
        fixture.git(&["add", ".astral"]);
        fixture.git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "Fixture",
        ]);
        fixture
    }
    fn put(&self, relative: &str, bytes: &[u8]) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn git(&self, args: &[&str]) {
        let mut command = Command::new("git");
        environment(&mut command, &self.home);
        let result = command.current_dir(&self.root).args(args).output().unwrap();
        assert!(result.status.success(), "{result:?}");
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_astral"));
        environment(&mut command, &self.home);
        command.current_dir(&self.root).args(args);
        let stub = self.home.join("fake-codex");
        if stub.exists() {
            command.env("ASTRAL_CODEX_BIN", stub);
        }
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        let mut child = OwnedChild(
            self.command(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap(),
        );
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
            .take(OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut stdout)
            .unwrap();
        child
            .0
            .stderr
            .take()
            .unwrap()
            .take(OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut stderr)
            .unwrap();
        assert!(stdout.len() <= OUTPUT_LIMIT && stderr.len() <= OUTPUT_LIMIT);
        Output {
            status,
            stdout,
            stderr,
        }
    }
    fn bound_storage_worker(&self) -> WorktreeBinding {
        let mut binding = WorktreeBinding::acquire_with_root(
            &self.root,
            "picker-fixture",
            "storage",
            "AST-open",
            None,
        )
        .unwrap();
        let context = Project::load(binding.root())
            .unwrap()
            .launch_context("storage", Some("AST-open"))
            .unwrap();
        binding
            .update_worker_metadata(WorkerMetadata {
                context_initialized: true,
                thread_id: Some("01a10000-1234-7000-8000-000000000091".into()),
                model: Some("gpt-6-astra".into()),
                provider: Some("openai".into()),
                selection_digest: Some(context.current_context.selection_digest),
                selected_bundle_recorded: true,
                ..Default::default()
            })
            .unwrap();
        binding
    }
    fn snapshot(&self) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
        fn walk(base: &Path, path: &Path, rows: &mut BTreeMap<PathBuf, (u32, Vec<u8>)>) {
            for entry in fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                let meta = fs::symlink_metadata(&path).unwrap();
                let bytes = if meta.is_dir() {
                    Vec::new()
                } else if meta.file_type().is_symlink() {
                    fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec()
                } else {
                    fs::read(&path).unwrap()
                };
                rows.insert(
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    (meta.mode(), bytes),
                );
                if meta.is_dir() {
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
            // SAFETY: only the process group created for this fixture is signalled.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn terminal_state(file: &File) -> libc::termios {
    let mut value = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: tcgetattr writes the supplied termios on success.
    assert_eq!(
        unsafe { libc::tcgetattr(file.as_raw_fd(), value.as_mut_ptr()) },
        0
    );
    unsafe { value.assume_init() }
}
fn assert_restored(before: &libc::termios, after: &libc::termios) {
    assert_eq!(after.c_iflag, before.c_iflag, "input terminal flags leaked");
    assert_eq!(
        after.c_oflag, before.c_oflag,
        "output terminal flags leaked"
    );
    assert_eq!(
        after.c_cflag, before.c_cflag,
        "control terminal flags leaked"
    );
    assert_eq!(after.c_lflag, before.c_lflag, "local terminal flags leaked");
    assert_eq!(
        after.c_cc, before.c_cc,
        "terminal control characters changed"
    );
    // SAFETY: both references came from successful tcgetattr calls.
    assert_eq!(unsafe { libc::cfgetispeed(after) }, unsafe {
        libc::cfgetispeed(before)
    });
    assert_eq!(unsafe { libc::cfgetospeed(after) }, unsafe {
        libc::cfgetospeed(before)
    });
}
fn plain(text: &[u8]) -> String {
    let source = String::from_utf8_lossy(text);
    let mut chars = source.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for part in chars.by_ref() {
                if ('@'..='~').contains(&part) {
                    break;
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

struct Pty {
    child: OwnedChild,
    master: File,
    terminal: File,
    before: libc::termios,
    transcript: Vec<u8>,
    started: Instant,
}
impl Pty {
    fn start(fixture: &Fixture, args: &[&str]) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize {
            ws_row: 40,
            ws_col: 200,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty initializes two distinct fds; size is a valid winsize.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &raw mut size,
                )
            },
            0,
            "{}",
            io::Error::last_os_error()
        );
        // SAFETY: each fd returned by openpty is transferred exactly once.
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        for file in [&master, &slave] {
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
        let before = terminal_state(&slave);
        let terminal = slave.try_clone().unwrap();
        let child = fixture
            .command(args)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave))
            // Picker output belongs to stdout, not the diagnostic stream.
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        Self {
            child: OwnedChild(child),
            master,
            terminal,
            before,
            transcript: Vec::new(),
            started: Instant::now(),
        }
    }
    fn drain(&mut self) {
        let mut bytes = [0; 4096];
        loop {
            match self.master.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    self.transcript.extend_from_slice(&bytes[..n]);
                    assert!(
                        self.transcript.len() <= OUTPUT_LIMIT,
                        "picker output exceeded fixture bound"
                    );
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.raw_os_error() == Some(libc::EIO) =>
                {
                    break;
                }
                Err(e) => panic!("read picker terminal: {e}"),
            }
        }
    }
    fn mark(&mut self) -> usize {
        self.drain();
        self.transcript.len()
    }
    fn wait_for(&mut self, from: usize, expected: &str) -> String {
        loop {
            self.drain();
            let text = plain(&self.transcript[from..]);
            if text.contains(expected) {
                return text;
            }
            if let Some(status) = self.child.0.try_wait().unwrap() {
                let mut stderr = String::new();
                self.child
                    .0
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                panic!("picker exited {status} before {expected:?}: {text}\n{stderr}");
            }
            assert!(
                self.started.elapsed() < DEADLINE,
                "picker timeout waiting for {expected:?}: {}",
                plain(&self.transcript)
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn send(&mut self, bytes: &[u8]) -> usize {
        let mark = self.mark();
        self.master.write_all(bytes).unwrap();
        mark
    }
    fn go(&mut self, bytes: &[u8], expected: &str) -> String {
        let mark = self.send(bytes);
        self.wait_for(mark, expected)
    }
    fn finish(mut self) -> (ExitStatus, String, String) {
        let status = loop {
            self.drain();
            if let Some(status) = self.child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                self.started.elapsed() < DEADLINE,
                "picker exit timed out: {}",
                plain(&self.transcript)
            );
            thread::sleep(Duration::from_millis(10));
        };
        self.drain();
        assert_restored(&self.before, &terminal_state(&self.terminal));
        let mut stderr = String::new();
        self.child
            .0
            .stderr
            .take()
            .unwrap()
            .take(OUTPUT_LIMIT as u64 + 1)
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(stderr.len() <= OUTPUT_LIMIT);
        (status, plain(&self.transcript), stderr)
    }
}

#[test]
fn tty_context_list_opens_picker_and_escape_restores_terminal_without_writes() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let mut child = Pty::start(&fixture, &["context", "list"]);
    child.wait_for(0, CONTEXT);
    child.send(b"\x1b");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
    assert!(!output.contains(DOCUMENT));
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn ctrl_c_cancels_from_each_screen_without_mutation_or_terminal_leaks() {
    for screen in 0..3 {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        let mut child = Pty::start(&fixture, &["project", "web", "--pick"]);
        child.wait_for(0, CONTEXT);
        if screen >= 1 {
            child.go(b"\r", WORK);
        }
        if screen >= 2 {
            child.go(b"\r", REVIEW);
        }
        child.send(b"\x03");
        let (status, output, stderr) = child.finish();
        assert!(status.success(), "screen {screen}: {output}\n{stderr}");
        assert_eq!(fixture.snapshot(), before, "cancel from screen {screen}");
    }
}

#[test]
fn work_items_are_status_ordered_filterable_and_back_navigation_is_nonmutating() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let mut child = Pty::start(&fixture, &["project", "web", "--pick"]);
    child.wait_for(0, CONTEXT);
    let work_mark = child.send(b"\r");
    let work_screen = child.wait_for(work_mark, "Finished task");
    assert!(work_screen.contains(WORK));
    let positions: Vec<_> = [
        "Continue without a work item",
        "AST-active",
        "AST-open",
        "AST-blocked",
        "AST-complete",
    ]
    .into_iter()
    .map(|label| work_screen.find(label).unwrap())
    .collect();
    assert!(positions.windows(2).all(|p| p[0] < p[1]), "{work_screen}");
    child.go(b"Needle", "Search: Needle");
    let reviewed = child.go(b"\r", REVIEW);
    assert!(reviewed.contains("AST-open"), "{reviewed}");
    assert!(reviewed.contains("subsystem:web"), "{reviewed}");
    assert!(reviewed.contains("Launch"), "{reviewed}");
    child.go(b"\x1b", WORK);
    child.go(b"\x1b", CONTEXT);
    child.send(b"\x1b");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn context_filter_empty_results_backspace_and_review_back_choice_work() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let mut child = Pty::start(&fixture, &["project", "--pick"]);
    child.wait_for(0, CONTEXT);
    child.go(b"zzzz", "No matches.");
    child.go(b"\x7f\x7f\x7f\x7f", "subsystem:web");
    child.go(b"subsystem:storage", "Search: subsystem:storage");
    child.go(b"\r", WORK);
    let review = child.go(b"\r", REVIEW);
    assert!(review.contains("subsystem:storage"), "{review}");
    assert!(review.contains("Work: none"), "{review}");
    child.go(b"\x1b[B\r", WORK);
    child.go(b"\x1b", CONTEXT);
    child.send(b"\x1b");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn explicit_work_preselection_and_literal_child_json_are_retained_for_review() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let mut child = Pty::start(
        &fixture,
        &[
            "project",
            "web",
            "--pick",
            "--work",
            "AST-open",
            "--proxy",
            "--",
            "--json",
            "review-only-marker",
        ],
    );
    child.wait_for(0, CONTEXT);
    child.go(b"\r", WORK);
    let review = child.go(b"\r", REVIEW);
    assert!(review.contains("AST-open"), "{review}");
    assert!(review.contains("Launch with --proxy"), "{review}");
    assert!(review.contains("--json"), "{review}");
    assert!(review.contains("review-only-marker"), "{review}");
    child.send(b"\x03");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn json_and_plain_modes_on_a_tty_remain_reports_without_a_picker() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    for args in [
        vec!["context", "list", "--json"],
        vec!["--json", "context", "list"],
        vec!["context", "list", "--plain"],
        vec!["--plain", "context", "list"],
    ] {
        let child = Pty::start(&fixture, &args);
        let (status, output, stderr) = child.finish();
        assert!(status.success(), "{args:?}: {output}\n{stderr}");
        assert!(!output.contains(CONTEXT), "{output}");
        assert!(stderr.is_empty(), "{stderr}");
        if args.contains(&"--json") {
            let value: Value = serde_json::from_str(&output).unwrap();
            assert_eq!(value["project_id"], "picker-fixture");
        } else {
            assert!(output.contains("subsystem:web"), "{output}");
            assert!(serde_json::from_str::<Value>(&output).is_err());
        }
    }
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn redirected_list_reports_and_explicit_pick_fails_fast_without_a_terminal() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let listed = fixture.run(&["context", "list"]);
    assert!(listed.status.success(), "{listed:?}");
    assert!(!String::from_utf8_lossy(&listed.stdout).contains(CONTEXT));
    assert!(String::from_utf8_lossy(&listed.stdout).contains("subsystem:web"));
    let picked = fixture.run(&["project", "--pick"]);
    assert_eq!(picked.status.code(), Some(2), "{picked:?}");
    assert!(picked.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&picked.stderr).contains("PICKER_REQUIRES_TTY"),
        "{picked:?}"
    );
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn incompatible_explicit_picker_options_fail_before_opening_terminal_or_runtime() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    for suffix in [
        vec!["--json"],
        vec!["--plain"],
        vec!["--inspect"],
        vec!["--non-interactive"],
        vec!["--resume", "0123456789abcdef0123456789abcdef"],
    ] {
        let mut args = vec!["project", "--pick"];
        args.extend(suffix);
        let (status, output, stderr) = Pty::start(&fixture, &args).finish();
        assert_eq!(status.code(), Some(2), "{args:?}: {output}\n{stderr}");
        assert!(!output.contains(CONTEXT));
        assert!(!stderr.is_empty());
        if args.contains(&"--json") {
            assert!(
                serde_json::from_str::<Value>(&stderr)
                    .unwrap()
                    .get("error")
                    .is_some()
            );
        }
    }
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn changed_context_after_review_is_rejected_before_any_launch_side_effect() {
    let fixture = Fixture::new();
    let mut child = Pty::start(&fixture, &["project", "web", "--pick"]);
    child.wait_for(0, CONTEXT);
    child.go(b"\r", WORK);
    child.go(b"\r", REVIEW);
    fixture.put(
        ".astral/core/web/README.md",
        b"Changed during review; no runtime action is authorized by the old preview.",
    );
    let after_edit = fixture.snapshot();
    child.send(b"\r");
    let (status, output, stderr) = child.finish();
    assert_eq!(status.code(), Some(2), "{output}\n{stderr}");
    assert!(stderr.contains("PICKER_CHANGED"), "{stderr}");
    assert!(
        !stderr.contains("codex-must-not-run"),
        "stale review reached runtime setup: {stderr}"
    );
    assert_eq!(fixture.snapshot(), after_edit);
}

#[test]
fn existing_worker_review_keeps_its_recorded_selector_and_checkout() {
    let fixture = Fixture::new();
    let binding = fixture.bound_storage_worker();
    let worker_root = binding.root().to_path_buf();
    drop(binding);
    let before = fixture.snapshot();
    let mut child = Pty::start(
        &fixture,
        &["project", "web", "--pick", "--work", "AST-open"],
    );
    child.wait_for(0, CONTEXT);
    child.go(b"\r", WORK);
    let review = child.go(b"\r", REVIEW);
    assert!(review.contains("Resume"), "{review}");
    assert!(review.contains("Context: storage"), "{review}");
    assert!(review.contains(worker_root.to_str().unwrap()), "{review}");
    assert!(review.contains("recorded context"), "{review}");
    child.send(b"\x03");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn busy_worker_has_a_back_only_diagnostic_and_is_not_resumed() {
    let fixture = Fixture::new();
    let binding = fixture.bound_storage_worker();
    let before = fixture.snapshot();
    let mut child = Pty::start(
        &fixture,
        &["project", "web", "--pick", "--work", "AST-open"],
    );
    child.wait_for(0, CONTEXT);
    child.go(b"\r", WORK);
    let error = child.go(b"\r", "Cannot launch this selection");
    assert!(error.contains("PICKER_WORK_UNAVAILABLE"), "{error}");
    assert!(error.contains("Back"), "{error}");
    assert!(!error.contains("Review launch"), "{error}");
    child.go(b"\x1b", WORK);
    child.send(b"\x03");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert_eq!(fixture.snapshot(), before);
    drop(binding);
}

#[test]
fn new_worker_rejects_source_cd_before_reviewing_an_impossible_launch() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let source = fixture.root.to_str().unwrap();
    let mut child = Pty::start(
        &fixture,
        &[
            "project", "web", "--pick", "--work", "AST-open", "--", "--cd", source,
        ],
    );
    child.wait_for(0, CONTEXT);
    child.go(b"\r", WORK);
    let diagnostic = child.go(b"\r", "Cannot launch this selection");
    assert!(!diagnostic.contains("Review launch"), "{diagnostic}");
    child.send(b"\x03");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert_eq!(fixture.snapshot(), before);
}

const CODEX_STUB: &str = r#"#!/usr/bin/env python3
import json, os, pathlib, sys, termios
log = pathlib.Path(__file__).with_suffix('.jsonl')
def record(kind, **values):
    with log.open('a', encoding='utf-8') as output:
        output.write(json.dumps(dict(kind=kind, **values)) + '\n')
record('argv', args=sys.argv[1:], cwd=os.getcwd())
if sys.argv[1:] == ['--version']:
    print('codex-cli 0.154.0')
elif sys.argv[1] == 'app-server':
    for line in sys.stdin:
        request = json.loads(line)
        if 'id' not in request:
            continue
        record('rpc', method=request['method'])
        result = {}
        if request['method'] == 'config/read':
            result = dict(config={})
        elif request['method'] == 'thread/resume':
            result = dict(thread=dict(id=request['params']['threadId'], ephemeral=False), cwd=os.getcwd(), model='gpt-6-astra', modelProvider='openai')
        elif request['method'] == 'thread/start':
            raise AssertionError('bound worker must resume, not create a thread')
        print(json.dumps(dict(id=request['id'], result=result)), flush=True)
    record('closed')
elif sys.argv[1] == 'resume':
    assert any(json.loads(line)['kind'] == 'closed' for line in log.read_text().splitlines())
    flags = termios.tcgetattr(0)
    assert flags[3] & termios.ICANON and flags[3] & termios.ECHO, 'picker left terminal in raw mode'
    record('resumed', args=sys.argv[1:], cwd=os.getcwd(), canonical=bool(flags[3] & termios.ICANON), echo=bool(flags[3] & termios.ECHO))
    print('STUB_RESUMED')
else:
    raise AssertionError('unexpected fixture invocation')
"#;

#[test]
fn confirmed_existing_worker_restores_terminal_before_stub_and_forwards_literal_argv() {
    let fixture = Fixture::new();
    let binding = fixture.bound_storage_worker();
    let worker_root = binding.root().to_path_buf();
    drop(binding);
    let stub = fixture.home.join("fake-codex");
    fs::write(&stub, CODEX_STUB).unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o700)).unwrap();
    let literal = "literal $(touch forbidden) ; --pick";
    let mut child = Pty::start(
        &fixture,
        &[
            "project", "web", "--pick", "--work", "AST-open", "--", "--json", literal,
        ],
    );
    child.wait_for(0, CONTEXT);
    child.go(b"\r", WORK);
    let review = child.go(b"\r", REVIEW);
    assert!(review.contains("Resume"), "{review}");
    child.send(b"\r");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert!(output.contains("STUB_RESUMED"), "{output}");
    let rows: Vec<Value> = fs::read_to_string(stub.with_extension("jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let resumed: Vec<_> = rows.iter().filter(|row| row["kind"] == "resumed").collect();
    assert_eq!(resumed.len(), 1);
    assert_eq!(
        resumed[0]["args"],
        json!([
            "resume",
            "01a10000-1234-7000-8000-000000000091",
            "--json",
            literal
        ])
    );
    assert_eq!(resumed[0]["cwd"], worker_root.to_str().unwrap());
    assert_eq!(resumed[0]["canonical"], true);
    assert_eq!(resumed[0]["echo"], true);
    assert!(!fixture.root.join("forbidden").exists());
    assert!(!worker_root.join("forbidden").exists());
}

#[test]
fn compacted_existing_worker_requires_explicit_proxy_consent_in_review() {
    let fixture = Fixture::new();
    let mut binding = fixture.bound_storage_worker();
    let mut metadata = binding.worker_metadata().clone();
    metadata.requires_tool_rebinding = true;
    binding.update_worker_metadata(metadata).unwrap();
    drop(binding);
    let before = fixture.snapshot();
    let mut child = Pty::start(
        &fixture,
        &["project", "web", "--pick", "--work", "AST-open"],
    );
    child.wait_for(0, CONTEXT);
    child.go(b"\r", WORK);
    let review = child.go(b"\r", REVIEW);
    assert!(review.contains("Resume with --proxy"), "{review}");
    assert!(review.contains("Context: storage"), "{review}");
    child.send(b"\x03");
    let (status, output, stderr) = child.finish();
    assert!(status.success(), "{output}\n{stderr}");
    assert_eq!(fixture.snapshot(), before);
}
