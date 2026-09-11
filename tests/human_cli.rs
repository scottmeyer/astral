#![cfg(unix)]

use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

const DOCUMENT_MARKER: &str = "PRIVATE_DOCUMENT_BODY_NOT_CLI_OUTPUT";

fn put(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn fixture() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    put(
        root.path(),
        ".astral/project.toml",
        "schema_version=1\nid='display-fixture'\nname='Display fixture'\ndescription='Offline CLI output fixture'\ncore='core'\nprojections='projections'\nwork_items='work/items.jsonl'\n[identity]\nscope='local'\nruntime_bindings='external'\n[subsystems]\nproject-context='core/project-context'\n",
    );
    for file in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
        put(
            root.path(),
            &format!(".astral/core/{file}"),
            DOCUMENT_MARKER,
        );
    }
    put(
        root.path(),
        ".astral/core/project-context/subsystem.toml",
        "schema_version=1\nid='project-context'\npurpose='Fixture'\nreadme='README.md'\nrules=[]\ndecisions=[]\nwork_items=[]\nprojection='saved'\n",
    );
    put(
        root.path(),
        ".astral/core/project-context/README.md",
        DOCUMENT_MARKER,
    );
    put(
        root.path(),
        ".astral/projections/saved/projection.toml",
        "schema_version=1\nid='saved'\nkind='fresh-context'\nsubsystems=['project-context']\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n",
    );
    put(
        root.path(),
        ".astral/projections/saved/handoff.md",
        DOCUMENT_MARKER,
    );
    put(root.path(), ".astral/work/items.jsonl", "");
    root
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_astral"))
        .current_dir(root)
        .args(args)
        .env("ASTRAL_CODEX_BIN", root.join("must-not-launch-codex"))
        .env("NO_COLOR", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("ASTRAL_UPSTREAM")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn success(output: Output) -> String {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn structured(text: &str) -> Value {
    let value: Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        text,
        format!("{}\n", serde_json::to_string_pretty(&value).unwrap())
    );
    value
}

#[test]
fn context_reports_are_actionable_and_machine_output_is_explicit() {
    let root = fixture();
    let listed = success(run(root.path(), &["context", "list"]));
    assert!(listed.contains("subsystem:project-context"), "{listed}");
    assert!(listed.contains("astral project"), "{listed}");
    assert!(serde_json::from_str::<Value>(&listed).is_err());
    let checked = success(run(root.path(), &["context", "validate"]));
    assert!(
        checked.contains("Context valid: display-fixture"),
        "{checked}"
    );
    assert!(checked.contains("astral context list"), "{checked}");
    for args in [
        vec!["--json", "context", "list"],
        vec!["context", "--json", "list"],
        vec!["context", "list", "--json"],
    ] {
        let text = success(run(root.path(), &args));
        let value = structured(&text);
        assert_eq!(value["project_id"], "display-fixture");
        assert!(value["contexts"].as_array().unwrap().len() >= 2);
        assert!(!text.contains(DOCUMENT_MARKER));
    }
    assert!(!root.path().join(".git").exists());
}

#[test]
fn project_json_selection_preserves_literal_codex_json() {
    let root = fixture();
    let human = success(run(
        root.path(),
        &["project", "--inspect", "--", "--json", "Review this task"],
    ));
    assert!(human.contains("Inspection only"), "{human}");
    assert!(
        human.contains("Explicit Codex arguments: --json"),
        "{human}"
    );
    assert!(serde_json::from_str::<Value>(&human).is_err());
    assert!(!human.contains(DOCUMENT_MARKER));
    for args in [
        vec![
            "--json",
            "project",
            "--inspect",
            "--",
            "--json",
            "Review this task",
        ],
        vec![
            "project",
            "--json",
            "--inspect",
            "--",
            "--json",
            "Review this task",
        ],
        vec![
            "project",
            "--inspect",
            "--json",
            "--",
            "--json",
            "Review this task",
        ],
    ] {
        let text = success(run(root.path(), &args));
        let value = structured(&text);
        assert_eq!(value["selection"]["id"], "project-context");
        assert_eq!(value["launch_request"]["executed"], false);
        assert_eq!(
            value["launch_request"]["codex_args"],
            json!(["--json", "Review this task"])
        );
        assert!(!text.contains(DOCUMENT_MARKER));
    }
    assert!(!root.path().join(".git").exists());
}

#[test]
fn init_preview_json_selection_never_consumes_child_json_or_launches_codex() {
    let root = tempfile::tempdir().unwrap();
    let human = success(run(root.path(), &["init", "--inspect", "--", "--json"]));
    assert!(human.contains("Project initialization preview"), "{human}");
    assert!(human.contains("--json"), "{human}");
    assert!(serde_json::from_str::<Value>(&human).is_err());
    for args in [
        vec!["--json", "init", "--inspect", "--", "--json"],
        vec!["init", "--inspect", "--json", "--", "--json"],
    ] {
        let value = structured(&success(run(root.path(), &args)));
        assert_eq!(value["operation"], "initialize");
        assert_eq!(value["launch_request"]["codex_args"], json!(["--json"]));
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn project_errors_are_human_or_explicit_json_without_stdout() {
    let root = fixture();
    let output = run(root.path(), &["project", "missing-context", "--inspect"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("Error ["), "{error}");
    assert!(error.contains("astral context list"), "{error}");
    assert!(serde_json::from_str::<Value>(&error).is_err());
    let output = run(
        root.path(),
        &["project", "missing-context", "--inspect", "--json"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let value = structured(&String::from_utf8(output.stderr).unwrap());
    assert!(value["error"]["code"].as_str().is_some());
    assert!(value["error"]["message"].as_str().is_some());
}

#[test]
fn work_id_is_plain_by_default_and_json_at_either_global_flag_position() {
    let root = fixture();
    let records = root.path().join(".astral/work/items.jsonl");
    let before = fs::read(&records).unwrap();
    let id = success(run(root.path(), &["work", "id"]));
    assert!(id.starts_with("AST-"));
    assert_eq!(id.trim().len(), 16);
    assert_eq!(id.lines().count(), 1);
    for args in [
        vec!["--json", "work", "id"],
        vec!["work", "--json", "id"],
        vec!["work", "id", "--json"],
    ] {
        let value = structured(&success(run(root.path(), &args)));
        assert!(value["id"].as_str().unwrap().starts_with("AST-"));
    }
    assert_eq!(fs::read(&records).unwrap(), before);
}

#[test]
fn follow_up_commands_preserve_a_repository_selected_from_another_directory() {
    let root = fixture();
    let outside = tempfile::tempdir().unwrap();
    let path = root.path().to_str().unwrap();
    let listed = success(run(outside.path(), &["--root", path, "context", "list"]));
    assert!(
        listed.contains(&format!("astral --root {path} project")),
        "{listed}"
    );
    let error = run(
        outside.path(),
        &["--root", path, "project", "missing", "--inspect"],
    );
    assert_eq!(error.status.code(), Some(2));
    let diagnostic = String::from_utf8(error.stderr).unwrap();
    assert!(diagnostic.contains(path), "{diagnostic}");
    let initialized = Command::new("git")
        .current_dir(root.path())
        .args(["-c", "init.templateDir=", "init", "-q"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(initialized.status.success(), "{initialized:?}");
    let hooks = success(run(outside.path(), &["hooks", "status", "--root", path]));
    assert!(
        hooks.contains(&format!("astral --root {path} hooks install git")),
        "{hooks}"
    );
    let structured = structured(&success(run(
        outside.path(),
        &["--root", path, "context", "list", "--json"],
    )));
    assert_eq!(structured["project_id"], "display-fixture");
    assert!(
        structured.get("root").is_none(),
        "human formatting must not alter the machine schema"
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}
