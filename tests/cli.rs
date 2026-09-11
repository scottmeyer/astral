use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

struct FixtureProcess {
    child: Child,
    banner_reader: Option<JoinHandle<()>>,
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        // This is the exact child spawned by this test, never a PID discovered
        // from the environment or a running development proxy.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.banner_reader.take() {
            let _ = reader.join();
        }
    }
}

fn start_fixture_proxy(
    explicit_subcommand: bool,
    upstream: SocketAddr,
) -> (tempfile::TempDir, FixtureProcess, SocketAddr) {
    let directory = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_astral"));
    if explicit_subcommand {
        command.arg("proxy");
    }
    command
        .args([
            "--listen",
            "127.0.0.1:0",
            "--mode",
            "passthrough",
            "--upstream",
        ])
        .arg(format!("http://{upstream}/v1"))
        .arg("--state-dir")
        .arg(directory.path().join("proxy-state"))
        .env_remove("SSL_CERT_FILE")
        .env_remove("ASTRAL_UPSTREAM")
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for variable in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env_remove(variable);
    }
    let mut child = command.spawn().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stderr).read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });
    let mut process = FixtureProcess {
        child,
        banner_reader: Some(reader),
    };
    let banner = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("fixture startup banner timeout")
        .expect("fixture banner read failed");
    let prefix = format!("astral {} listening on ", env!("CARGO_PKG_VERSION"));
    assert!(
        banner.starts_with(&prefix),
        "unexpected fixture banner: {banner}"
    );
    let address: SocketAddr = banner
        .trim()
        .strip_prefix(&prefix)
        .unwrap()
        .parse()
        .unwrap();
    assert!(address.ip().is_loopback());
    assert_ne!(address.port(), 0);
    assert!(process.child.try_wait().unwrap().is_none());
    (directory, process, address)
}

fn health(address: SocketAddr) -> Value {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET /healthz HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.take(16_385).read_to_string(&mut response).unwrap();
    assert!(response.len() <= 16_384, "unbounded health response");
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .expect("HTTP health response");
    assert!(headers.starts_with("HTTP/1.1 200 "), "{headers}");
    serde_json::from_str(body).unwrap()
}

#[test]
fn renamed_binary_serves_proxy_subcommand_and_direct_proxy_options() {
    for explicit_subcommand in [true, false] {
        // The only configured upstream is another loopback fixture. A health
        // request must not contact it; no provider is reachable from this test.
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        upstream.set_nonblocking(true).unwrap();
        let (_directory, mut process, address) =
            start_fixture_proxy(explicit_subcommand, upstream.local_addr().unwrap());
        let receipt = health(address);
        assert_eq!(receipt["status"], "ok");
        assert_eq!(receipt["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(receipt["requests_received"], 0);
        assert_eq!(
            upstream.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(process.child.try_wait().unwrap().is_none());
        drop(process);
    }
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_astral"))
        .args(args)
        .env("NO_COLOR", "1")
        .env_remove("ASTRAL_UPSTREAM")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn help(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_err());
    String::from_utf8(output.stdout).unwrap()
}

fn pretty_json(bytes: &[u8]) -> Value {
    let value: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        bytes,
        format!("{}\n", serde_json::to_string_pretty(&value).unwrap()).as_bytes()
    );
    value
}

#[test]
fn unified_help_is_plain_and_describes_commands_and_default_context() {
    let global = help(&["--help"]);
    for command in ["proxy", "context", "project", "work", "init"] {
        assert!(global.contains(command), "missing {command}: {global}");
    }
    assert!(global.contains("astral"));
    assert!(global.contains("--json"));
    let project = help(&["project", "--help"]);
    assert!(project.contains("--non-interactive"));
    assert!(project.contains("--resume"));
    assert!(project.contains("codex exec resume"));
    assert!(project.contains("project-context"));
    assert!(project.contains("--proxy"));
    assert!(project.contains("NAME is omitted"));
    let proxy = help(&["proxy", "--help"]);
    assert!(proxy.contains("--listen"));
    assert!(proxy.contains("--upstream"));
    assert!(proxy.contains("ASTRAL_UPSTREAM"));
    assert!(proxy.contains(".astral-runtime"));
    assert_eq!(
        help(&["--version"]).trim(),
        format!("astral {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn json_help_and_version_are_explicit_pretty_envelopes() {
    for args in [
        vec!["--json", "--help"],
        vec!["--help", "--json"],
        vec!["--json", "project", "--help"],
        vec!["project", "--json", "--help"],
        vec!["project", "--help", "--json"],
        vec!["proxy", "--help", "--json"],
        vec!["--json", "--version"],
    ] {
        let output = run(&args);
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty());
        let value = pretty_json(&output.stdout);
        assert_eq!(value["status"], "help");
        let message = value["message"].as_str().unwrap();
        assert!(message.contains("astral"), "{args:?}: {message}");
        if args.contains(&"--version") {
            assert_eq!(
                message.trim(),
                format!("astral {}", env!("CARGO_PKG_VERSION"))
            );
        }
    }
}

#[test]
fn typo_error_keeps_clap_suggestion_and_usage_in_plain_stderr() {
    let output = run(&["proxy", "--upstrean", "http://127.0.0.1:1"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(serde_json::from_slice::<Value>(&output.stderr).is_err());
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains("--upstrean"), "{text}");
    assert!(text.contains("similar argument"), "{text}");
    assert!(text.contains("--upstream"), "{text}");
    assert!(text.contains("Usage:"), "{text}");
}

#[test]
fn json_usage_errors_preserve_machine_code_and_full_suggestion() {
    for args in [
        vec!["--json", "proxy", "--upstrean", "http://127.0.0.1:1"],
        vec!["proxy", "--json", "--upstrean", "http://127.0.0.1:1"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let value = pretty_json(&output.stderr);
        assert_eq!(value["error"]["code"], "CLI_USAGE");
        let message = value["error"]["message"].as_str().unwrap();
        assert!(message.contains("similar argument"), "{message}");
        assert!(message.contains("--upstream"), "{message}");
    }
}
