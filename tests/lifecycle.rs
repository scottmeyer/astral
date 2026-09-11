#![cfg(unix)]
use astral::{
    lifecycle::{self, Scope},
    project::Project,
    workspace::{WorkerMetadata, WorktreeBinding},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};
const WORK: &str = "AST-example";
const THREAD: &str = "01a10000-1234-7000-8000-000000000001";
// Each functional fixture spawns many Git probes under the real three-second
// callback budget. Avoid competing fixtures exhausting a small hosted runner;
// concurrency and deadline enforcement have their own explicit controls.
static CALLBACK_FIXTURE: Mutex<()> = Mutex::new(());
fn put(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn environment(command: &mut Command) -> &mut Command {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "maintenance.auto")
        .env("GIT_CONFIG_VALUE_1", "false")
        .env("GIT_CONFIG_KEY_2", "gc.auto")
        .env("GIT_CONFIG_VALUE_2", "0")
        .env("GIT_AUTHOR_NAME", "Lifecycle Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Lifecycle Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env_remove("ASTRAL_HOOK_DEPTH")
        .env_remove("ASTRAL_HOOK_CHILD")
        .env_remove("ASTRAL_WORKTREE_ROOT")
}
fn git(root: &Path, args: &[&str]) -> Output {
    let out = environment(Command::new("git").current_dir(root))
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    _serial: MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        let serial = CALLBACK_FIXTURE.lock().unwrap_or_else(|e| e.into_inner());
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("repo");
        fs::create_dir(&root).unwrap();
        put(
            &root,
            ".astral/project.toml",
            "schema_version=1\nid='fixture'\nname='Fixture'\ndescription='Lifecycle'\ncore='core'\nprojections='projections'\nwork_items='work/items.jsonl'\n[identity]\nscope='repository'\nruntime_bindings='private'\n[subsystems]\nweb='core/web'\n",
        );
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            put(
                &root,
                &format!(".astral/core/{name}"),
                "Original documentation.\n",
            );
        }
        put(
            &root,
            ".astral/core/web/subsystem.toml",
            "schema_version=1\nid='web'\npurpose='fixture'\nreadme='README.md'\nrules=[]\ndecisions=[]\nwork_items=[]\nprojection='seed'\ndepends_on=[]\n",
        );
        put(&root, ".astral/core/web/README.md", "Web context.\n");
        put(
            &root,
            ".astral/projections/seed/projection.toml",
            "schema_version=1\nid='seed'\nkind='fresh-context'\nsubsystems=['web']\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n",
        );
        put(&root, ".astral/projections/seed/handoff.md", "Seed.\n");
        put(
            &root,
            ".astral/work/items.jsonl",
            format!(
                "{}\n",
                json!({"schema_version":1,"id":WORK,"title":"Lifecycle","status":"open","acceptance":["Observe"],"depends_on":[]})
            ),
        );
        git(&root, &["init", "--initial-branch=main"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "fixture"]);
        Self {
            _temp: temp,
            root,
            _serial: serial,
        }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_astral"));
        environment(&mut cmd)
            .current_dir(&self.root)
            .env("ASTRAL_CODEX_BIN", self.root.join("must-not-execute"));
        cmd
    }
    fn bind(&self) -> WorktreeBinding {
        let mut binding = WorktreeBinding::acquire(&self.root, "fixture", "web", WORK).unwrap();
        let current = Project::load(binding.root())
            .unwrap()
            .launch_context("web", Some(WORK))
            .unwrap();
        binding
            .update_worker_metadata(WorkerMetadata {
                context_initialized: true,
                thread_id: Some(THREAD.into()),
                selection_digest: Some(current.current_context.selection_digest),
                selected_bundle_recorded: true,
                ..Default::default()
            })
            .unwrap();
        binding
    }
    fn install_git(&self) {
        use astral::hooks::install::{self, Action, Target};
        let executable = Path::new(env!("CARGO_BIN_EXE_astral"));
        let plan = install::plan(&self.root, executable, Target::Git, Action::Install).unwrap();
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        install::apply(
            &self.root,
            executable,
            Target::Git,
            Action::Install,
            &plan.plan_sha256,
        )
        .unwrap();
    }
    fn event(&self, name: &str, session: &str) -> Output {
        let event = json!({"hook_event_name":name,"source":"startup","session_id":session,"cwd":self.root,"model":"fixture","prompt":"SECRET-PROMPT","last_assistant_message":"SECRET-ANSWER","transcript_path":"/must/not/read"});
        let started = Instant::now();
        let mut child = self
            .command()
            .args(["hook", "codex"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(event.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        if String::from_utf8_lossy(&output.stdout).contains("observation unavailable") {
            let callback_elapsed = started.elapsed();
            // Only after failure, inspect the same helper without its outer
            // deadline. This distinguishes a slow observation from an error;
            // it cannot populate the notification cache before a passing test.
            let probe_started = Instant::now();
            let probe = self
                .command()
                .env("ASTRAL_HOOK_DEPTH", "1")
                .env("ASTRAL_HOOK_CHILD", "1")
                .args([
                    "hook-check",
                    "--scope",
                    "worktree",
                    "--audience",
                    "codex-model",
                    "--session-scope",
                    session,
                    "--claimed-session",
                    session,
                    "--force-notice",
                ])
                .output()
                .unwrap();
            panic!(
                "{name} callback unavailable after {callback_elapsed:?}; direct helper took {:?}, status {}; stdout: {}; stderr: {}",
                probe_started.elapsed(),
                probe.status,
                String::from_utf8_lossy(&probe.stdout)
                    .chars()
                    .take(1024)
                    .collect::<String>(),
                String::from_utf8_lossy(&probe.stderr)
                    .chars()
                    .take(1024)
                    .collect::<String>(),
            );
        }
        output
    }
}

#[test]
fn independent_snapshots_and_explicit_checks_do_not_mutate_state() {
    let f = Fixture::new();
    let index = fs::read(f.root.join(".git/index")).unwrap();
    let before = lifecycle::check(&f.root, Scope::Index, None).unwrap();
    assert!(before.diagnostics.is_empty(), "{:?}", before.diagnostics);
    assert_eq!(before.index.digest, before.committed.digest);
    assert_eq!(before.index.digest, before.worktree.digest);
    assert_eq!(index, fs::read(f.root.join(".git/index")).unwrap());
    assert!(!f.root.join(".git/astral-hooks").exists());
    put(&f.root, ".astral/core/web/README.md", "Changed.\n");
    let changed = lifecycle::check(&f.root, Scope::Worktree, None).unwrap();
    assert_eq!(before.index.digest, changed.index.digest);
    assert!(
        changed
            .diagnostics
            .contains(&"UNSTAGED_CONTEXT_DIFFERENCE".into())
    );
    git(&f.root, &["add", ".astral"]);
    let staged = lifecycle::check(&f.root, Scope::Index, None).unwrap();
    assert_eq!(staged.index.digest, staged.worktree.digest);
    assert!(
        staged
            .diagnostics
            .contains(&"UNCOMMITTED_CONTEXT_DIFFERENCE".into())
    );
    git(&f.root, &["commit", "-m", "context"]);
    assert!(
        lifecycle::check(&f.root, Scope::Head, None)
            .unwrap()
            .diagnostics
            .is_empty()
    );
}

#[test]
fn invalid_candidate_cannot_be_hidden_by_valid_working_manifest() {
    let f = Fixture::new();
    let manifest = f.root.join(".astral/project.toml");
    let original = fs::read(&manifest).unwrap();
    fs::write(&manifest, "invalid TOML").unwrap();
    git(&f.root, &["add", ".astral/project.toml"]);
    fs::write(&manifest, original).unwrap();
    let report = lifecycle::check(&f.root, Scope::Index, None).unwrap();
    assert_eq!(report.index.availability, "invalid");
    assert_eq!(report.worktree.availability, "valid");
    assert!(
        lifecycle::notice(&report, None)
            .contains("index project context is unavailable or invalid")
    );
    assert!(lifecycle::check(&f.root, Scope::Worktree, Some("../bad")).is_err());
}

#[test]
fn busy_worker_drift_is_observed_without_rebinding_or_acknowledgement() {
    let f = Fixture::new();
    let mut binding = f.bind();
    let mut metadata = binding.worker_metadata().clone();
    metadata.staging_in_progress = true;
    binding.update_worker_metadata(metadata.clone()).unwrap();
    put(
        binding.root(),
        ".astral/core/web/README.md",
        "Changed while worker owns checkout.\n",
    );
    let report = lifecycle::check(binding.root(), Scope::Worktree, None).unwrap();
    let worker = report.worker.as_ref().unwrap();
    assert_eq!(worker.ownership, "busy");
    assert_eq!(worker.changed_since_staging, Some(true));
    assert!(worker.pending_operation);
    assert!(
        !worker
            .diagnostics
            .contains(&"WORKER_RECOVERY_REVIEW_REQUIRED".into())
    );
    assert!(lifecycle::notice(&report, Some("other-session")).contains("identity does not match"));
    assert_eq!(
        serde_json::to_value(binding.worker_metadata()).unwrap(),
        serde_json::to_value(metadata).unwrap()
    );
    let root = binding.root().to_path_buf();
    drop(binding);
    let stopped = lifecycle::check(&root, Scope::Worktree, None).unwrap();
    assert!(
        stopped
            .worker
            .unwrap()
            .diagnostics
            .contains(&"WORKER_RECOVERY_REVIEW_REQUIRED".into())
    );
}

#[test]
fn manual_branch_switch_is_reported_without_adopting_session() {
    let f = Fixture::new();
    let binding = f.bind();
    let root = binding.root().to_path_buf();
    drop(binding);
    git(&root, &["checkout", "-b", "manual-change"]);
    let report = lifecycle::check(&root, Scope::Worktree, None).unwrap();
    assert!(!report.worker.unwrap().diagnostics.is_empty());
    let manifest = root.join(".astral/project.toml");
    let changed = fs::read_to_string(&manifest)
        .unwrap()
        .replace("id='fixture'", "id='different-project'");
    fs::write(&manifest, changed).unwrap();
    let mismatched = lifecycle::check(&root, Scope::Worktree, None).unwrap();
    assert!(!mismatched.worker.as_ref().unwrap().diagnostics.is_empty());
    assert!(lifecycle::notice(&mismatched, None).contains("needs review"));
}

#[test]
fn codex_events_coexist_and_do_not_leak_payloads_or_request_continuation() {
    let f = Fixture::new();
    let first = f.event("SessionStart", "session-one");
    assert!(first.status.success());
    let value: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert!(value["hookSpecificOutput"]["additionalContext"].is_string());
    assert!(!String::from_utf8_lossy(&first.stdout).contains("SECRET"));
    assert!(first.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&f.event("UserPromptSubmit", "session-one").stdout)
            .unwrap(),
        json!({})
    );
    // A new context boundary needs another advisory even if its digest matches.
    assert!(serde_json::from_slice::<Value>(&f.event("SessionStart", "session-one").stdout).unwrap()["hookSpecificOutput"].is_object());
    let stop: Value = serde_json::from_slice(&f.event("Stop", "session-one").stdout).unwrap();
    assert!(stop["systemMessage"].is_string());
    assert!(stop.get("decision").is_none());
    assert!(stop.get("hookSpecificOutput").is_none());
    assert!(
        serde_json::from_slice::<Value>(&f.event("UserPromptSubmit", "session-two").stdout)
            .unwrap()["hookSpecificOutput"]
            .is_object()
    );
    put(&f.root, ".astral/core/web/README.md", "new current state");
    assert!(
        serde_json::from_slice::<Value>(&f.event("UserPromptSubmit", "session-one").stdout)
            .unwrap()["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("differs")
    );
}

#[test]
fn callbacks_are_advisory_and_recursion_is_inert() {
    let f = Fixture::new();
    let unregistered = f
        .command()
        .args(["hook", "git", "pre-commit"])
        .output()
        .unwrap();
    assert!(unregistered.status.success());
    assert!(unregistered.stdout.is_empty());
    assert!(unregistered.stderr.is_empty());
    let recursive = f
        .command()
        .env("ASTRAL_HOOK_DEPTH", "1")
        .args(["hook", "git", "post-merge", "--manual", "ignored"])
        .output()
        .unwrap();
    assert!(recursive.status.success());
    assert!(recursive.stdout.is_empty());
    assert!(recursive.stderr.is_empty());
    let malformed = f
        .command()
        .args(["hook", "codex"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(malformed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&malformed.stdout).unwrap(),
        json!({})
    );
    let now = Instant::now();
    let mut stalled = f
        .command()
        .args(["hook", "codex"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let _held_stdin = stalled.stdin.take().unwrap();
    let output = stalled.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(now.elapsed() < Duration::from_secs(3));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({})
    );
    let failed_sink = f
        .command()
        .args(["hook", "git", "pre-commit", "--manual"])
        .stderr(Stdio::from(fs::File::open("/dev/null").unwrap()))
        .output()
        .unwrap();
    assert!(failed_sink.status.success());
}

#[test]
fn commit_forwards_literal_git_arguments_and_existing_hook_exit_status() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    put(&f.root, "message.txt", "literal message\n");
    let result = f
        .command()
        .args(["commit", "--", "--allow-empty", "-F", "message.txt"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(git(&f.root, &["log", "-1", "--format=%s"]).stdout)
            .unwrap()
            .trim(),
        "literal message"
    );
    put(
        &f.root,
        ".git/hooks/pre-commit",
        "#!/bin/sh\nprintf 'FOREIGN_HOOK\\n' >&2\nexit 1\n",
    );
    fs::set_permissions(
        f.root.join(".git/hooks/pre-commit"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let rejected = f
        .command()
        .args(["commit", "--", "--allow-empty", "-m", "rejected"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("FOREIGN_HOOK"));
    assert!(
        f.command()
            .args([
                "commit",
                "--",
                "--no-verify",
                "--allow-empty",
                "-m",
                "explicit skip"
            ])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn installed_hooks_read_the_commit_only_temporary_index() {
    let f = Fixture::new();
    f.install_git();
    let manifest = f.root.join(".astral/project.toml");
    let original = fs::read(&manifest).unwrap();
    fs::write(&manifest, "broken staged manifest").unwrap();
    git(&f.root, &["add", ".astral/project.toml"]);
    fs::write(&manifest, &original).unwrap();
    assert_eq!(
        lifecycle::check(&f.root, Scope::Index, None)
            .unwrap()
            .index
            .availability,
        "invalid"
    );
    put(
        &f.root,
        ".astral/core/web/README.md",
        "only this context change\n",
    );
    let output = git(
        &f.root,
        &[
            "commit",
            "--only",
            ".astral/core/web/README.md",
            "-m",
            "only selected path",
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Astral advisory:"), "{stderr}");
    assert!(!stderr.contains("observation unavailable"), "{stderr}");
    assert!(
        !stderr.contains("index project context is unavailable or invalid"),
        "{stderr}"
    );
    assert_eq!(
        git(&f.root, &["show", "HEAD:.astral/project.toml"]).stdout,
        original
    );
    assert_eq!(
        lifecycle::check(&f.root, Scope::Index, None)
            .unwrap()
            .index
            .availability,
        "invalid"
    );
}

#[test]
fn commit_all_warns_about_invalid_candidate_without_blocking_git() {
    let f = Fixture::new();
    f.install_git();
    put(&f.root, ".astral/project.toml", "invalid new candidate");
    let output = f
        .command()
        .args(["commit", "--", "-am", "explicitly commit invalid fixture"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("index project context is unavailable or invalid"),
        "{stderr}"
    );
    assert_eq!(
        git(&f.root, &["show", "HEAD:.astral/project.toml"]).stdout,
        b"invalid new candidate"
    );
}

#[test]
fn delayed_and_repeated_events_reobserve_current_state() {
    let f = Fixture::new();
    let first = f
        .command()
        .args([
            "hook",
            "git",
            "post-checkout",
            "--manual",
            "old-sha",
            "new-sha",
            "1",
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&first.stderr).contains("validated against current"));
    let repeated = f
        .command()
        .args(["hook", "git", "post-merge", "--manual", "0"])
        .output()
        .unwrap();
    assert!(repeated.stderr.is_empty());
    put(
        &f.root,
        ".astral/core/web/README.md",
        "changed after event\n",
    );
    let stale = f
        .command()
        .args([
            "hook",
            "git",
            "post-checkout",
            "--manual",
            "old-sha",
            "new-sha",
            "1",
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&stale.stderr).contains("differs"));
    assert!(stale.status.success());
    assert!(!f.root.join(".git/astral").exists());
}

#[test]
fn ordinary_checkout_and_fast_forward_merge_invoke_installed_observation() {
    let f = Fixture::new();
    f.install_git();
    let original = lifecycle::check(&f.root, Scope::Worktree, None)
        .unwrap()
        .worktree
        .digest;
    git(&f.root, &["checkout", "-b", "context-update"]);
    put(
        &f.root,
        ".astral/core/web/README.md",
        "context on another branch\n",
    );
    git(&f.root, &["commit", "-am", "update context"]);
    let checkout = git(&f.root, &["checkout", "main"]);
    assert!(String::from_utf8_lossy(&checkout.stderr).contains("Astral advisory:"));
    assert_eq!(
        lifecycle::check(&f.root, Scope::Worktree, None)
            .unwrap()
            .worktree
            .digest,
        original
    );
    let merged = git(&f.root, &["merge", "--ff-only", "context-update"]);
    assert!(String::from_utf8_lossy(&merged.stderr).contains("Astral advisory:"));
    let report = lifecycle::check(&f.root, Scope::Worktree, None).unwrap();
    assert_ne!(report.worktree.digest, original);
    assert!(report.diagnostics.is_empty());
    assert!(!f.root.join(".git/astral").exists());
}
