//! Local JSON-lines host adapter. Profiles are operator-owned, never model-generated.
use astral::working::{Profile, Runtime};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Read, Write},
    path::PathBuf,
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    workspace: PathBuf,
    #[arg(long)]
    state_dir: PathBuf,
    #[arg(long)]
    profile: PathBuf,
    #[arg(long)]
    raw_observations: bool,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let profile: Profile = serde_json::from_slice(&std::fs::read(args.profile)?)?;
    let mut runtime = Runtime::open(
        &args.workspace,
        &args.state_dir,
        profile,
        !args.raw_observations,
    )?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut input = stdin.lock();
    loop {
        let mut line = Vec::new();
        (&mut input)
            .take(9 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if line.is_empty() {
            break;
        }
        // End the stream on an oversized frame; never treat its suffix as another request.
        anyhow::ensure!(line.len() <= 9 * 1024 * 1024, "host request too large");
        let result = match serde_json::from_slice::<Value>(&line) {
            Ok(v) => runtime.dispatch(&v).await,
            Err(e) => Err(e.into()),
        };
        let response = match result {
            Ok(value) => json!({"ok":true,"result":value}),
            Err(error) => json!({"ok":false,"error":error.to_string()}),
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}
