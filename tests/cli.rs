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
        .env_remove("OSTK_GPT_UPSTREAM")
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
    assert!(!banner.contains("ostk-gpt-cache"));
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

fn help(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_astral"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let output: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output["status"], "help");
    output["message"].as_str().unwrap().to_owned()
}

#[test]
fn unified_help_describes_commands_and_default_project_context() {
    let global = help(&["--help"]);
    for command in ["proxy", "context", "project", "work", "init"] {
        assert!(global.contains(command), "missing {command}: {global}");
    }
    assert!(global.contains("astral"));
    assert!(!global.contains("ostk-gpt-cache"));
    let project = help(&["project", "--help"]);
    assert!(project.contains("project-context"));
    assert!(project.contains("--proxy"));
    assert!(project.contains("NAME is omitted"));
    let proxy = help(&["proxy", "--help"]);
    assert!(proxy.contains("--listen"));
    assert!(proxy.contains("--upstream"));
    assert_eq!(
        help(&["--version"]).trim(),
        format!("astral {}", env!("CARGO_PKG_VERSION"))
    );
}
