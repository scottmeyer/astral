#![cfg(unix)]
use astral::{
    lifecycle::{self, Scope},
    project::{
        Limits, Project,
        freshness::{self, State},
    },
};
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const MANIFEST: &str = ".astral/core/web/subsystem.toml";
const CODE: &str = "src/web.rs";
fn put(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn git(root: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(name);
        }
    }
    let out = cmd
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_CONFIG_KEY_1", "maintenance.auto")
        .env("GIT_CONFIG_VALUE_1", "false")
        .env("GIT_CONFIG_KEY_2", "gc.auto")
        .env("GIT_CONFIG_VALUE_2", "0")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn fixture(inputs: Option<&[&str]>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        ".astral/project.toml",
        "schema_version=1\nid='fixture'\nname='Fixture'\ndescription='Freshness'\ncore='core'\nprojections='projections'\nwork_items='work/items.jsonl'\n[identity]\nscope='repository'\nruntime_bindings='private'\n[subsystems]\nweb='core/web'\n",
    );
    for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
        put(
            root,
            &format!(".astral/core/{name}"),
            "Historical verification only.\n",
        );
    }
    let mut subsystem = "# Preserve this comment\nschema_version=1\nid='web'\npurpose='fixture'\nreadme='README.md'\nrules=[]\ndecisions=[]\nwork_items=[]\nprojection='seed'\ndepends_on=[]\n".to_owned();
    if let Some(inputs) = inputs {
        subsystem += &format!(
            "\n[freshness]\ninputs={}\n",
            serde_json::to_string(inputs).unwrap()
        );
    }
    put(root, MANIFEST, subsystem);
    put(root, ".astral/core/web/README.md", "Web knowledge.\n");
    put(
        root,
        ".astral/projections/seed/projection.toml",
        "schema_version=1\nid='seed'\nkind='fresh-context'\nsubsystems=['web']\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n",
    );
    put(
        root,
        ".astral/projections/seed/handoff.md",
        "Historical context.\n",
    );
    put(root, ".astral/work/items.jsonl", "");
    put(root, CODE, "SECRET_CODE_BODY_not_prompt_context\n");
    dir
}
fn observation(root: &Path) -> freshness::Observation {
    Project::load(root)
        .unwrap()
        .freshness()
        .next()
        .unwrap()
        .clone()
}
fn acknowledge(root: &Path) -> Value {
    let preview = freshness::review(root, "web", None).unwrap();
    freshness::review(root, "web", preview["plan_sha256"].as_str()).unwrap()
}

#[test]
fn explicit_review_preserves_formatting_and_excludes_code_from_context() {
    let dir = fixture(Some(&[CODE]));
    let root = dir.path();
    let before = fs::read(root.join(MANIFEST)).unwrap();
    let preview = freshness::review(root, "web", None).unwrap();
    assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), before);
    assert_eq!(observation(root).state, State::Unreviewed);
    assert!(acknowledge(root)["applied"].as_bool().unwrap());
    assert_eq!(observation(root).state, State::Unchanged);
    assert!(
        fs::read_to_string(root.join(MANIFEST))
            .unwrap()
            .starts_with("# Preserve this comment\nschema_version=1")
    );
    let project = Project::load(root).unwrap();
    let context = project.fresh_context("web", None).unwrap();
    assert!(
        !serde_json::to_string(&context)
            .unwrap()
            .contains("SECRET_CODE_BODY")
    );
    assert!(!context.sources.iter().any(|s| s.path == CODE));
    assert_eq!(context.freshness[0].state, State::Unchanged);
    let previous = context.selection_digest;
    put(root, "unrelated.txt", "not declared");
    assert_eq!(observation(root).state, State::Unchanged);
    put(root, CODE, "changed implementation");
    let current = Project::load(root)
        .unwrap()
        .fresh_context("web", None)
        .unwrap();
    assert_eq!(current.freshness[0].state, State::NeedsReview);
    assert_ne!(previous, current.selection_digest);
    assert_eq!(
        freshness::review(root, "web", preview["plan_sha256"].as_str())
            .unwrap_err()
            .code,
        "FRESHNESS_REVIEW_STALE"
    );
}

#[test]
fn code_and_document_edits_invalidate_review_previews() {
    for path in [CODE, ".astral/core/web/README.md", ".astral/core/TEST.md"] {
        let dir = fixture(Some(&[CODE]));
        let root = dir.path();
        acknowledge(root);
        let preview = freshness::review(root, "web", None).unwrap();
        let baseline = fs::read(root.join(MANIFEST)).unwrap();
        put(root, path, "changed after preview");
        assert_eq!(observation(root).state, State::NeedsReview);
        assert_eq!(
            freshness::review(root, "web", preview["plan_sha256"].as_str())
                .unwrap_err()
                .code,
            "FRESHNESS_REVIEW_STALE"
        );
        assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), baseline);
    }
}

#[test]
fn missing_symlink_directory_and_oversize_inputs_are_unavailable() {
    let dir = fixture(Some(&["missing.rs"]));
    let root = dir.path();
    assert_eq!(observation(root).state, State::Unavailable);
    assert_eq!(
        freshness::review(root, "web", None).unwrap_err().code,
        "FRESHNESS_UNAVAILABLE"
    );
    std::os::unix::fs::symlink(CODE, root.join("missing.rs")).unwrap();
    assert_eq!(
        observation(root).inputs[0].error_code.as_deref(),
        Some("SYMLINK_REJECTED")
    );
    let dir = fixture(Some(&["src"]));
    assert_eq!(
        observation(dir.path()).inputs[0].error_code.as_deref(),
        Some("INVALID_FILE_TYPE")
    );
    let dir = fixture(Some(&[CODE]));
    put(dir.path(), CODE, vec![0; Limits::default().file_bytes + 1]);
    assert_eq!(
        observation(dir.path()).inputs[0].error_code.as_deref(),
        Some("LIMIT_EXCEEDED")
    );
}

#[test]
fn byte_file_and_input_budgets_cannot_be_bypassed() {
    let dir = fixture(Some(&[CODE]));
    let root = dir.path();
    let project = Project::load(root).unwrap();
    let documents: usize = project.observed_sources().map(|s| s.bytes).sum();
    for limits in [
        Limits {
            total_bytes: documents,
            ..Default::default()
        },
        Limits {
            files: project.observed_sources().count(),
            ..Default::default()
        },
    ] {
        let project = Project::load_with_limits(root, limits).unwrap();
        assert_eq!(
            project.freshness().next().unwrap().state,
            State::Unavailable
        );
    }
    let many: Vec<_> = (0..65).map(|i| format!("file-{i}")).collect();
    let names: Vec<_> = many.iter().map(String::as_str).collect();
    let dir = fixture(Some(&names));
    assert_eq!(
        Project::load(dir.path()).err().unwrap().code,
        "FRESHNESS_INPUT_LIMIT"
    );
}

#[test]
fn legacy_projects_remain_opt_out_and_unsafe_declarations_fail() {
    let dir = fixture(None);
    let project = Project::load(dir.path()).unwrap();
    assert_eq!(project.freshness().count(), 0);
    assert!(
        serde_json::to_value(project.fresh_context("web", None).unwrap())
            .unwrap()
            .get("freshness")
            .is_none()
    );
    assert_eq!(
        freshness::review(dir.path(), "web", None).unwrap_err().code,
        "FRESHNESS_NOT_CONFIGURED"
    );
    for path in [
        "../outside",
        "/absolute",
        ".git/config",
        ".astral/core/TEST.md",
        "src/.git/config",
    ] {
        let dir = fixture(Some(&[path]));
        assert!(Project::load(dir.path()).is_err(), "{path}");
    }
}

#[test]
fn lifecycle_keeps_committed_staged_and_working_code_distinct() {
    let dir = fixture(Some(&[CODE]));
    let root = dir.path();
    acknowledge(root);
    git(root, &["init", "--initial-branch=main"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "baseline"]);
    put(root, CODE, "staged implementation");
    git(root, &["add", CODE]);
    put(root, CODE, "working implementation");
    acknowledge(root);
    let report = lifecycle::check(root, Scope::Worktree, None).unwrap();
    assert_eq!(report.committed.freshness[0].state, State::Unchanged);
    assert_eq!(report.index.freshness[0].state, State::NeedsReview);
    assert_eq!(report.worktree.freshness[0].state, State::Unchanged);
    assert_ne!(
        report.index.freshness[0].inputs[0].sha256,
        report.worktree.freshness[0].inputs[0].sha256
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|c| c == "INDEX_FRESHNESS_NEEDS_REVIEW")
    );
    // Staging only the review must not bless a different staged implementation.
    git(root, &["add", MANIFEST]);
    assert_eq!(
        lifecycle::check(root, Scope::Index, None)
            .unwrap()
            .index
            .freshness[0]
            .state,
        State::NeedsReview
    );
    git(root, &["add", CODE]);
    let report = lifecycle::check(root, Scope::Index, None).unwrap();
    assert_eq!(report.index.freshness[0].state, State::Unchanged);
    // Remove only from the fixture index: the working file still exists.
    git(root, &["update-index", "--force-remove", CODE]);
    let report = lifecycle::check(root, Scope::Index, None).unwrap();
    assert_eq!(report.index.freshness[0].state, State::Unavailable);
    assert_eq!(report.worktree.freshness[0].state, State::Unchanged);
}

#[test]
fn exact_git_paths_do_not_expand_globs_or_read_unlisted_files() {
    let dir = fixture(Some(&["src/file[1].rs"]));
    let root = dir.path();
    put(root, "src/file[1].rs", "literal");
    put(root, "src/file1.rs", "other");
    acknowledge(root);
    git(root, &["init", "--initial-branch=main"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "literal"]);
    let report = lifecycle::check(root, Scope::Head, None).unwrap();
    assert_eq!(report.index.freshness[0].state, State::Unchanged);
    assert_eq!(report.committed.freshness[0].state, State::Unchanged);
}

#[test]
fn cli_reports_human_advice_and_machine_metadata() {
    let dir = fixture(Some(&[CODE]));
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_astral"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let human = run(&["context", "freshness", "web"]);
    assert!(human.status.success());
    assert!(String::from_utf8_lossy(&human.stdout).contains("no review baseline"));
    let json = run(&["context", "review", "web", "--json"]);
    assert_eq!(observation(dir.path()).state, State::Unreviewed);
    let preview: Value = serde_json::from_slice(&json.stdout).unwrap();
    let applied = run(&[
        "context",
        "review",
        "web",
        "--apply",
        preview["plan_sha256"].as_str().unwrap(),
    ]);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(String::from_utf8_lossy(&applied.stdout).contains("Context review recorded"));
    put(dir.path(), CODE, "new bytes");
    assert!(
        run(&["context", "review", "web", "--dry-run"])
            .status
            .success()
    );
    assert_eq!(observation(dir.path()).state, State::NeedsReview);
    assert!(
        !run(&["context", "review", "web", "--yes", "--dry-run"])
            .status
            .success()
    );
    assert!(
        run(&["context", "review", "web", "--yes", "--json"])
            .status
            .success()
    );
    assert_eq!(observation(dir.path()).state, State::Unchanged);
}

#[test]
fn dependency_inputs_and_knowledge_participate_without_review_cascades() {
    let dir = fixture(Some(&[CODE]));
    let root = dir.path();
    let project = fs::read_to_string(root.join(".astral/project.toml")).unwrap();
    put(
        root,
        ".astral/project.toml",
        format!("{project}auth='core/auth'\n"),
    );
    let web = fs::read_to_string(root.join(MANIFEST)).unwrap();
    put(
        root,
        MANIFEST,
        web.replace("depends_on=[]", "depends_on=['auth']"),
    );
    put(
        root,
        ".astral/core/auth/subsystem.toml",
        "schema_version=1\nid='auth'\npurpose='auth'\nreadme='README.md'\nrules=[]\ndecisions=[]\nwork_items=[]\nprojection='seed'\n[freshness]\ninputs=['src/auth.rs']\n",
    );
    put(root, ".astral/core/auth/README.md", "Auth knowledge");
    put(root, "src/auth.rs", "Auth implementation");
    acknowledge(root);
    let preview = freshness::review(root, "auth", None).unwrap();
    freshness::review(root, "auth", preview["plan_sha256"].as_str()).unwrap();
    let web = || {
        Project::load(root)
            .unwrap()
            .inspect_freshness("web")
            .unwrap()["freshness"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["subsystem"] == "web")
            .unwrap()
            .clone()
    };
    assert_eq!(web()["state"], "unchanged");
    put(root, "src/auth.rs", "Changed authentication");
    assert_eq!(web()["state"], "needs_review");
    acknowledge(root);
    put(
        root,
        ".astral/core/auth/README.md",
        "Changed auth knowledge",
    );
    assert_eq!(web()["state"], "needs_review");
}

#[test]
fn review_refuses_hardlinked_manifests_and_respects_cooperating_writer_lock() {
    use fs2::FileExt;
    let dir = fixture(Some(&[CODE]));
    let root = dir.path();
    let preview = freshness::review(root, "web", None).unwrap();
    let lock = fs::File::open(root.join(".astral")).unwrap();
    lock.try_lock_exclusive().unwrap();
    assert_eq!(
        freshness::review(root, "web", preview["plan_sha256"].as_str())
            .unwrap_err()
            .code,
        "FRESHNESS_WRITE_FAILED"
    );
    FileExt::unlock(&lock).unwrap();
    fs::hard_link(root.join(MANIFEST), root.join("manifest-copy.toml")).unwrap();
    assert_eq!(
        freshness::review(root, "web", preview["plan_sha256"].as_str())
            .unwrap_err()
            .code,
        "FRESHNESS_WRITE_FAILED"
    );
    assert_eq!(observation(root).state, State::Unreviewed);
}

#[test]
fn unsafe_index_mode_and_large_unrelated_trees_never_become_fresh() {
    let dir = fixture(Some(&[CODE]));
    let root = dir.path();
    acknowledge(root);
    git(root, &["init", "--initial-branch=main"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "baseline"]);
    // A symlink in the index may have perfectly ordinary working bytes.
    let object = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD:src/web.rs"])
        .output()
        .unwrap();
    let oid = String::from_utf8(object.stdout).unwrap();
    git(
        root,
        &[
            "update-index",
            "--cacheinfo",
            &format!("120000,{},{}", oid.trim(), CODE),
        ],
    );
    for i in 0..2100 {
        put(
            root,
            &format!("unrelated/{i}"),
            "outside freshness discovery",
        );
    }
    let report = lifecycle::check(root, Scope::Index, None).unwrap();
    assert_eq!(report.index.freshness[0].state, State::Unavailable);
    assert_eq!(
        report.index.freshness[0].inputs[0].error_code.as_deref(),
        Some("SNAPSHOT_UNSAFE_MODE")
    );
    assert_eq!(report.worktree.freshness[0].state, State::Unchanged);
    assert!(!lifecycle::notice(&report, None).contains("src/web.rs"));
}
