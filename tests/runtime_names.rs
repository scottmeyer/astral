use astral::config::Config;
use clap::Parser;
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn proxy_default_state_is_separate_from_project_context() {
    let config = Config::try_parse_from(["astral", "--upstream", "http://127.0.0.1:1"]).unwrap();
    assert_eq!(config.state_dir, PathBuf::from(".astral-runtime"));
}

#[tokio::test]
async fn upstream_environment_is_parsed_before_runtime_or_network_start() {
    let root = tempfile::tempdir().unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_astral"));
    command
        .current_dir(root.path())
        .args(["proxy", "--json", "--listen", "127.0.0.1:0"])
        .env("ASTRAL_UPSTREAM", "file:///invalid-upstream")
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(5), command.output())
        .await
        .expect("invalid upstream environment must fail before starting the proxy")
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("upstream must be HTTP(S)"));
    assert!(!root.path().join(".astral-runtime").exists());
    assert!(!root.path().join(".astral").exists());
}
