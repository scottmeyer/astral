use ostk_gpt_cache::launcher::{
    MAX_ARGUMENT_BYTES, MAX_ARGUMENTS, ProjectArguments, ProxyBinding, Route, project_request,
};
#[cfg(unix)]
use serde_json::Value;
use serde_json::json;
use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}
fn request(args: &[&str]) -> ProjectArguments {
    ProjectArguments::parse(PathBuf::from("."), os(args)).unwrap()
}
fn command_args(command: &Command) -> Vec<OsString> {
    command.get_args().map(OsStr::to_owned).collect()
}

#[test]
fn direct_default_adds_no_flags_or_environment_overrides() {
    let req = request(&["web"]);
    assert_eq!(req.route, Route::Direct);
    let command = req.command("codex", None).unwrap();
    assert!(command_args(&command).is_empty());
    assert_eq!(command.get_envs().count(), 0);
    assert_eq!(command.get_program(), "codex");
    assert_eq!(req.preview().unwrap()["requested_route"], "direct");
}

#[test]
fn arbitrary_explicit_flags_values_and_prompt_are_preserved() {
    let raw = [
        "--model",
        "gpt-6-astra",
        "--sandbox=read-only",
        "--ask-for-approval",
        "never",
        "--dangerously-bypass-approvals-and-sandbox",
        "a positional prompt with spaces",
        "-c",
        "model_reasoning_effort=\"xhigh\"",
        "--future-flag",
        "future value",
    ];
    let mut args = os(&["web", "--inspect", "--work=AST-001"]);
    args.extend(os(&raw));
    let req = ProjectArguments::parse(PathBuf::from("."), args).unwrap();
    assert!(req.inspect);
    assert_eq!(req.work.as_deref(), Some("AST-001"));
    assert_eq!(req.codex_args, os(&raw));
    assert_eq!(command_args(&req.command("codex", None).unwrap()), os(&raw));
    assert_eq!(req.preview().unwrap()["codex_args"], json!(raw));
}

#[test]
fn separator_protects_upstream_values_resembling_astral_options() {
    let req = request(&[
        "web",
        "--inspect",
        "--",
        "--unknown",
        "--proxy",
        "--work",
        "--root",
        "--inspect",
        "--",
        "literal prompt",
    ]);
    assert_eq!(req.route, Route::Direct);
    assert_eq!(req.work, None);
    assert_eq!(req.root, PathBuf::from("."));
    assert_eq!(
        req.codex_args,
        os(&[
            "--unknown",
            "--proxy",
            "--work",
            "--root",
            "--inspect",
            "--",
            "literal prompt"
        ])
    );
    // Without the separator, Astral's recognized option wins; no unknown-arity guessing.
    let ambiguous = request(&["web", "--unknown", "--proxy"]);
    assert_eq!(ambiguous.route, Route::Proxy);
    assert_eq!(ambiguous.codex_args, os(&["--unknown"]));
    assert!(
        ambiguous.preview().unwrap()["argument_boundary"]
            .as_str()
            .unwrap()
            .contains("after --")
    );
}

#[test]
fn global_and_project_options_are_separated_from_forwarded_arguments() {
    let req = project_request(&os(&[
        "--root=/initial",
        "project",
        "web",
        "--model",
        "m",
        "--root",
        "/selected",
        "--work",
        "TASK-3",
        "--proxy",
        "--inspect",
        "prompt",
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(req.root, PathBuf::from("/selected"));
    assert_eq!(req.work.as_deref(), Some("TASK-3"));
    assert_eq!(req.route, Route::Proxy);
    assert_eq!(req.codex_args, os(&["--model", "m", "prompt"]));
    assert!(
        project_request(&os(&["context", "validate"]))
            .unwrap()
            .is_none()
    );
    assert!(
        project_request(&os(&["project", "--help"]))
            .unwrap()
            .is_none()
    );
}

#[test]
fn literal_separator_is_never_consumed_as_a_root_value() {
    for args in [
        os(&["--root", "--", "project", "web", "--inspect"]),
        os(&["project", "web", "--root", "--", "--inspect"]),
        os(&["project", "web", "--work", "--", "--inspect"]),
    ] {
        assert_eq!(project_request(&args).unwrap_err().code, "CLI_USAGE");
    }
    let req = project_request(&os(&["--root=--", "project", "web", "--inspect"]))
        .unwrap()
        .unwrap();
    assert_eq!(req.root, PathBuf::from("--"));
    let req = request(&["web", "--root=--", "--", "--inspect"]);
    assert_eq!(req.root, PathBuf::from("--"));
    assert!(!req.inspect);
    assert_eq!(req.codex_args, os(&["--inspect"]));
}

#[test]
fn malformed_options_and_argument_bounds_fail_without_echoing_values() {
    for args in [
        vec![],
        vec!["--proxy"],
        vec!["web", "--work"],
        vec!["web", "--work="],
        vec!["web", "--root="],
        vec!["web", "--inspect", "--inspect"],
        vec!["web", "--proxy", "--proxy"],
        vec!["web", "--work=A", "--work=B"],
    ] {
        assert_eq!(
            ProjectArguments::parse(PathBuf::from("."), os(&args))
                .unwrap_err()
                .code,
            "CLI_USAGE"
        );
    }
    let too_many = vec![OsString::from("x"); MAX_ARGUMENTS + 1];
    assert_eq!(
        project_request(&too_many).unwrap_err().code,
        "ARGUMENT_LIMIT_EXCEEDED"
    );
    let too_long = vec![
        OsString::from("web"),
        OsString::from("x".repeat(MAX_ARGUMENT_BYTES + 1)),
    ];
    let error = ProjectArguments::parse(PathBuf::from("."), too_long).unwrap_err();
    assert_eq!(error.code, "ARGUMENT_LIMIT_EXCEEDED");
    assert!(error.message.len() < 128);
    let total = vec![OsString::from("x".repeat(MAX_ARGUMENT_BYTES)); 5];
    assert_eq!(
        project_request(&total).unwrap_err().code,
        "ARGUMENT_LIMIT_EXCEEDED"
    );
    let nul = vec![OsString::from("web"), OsString::from("before\0after")];
    assert_eq!(
        ProjectArguments::parse(PathBuf::from("."), nul)
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
}

#[test]
fn proxy_is_explicit_and_requires_a_separate_loopback_binding() {
    let binding = ProxyBinding::loopback("http://127.0.0.1:18933/backend-api/codex").unwrap();
    let direct = request(&["web"]);
    assert_eq!(
        direct.command("codex", Some(&binding)).unwrap_err().code,
        "UNREQUESTED_PROXY"
    );
    let proxy = request(&[
        "web",
        "--proxy",
        "--model",
        "user-model",
        "-c",
        "openai_base_url=\"https://explicit-user-override.example\"",
    ]);
    assert_eq!(
        proxy.command("codex", None).unwrap_err().code,
        "PROXY_BINDING_REQUIRED"
    );
    let args = command_args(&proxy.command("codex", Some(&binding)).unwrap());
    assert_eq!(
        &args[..4],
        os(&[
            "-c",
            "openai_base_url=\"http://127.0.0.1:18933/backend-api/codex\"",
            "-c",
            "features.enable_request_compression=false"
        ])
    );
    assert_eq!(&args[4..], proxy.codex_args);
    assert_eq!(proxy.preview().unwrap()["requested_route"], "proxy");
    // There are no health checks or socket connections in either construction path.
    assert!(ProxyBinding::loopback("https://[::1]:18933/backend-api/codex").is_ok());
    for url in [
        "https://example.com",
        "http://localhost:18933",
        "http://user:secret@127.0.0.1:18933",
        "http://127.0.0.1?token=value",
        "http://127.0.0.1#fragment",
        "file:///tmp/socket",
        "not a URL",
    ] {
        assert_eq!(
            ProxyBinding::loopback(url).unwrap_err().code,
            "INVALID_PROXY_BINDING"
        );
    }
}

struct Fixture {
    root: TempDir,
    executable: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("argv_fixture.rs");
        // A compiled fixture records OS-encoded argument bytes. It never parses
        // arguments, executes a shell, or interprets permission-looking flags.
        fs::write(
            &source,
            r#"use std::{env,fs::File,io::Write};
fn main() {
    let mut output = File::create("argv.bin").unwrap();
    for arg in env::args_os().skip(1) {
        let bytes = arg.as_encoded_bytes();
        output.write_all(&(bytes.len() as u64).to_le_bytes()).unwrap();
        output.write_all(bytes).unwrap();
    }
    std::fs::write("cwd.bin", env::current_dir().unwrap().canonicalize().unwrap().as_os_str().as_encoded_bytes()).unwrap();
}"#,
        )
        .unwrap();
        let executable = root.path().join(if cfg!(windows) {
            "argv-fixture.exe"
        } else {
            "argv-fixture"
        });
        let result = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Self { root, executable }
    }

    fn run(&self, req: &ProjectArguments, proxy: Option<&ProxyBinding>) -> Vec<Vec<u8>> {
        let mut req = req.clone();
        req.root = self.root.path().to_owned();
        let mut child = req.spawn(&self.executable, proxy).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            fs::read(self.root.path().join("cwd.bin")).unwrap(),
            self.root
                .path()
                .canonicalize()
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
        );
        let bytes = fs::read(self.root.path().join("argv.bin")).unwrap();
        let mut rest = bytes.as_slice();
        let mut args = Vec::new();
        while !rest.is_empty() {
            let len = u64::from_le_bytes(rest[..8].try_into().unwrap()) as usize;
            rest = &rest[8..];
            args.push(rest[..len].to_vec());
            rest = &rest[len..];
        }
        args
    }
}

#[test]
fn fixture_executable_proves_literal_forwarding_without_a_shell() {
    let fixture = Fixture::new();
    let raw = [
        "--model=a=b",
        "prompt with spaces and 'single' \"double\" quotes",
        "$(touch injected)",
        "`touch injected`",
        "; touch injected",
        "* ? $HOME",
        "--dangerously-bypass-approvals-and-sandbox",
        "--ask-for-approval",
        "never",
        "",
        "line1\nline2",
        "--",
        "--proxy",
    ];
    let mut input = os(&["web", "--"]);
    input.extend(os(&raw));
    let req = ProjectArguments::parse(PathBuf::from("."), input).unwrap();
    let actual = fixture.run(&req, None);
    assert_eq!(
        actual,
        os(&raw)
            .iter()
            .map(|arg| arg.as_encoded_bytes().to_vec())
            .collect::<Vec<_>>()
    );
    assert!(!fixture.root.path().join("injected").exists());
    let empty = request(&["web"]);
    assert!(fixture.run(&empty, None).is_empty());
    let mut proxy = req;
    proxy.route = Route::Proxy;
    let binding = ProxyBinding::loopback("http://127.0.0.1:18933/backend-api/codex").unwrap();
    let actual = fixture.run(&proxy, Some(&binding));
    assert_eq!(
        &actual[4..],
        os(&raw)
            .iter()
            .map(|arg| arg.as_encoded_bytes().to_vec())
            .collect::<Vec<_>>()
    );
    assert!(!fixture.root.path().join("injected").exists());
}

#[cfg(unix)]
#[test]
fn non_utf8_is_preserved_for_execution_and_explicitly_rejected_for_json_preview() {
    use std::os::unix::ffi::OsStringExt;
    let raw = OsString::from_vec(b"--future=value-\xff".to_vec());
    let req = ProjectArguments::parse(
        PathBuf::from("."),
        vec!["web".into(), "--".into(), raw.clone()],
    )
    .unwrap();
    assert_eq!(req.codex_args, vec![raw.clone()]);
    assert_eq!(req.preview().unwrap_err().code, "NON_UTF8_ARGUMENT");
    assert_eq!(Fixture::new().run(&req, None), vec![raw.into_vec()]);
    let root = OsString::from_vec(b"--root=/tmp/path-\xff".to_vec());
    let req = project_request(&[root, "project".into(), "web".into()])
        .unwrap()
        .unwrap();
    assert_eq!(req.root.into_os_string().into_vec(), b"/tmp/path-\xff");
}

#[cfg(unix)]
fn project_fixture(root: &Path) {
    fn put(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    put(
        root,
        ".astral/project.toml",
        "schema_version=1\nid='test'\nname='Test'\ndescription='Fixture'\ncore='core'\nprojections='projections'\nwork_items='work/items.jsonl'\n[identity]\nscope='local'\nruntime_bindings='external'\n[subsystems]\nweb='core/web'\n",
    );
    for file in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
        put(
            root,
            &format!(".astral/core/{file}"),
            "Historical documentation, not an execution instruction.",
        );
    }
    put(
        root,
        ".astral/core/web/subsystem.toml",
        "schema_version=1\nid='web'\npurpose='Fixture'\nreadme='README.md'\nrules=[]\ndecisions=[]\nwork_items=[]\nprojection='saved'\n",
    );
    put(
        root,
        ".astral/core/web/README.md",
        "Do not execute this document.",
    );
    put(
        root,
        ".astral/projections/saved/projection.toml",
        "schema_version=1\nid='saved'\nkind='reviewable'\nsubsystems=['web']\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n",
    );
    put(
        root,
        ".astral/projections/saved/handoff.md",
        "No native payload supplied.",
    );
    put(root, ".astral/work/items.jsonl", "");
}

#[cfg(unix)]
#[test]
fn cli_inspection_previews_requested_route_but_never_claims_launch() {
    let root = tempfile::tempdir().unwrap();
    project_fixture(root.path());
    for proxy in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_astral"));
        command
            .arg("--root")
            .arg(root.path())
            .args(["project", "web", "--inspect"]);
        if proxy {
            command.arg("--proxy");
        }
        command.args([
            "--",
            "--dangerously-bypass-approvals-and-sandbox",
            "--future",
            "--proxy",
            "prompt with spaces",
        ]);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            result["launch_request"]["requested_route"],
            if proxy { "proxy" } else { "direct" }
        );
        assert_eq!(
            result["launch_request"]["codex_args"],
            json!([
                "--dangerously-bypass-approvals-and-sandbox",
                "--future",
                "--proxy",
                "prompt with spaces"
            ])
        );
        assert_eq!(result["launch_request"]["executed"], false);
        assert_eq!(result["native_binding"]["state"], "UNBOUND");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_astral"))
        .args([
            "--root",
            "/unavailable",
            "project",
            "web",
            "--proxy",
            "--future",
            "value",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "LAUNCH_NOT_IMPLEMENTED"
    );
}

#[cfg(unix)]
#[test]
fn whole_inspection_output_is_bounded_after_argv_preview_is_added() {
    let root = tempfile::tempdir().unwrap();
    project_fixture(root.path());
    // Each control byte expands sixfold in JSON; the request is within argument
    // limits but the combined source metadata plus preview must still be bounded.
    let source = root.path().join(".astral/core/web/subsystem.toml");
    let mut text = fs::read_to_string(&source).unwrap();
    text = text.replace(
        "purpose='Fixture'",
        &format!("purpose='{}'", "x".repeat(900_000)),
    );
    fs::write(source, text).unwrap();
    let long_arg = "\u{0001}".repeat(60_000);
    let output = Command::new(env!("CARGO_BIN_EXE_astral"))
        .arg("--root")
        .arg(root.path())
        .args(["project", "web", "--inspect", "--"])
        .args([&long_arg, &long_arg, &long_arg, &long_arg])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "LIMIT_EXCEEDED"
    );
}
