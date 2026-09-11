use clap::{Parser, Subcommand, error::ErrorKind};
use ostk_gpt_cache::project::{Error, Project};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "astral",
    version,
    about = "Inspect Git-native Astral project context; no runtime launch"
)]
struct Cli {
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    Project {
        name: String,
        #[arg(long)]
        inspect: bool,
        #[arg(long)]
        work: Option<String>,
    },
}

#[derive(Subcommand)]
enum ContextCommand {
    Validate,
    List,
}

fn run(cli: Cli) -> Result<Value, Error> {
    if matches!(cli.command, Command::Project { inspect: false, .. }) {
        return Err(Error { code: "LAUNCH_NOT_IMPLEMENTED", message: "Native launch is not implemented; use project NAME --inspect for read-only context inspection".into() });
    }
    let project = Project::load(cli.root)?;
    match cli.command {
        Command::Context {
            command: ContextCommand::Validate,
        } => project.validate(),
        Command::Context {
            command: ContextCommand::List,
        } => project.list(),
        Command::Project { name, work, .. } => project.inspect(&name, work.as_deref()),
    }
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                println!("{}", json!({"status": "help", "message": e.to_string()}));
                return;
            }
            eprintln!(
                "{}",
                json!({"error": {"code": "CLI_USAGE", "message": e.to_string().chars().take(1024).collect::<String>()}})
            );
            std::process::exit(2);
        }
    };
    match run(cli) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("{}", json!({"error": error}));
            std::process::exit(2);
        }
    }
}
