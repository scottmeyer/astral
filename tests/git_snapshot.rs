#![cfg(unix)]
use ostk_gpt_cache::{
    git_snapshot::{GitSnapshot, MAX_ENTRIES, SnapshotKind},
    project::{Limits, Project},
};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn put(root: &Path, path: &str, body: impl AsRef<[u8]>) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}
fn command(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_AUTHOR_NAME", "Snapshot Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Snapshot Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid");
    cmd
}
fn git(root: &Path, args: &[&str]) -> String {
    let out = command(root).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim_end().into()
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new(committed: bool, sha256: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("repo");
        fs::create_dir(&root).unwrap();
        let mut args = vec!["init", "--initial-branch=main"];
        if sha256 {
            args.push("--object-format=sha256");
        }
        git(&root, &args);
        put(
            &root,
            ".astral/project.toml",
            r#"schema_version=1
id="fixture"
name="Fixture"
description="Snapshot fixture"
core="core"
projections="projections"
work_items="work/items.jsonl"
[identity]
scope="repository"
runtime_bindings="private"
[subsystems]
web="core/web"
"#,
        );
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            put(&root, &format!(".astral/core/{name}"), "Fixture docs.\n");
        }
        put(
            &root,
            ".astral/core/web/subsystem.toml",
            r#"schema_version=1
id="web"
purpose="Fixture"
readme="README.md"
rules=[]
decisions=[]
work_items=[]
projection="seed"
depends_on=[]
"#,
        );
        put(&root, ".astral/core/web/README.md", "original\n");
        put(&root, ".astral/work/items.jsonl", "");
        put(
            &root,
            ".astral/projections/seed/projection.toml",
            r#"schema_version=1
id="seed"
kind="fresh-context"
subsystems=["web"]
handoff="handoff.md"
native_payload_in_repository=false
sources=[]
"#,
        );
        put(
            &root,
            ".astral/projections/seed/handoff.md",
            "Seed handoff.\n",
        );
        git(&root, &["add", ".astral"]);
        if committed {
            git(&root, &["commit", "-m", "fixture"]);
        }
        Self { _temp: temp, root }
    }
    fn capture(&self, kind: SnapshotKind) -> GitSnapshot {
        GitSnapshot::capture(&self.root, kind).unwrap()
    }
}
fn readme(project: Project) -> String {
    project
        .fresh_context("web", None)
        .unwrap()
        .documents
        .into_iter()
        .find(|d| d.source.path == ".astral/core/web/README.md")
        .unwrap()
        .text
}
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in fs::read_dir(at).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                walk(root, &e.path(), out);
            } else {
                out.insert(
                    e.path().strip_prefix(root).unwrap().into(),
                    fs::read(e.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn exact_staged_head_and_working_versions_are_distinct_without_mutation() {
    let f = Fixture::new(true, false);
    put(&f.root, ".astral/core/web/README.md", "staged\n");
    git(&f.root, &["add", ".astral/core/web/README.md"]);
    put(&f.root, ".astral/core/web/README.md", "working\n");
    let before = files(&f.root);
    let index = f.capture(SnapshotKind::Index);
    let head = f.capture(SnapshotKind::Head);
    assert_eq!(readme(index.load_project().unwrap()), "staged\n");
    assert_eq!(readme(head.load_project().unwrap()), "original\n");
    assert_eq!(readme(Project::load(&f.root).unwrap()), "working\n");
    assert!(index.has_project());
    assert_eq!(index.branch(), Some("main"));
    assert_eq!(index.root(), f.root);
    assert_ne!(index.digest(), head.digest());
    index.verify_unchanged().unwrap();
    assert_eq!(before, files(&f.root));
}
#[test]
fn valid_working_manifest_does_not_hide_broken_staged_manifest() {
    let f = Fixture::new(true, false);
    put(&f.root, ".astral/project.toml", "broken=[");
    git(&f.root, &["add", ".astral/project.toml"]);
    let original = git(&f.root, &["show", "HEAD:.astral/project.toml"]);
    put(&f.root, ".astral/project.toml", original);
    assert!(Project::load(&f.root).is_ok());
    assert!(f.capture(SnapshotKind::Index).load_project().is_err());
}
#[test]
fn missing_untracked_and_intent_to_add_references_never_use_working_files() {
    for intent in [false, true] {
        let f = Fixture::new(true, false);
        let p = f.root.join(".astral/core/web/subsystem.toml");
        let text = fs::read_to_string(&p)
            .unwrap()
            .replace("README.md", "NEW.md");
        fs::write(p, text).unwrap();
        put(&f.root, ".astral/core/web/NEW.md", "working-only\n");
        git(&f.root, &["add", ".astral/core/web/subsystem.toml"]);
        if intent {
            git(&f.root, &["add", "-N", ".astral/core/web/NEW.md"]);
        }
        assert!(Project::load(&f.root).is_ok());
        let snapshot = f.capture(SnapshotKind::Index);
        assert!(!snapshot.contains(".astral/core/web/NEW.md"));
        assert_eq!(
            snapshot.load_project().err().unwrap().code,
            "PATH_UNAVAILABLE"
        );
    }
}
#[test]
fn unborn_sha256_and_detached_head_are_explicit() {
    for sha256 in [false, true] {
        let f = Fixture::new(false, sha256);
        let index = f.capture(SnapshotKind::Index);
        assert!(index.head().is_none());
        assert_eq!(index.branch(), Some("main"));
        assert!(index.load_project().is_ok());
        let head = f.capture(SnapshotKind::Head);
        assert!(!head.has_project());
        assert!(head.load_project().is_err());
        git(&f.root, &["commit", "-m", "first"]);
        assert!(index.verify_unchanged().is_err());
        git(&f.root, &["checkout", "--detach"]);
        let head = f.capture(SnapshotKind::Head);
        assert!(head.branch().is_none());
        assert_eq!(head.head().unwrap().len(), if sha256 { 64 } else { 40 });
        assert!(head.load_project().is_ok());
    }
}
#[test]
fn index_changes_outside_context_invalidate_evidence_without_changing_context_digest() {
    let f = Fixture::new(true, false);
    let old = f.capture(SnapshotKind::Index);
    put(&f.root, "code.txt", "new code");
    git(&f.root, &["add", "code.txt"]);
    assert_eq!(old.verify_unchanged().unwrap_err().code, "SNAPSHOT_CHANGED");
    assert_eq!(old.digest(), f.capture(SnapshotKind::Index).digest());
}

#[test]
fn context_child() {
    let Some(case) = std::env::var_os("ASTRAL_SNAPSHOT_TEST_CASE") else {
        return;
    };
    let root = std::env::current_dir().unwrap();
    let snapshot = GitSnapshot::capture(&root, SnapshotKind::Index);
    if case == "redirect" {
        assert_eq!(snapshot.err().unwrap().code, "SNAPSHOT_CONTEXT_MISMATCH");
    } else {
        assert_eq!(
            readme(snapshot.unwrap().load_project().unwrap()),
            "alternate\n"
        );
    }
}
#[test]
fn alternate_and_temporary_commit_indexes_are_honored_and_redirects_reject() {
    let f = Fixture::new(true, false);
    let alternate = f._temp.path().join("candidate-index");
    fs::copy(f.root.join(".git/index"), &alternate).unwrap();
    put(&f.root, ".astral/core/web/README.md", "alternate\n");
    let out = command(&f.root)
        .env("GIT_INDEX_FILE", &alternate)
        .args(["add", ".astral/core/web/README.md"])
        .output()
        .unwrap();
    assert!(out.status.success());
    put(&f.root, ".astral/core/web/README.md", "working\n");
    let before = fs::read(&alternate).unwrap();
    let run = |case: &str, dir: Option<&Path>| {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.current_dir(&f.root)
            .args(["--exact", "context_child", "--nocapture"])
            .env("ASTRAL_SNAPSHOT_TEST_CASE", case)
            .env("GIT_INDEX_FILE", &alternate)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
        if let Some(dir) = dir {
            cmd.env("GIT_DIR", dir);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run("alternate", None);
    assert_eq!(before, fs::read(&alternate).unwrap());
    assert_eq!(
        readme(f.capture(SnapshotKind::Index).load_project().unwrap()),
        "original\n"
    );
    let other = Fixture::new(true, false);
    run("redirect", Some(&other.root.join(".git")));
}
#[test]
fn unsafe_modes_and_unmerged_context_are_rejected() {
    let f = Fixture::new(true, false);
    let oid = git(&f.root, &["rev-parse", "HEAD:.astral/core/web/README.md"]);
    git(
        &f.root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{oid},.astral/link"),
        ],
    );
    assert_eq!(
        GitSnapshot::capture(&f.root, SnapshotKind::Index)
            .err()
            .unwrap()
            .code,
        "SNAPSHOT_UNSAFE_MODE"
    );
    let f = Fixture::new(true, false);
    let oid = git(&f.root, &["rev-parse", "HEAD:.astral/core/web/README.md"]);
    let mut child = command(&f.root)
        .args(["update-index", "--index-info"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    write!(
        child.stdin.take().unwrap(),
        "0 {}\t.astral/core/web/README.md\n100644 {oid} 1\t.astral/core/web/README.md\n",
        "0".repeat(40)
    )
    .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        GitSnapshot::capture(&f.root, SnapshotKind::Index)
            .err()
            .unwrap()
            .code,
        "SNAPSHOT_UNMERGED"
    );
}
#[test]
fn lazy_loading_ignores_superseded_blob_bytes_but_preserves_project_budgets() {
    let f = Fixture::new(true, false);
    put(
        &f.root,
        ".astral/projections/seed/bundles/unused/window.json",
        vec![b'x'; 9 * 1024 * 1024],
    );
    git(&f.root, &["add", ".astral"]);
    let snapshot = f.capture(SnapshotKind::Index);
    assert!(snapshot.load_project().is_ok());
    let limits = Limits {
        file_bytes: 10,
        ..Limits::default()
    };
    assert_eq!(
        snapshot
            .load_project_with_limits(limits)
            .err()
            .unwrap()
            .code,
        "LIMIT_EXCEEDED"
    );
}
#[test]
fn metadata_entry_limit_is_explicit() {
    let f = Fixture::new(true, false);
    let oid = git(&f.root, &["rev-parse", "HEAD:.astral/core/web/README.md"]);
    let mut child = command(&f.root)
        .args(["update-index", "--index-info"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut input = child.stdin.take().unwrap();
        for n in 0..MAX_ENTRIES {
            writeln!(input, "100644 {oid}\t.astral/extra/{n}").unwrap();
        }
    }
    assert!(child.wait().unwrap().success());
    let code = GitSnapshot::capture(&f.root, SnapshotKind::Index)
        .err()
        .unwrap()
        .code;
    assert_eq!(code, "SNAPSHOT_LIMIT");
}

#[test]
fn native_raw_bytes_and_number_spellings_survive_lazy_git_loading() {
    let f = Fixture::new(true, false);
    let payload=br#" [ {"type":"compaction","encrypted_content":"OPAQUE_SENTINEL","extension":123456789012345678901234567890,"fraction":1.2300e+02} ] "#;
    let manifest=serde_json::to_vec(&serde_json::json!({"schema_version":1,"format":"astral-codex-native",
        "payload":{"file":"window.json","sha256":ostk_gpt_cache::hash(payload),"bytes":payload.len(),"item_count":1},
        "compatibility":{"runtime":"codex","runtime_version":"0.154.0","protocol":"openai-responses-lite","provider":"openai","model":"gpt-6-astra","requires_tool_rebinding":true,"identity_scope":"same-account"},
        "source":{"project_id":"fixture","revision":null,"dirty":false,"selection_sha256":"a".repeat(64),"history_sha256":"b".repeat(64)},
        "capture":{"boundary":"completed-turn","history_complete":true,"last_checkpoint_index":0},"parents":[]})).unwrap();
    put(
        &f.root,
        ".astral/projections/saved/bundles/one/manifest.json",
        &manifest,
    );
    put(
        &f.root,
        ".astral/projections/saved/bundles/one/window.json",
        payload,
    );
    put(
        &f.root,
        ".astral/projections/saved/handoff.md",
        "Native fixture.\n",
    );
    put(
        &f.root,
        ".astral/projections/saved/projection.toml",
        format!(
            "schema_version=1\nid=\"saved\"\nkind=\"native-checkpoint\"\nsubsystems=[\"web\"]\nhandoff=\"handoff.md\"\nnative_payload_in_repository=true\nsources=[]\n[native_bundle]\nmanifest=\"bundles/one/manifest.json\"\nsha256={:?}\n",
            ostk_gpt_cache::hash(&manifest)
        ),
    );
    git(&f.root, &["add", ".astral"]);
    let snapshot = f.capture(SnapshotKind::Index);
    let project = snapshot.load_project().unwrap();
    let native = project
        .launch_context("projection:saved", None)
        .unwrap()
        .native
        .unwrap();
    assert_eq!(native.payload_bytes(), payload);
    assert_eq!(native.manifest_bytes(), manifest);
    assert!(!format!("{snapshot:?}").contains("OPAQUE_SENTINEL"));
    assert_eq!(
        project
            .observed_sources()
            .find(|s| s.path.ends_with("window.json"))
            .unwrap()
            .sha256,
        ostk_gpt_cache::hash(payload)
    );
}

#[test]
fn split_index_is_rejected_without_freshening_shared_file() {
    let f = Fixture::new(true, false);
    git(&f.root, &["update-index", "--split-index"]);
    let shared = git(&f.root, &["rev-parse", "--shared-index-path"]);
    let shared = if Path::new(&shared).is_absolute() {
        PathBuf::from(shared)
    } else {
        f.root.join(shared)
    };
    let before = files(&f.root);
    let stamp = fs::metadata(&shared).unwrap().modified().unwrap();
    assert_eq!(
        GitSnapshot::capture(&f.root, SnapshotKind::Index)
            .err()
            .unwrap()
            .code,
        "SNAPSHOT_SPLIT_INDEX"
    );
    assert_eq!(stamp, fs::metadata(&shared).unwrap().modified().unwrap());
    assert_eq!(before, files(&f.root));
}

#[test]
fn filter_configuration_does_not_execute_scripts_or_change_bytes() {
    let f = Fixture::new(true, false);
    put(&f.root, ".gitattributes", ".astral/** filter=forbidden\n");
    git(&f.root, &["add", ".gitattributes"]);
    git(
        &f.root,
        &["config", "filter.forbidden.clean", "touch SHOULD_NOT_RUN"],
    );
    git(
        &f.root,
        &["config", "filter.forbidden.smudge", "touch SHOULD_NOT_RUN"],
    );
    let before = files(&f.root);
    let snapshot = f.capture(SnapshotKind::Index);
    assert!(snapshot.load_project().is_ok());
    snapshot.verify_unchanged().unwrap();
    assert!(!f.root.join("SHOULD_NOT_RUN").exists());
    assert_eq!(before, files(&f.root));
}

#[test]
fn version_four_index_is_supported() {
    let f = Fixture::new(true, false);
    git(&f.root, &["update-index", "--index-version=4"]);
    let snapshot = f.capture(SnapshotKind::Index);
    assert_eq!(readme(snapshot.load_project().unwrap()), "original\n");
}
