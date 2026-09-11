use clap::{Parser, Subcommand, error::ErrorKind};
use ostk_gpt_cache::launcher::{ProjectArguments, project_request};
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
    #[command(
        after_help = "Astral recognizes --root, --work, --inspect and --proxy before the first literal --. Put all Codex arguments after -- if any flag value resembles an Astral option. Direct Codex is the requested default; --proxy explicitly requests local proxy routing. Native launch is not implemented yet."
    )]
    Project {
        name: String,
        #[arg(long)]
        inspect: bool,
        #[arg(long)]
        work: Option<String>,
        #[arg(long)]
        proxy: bool,
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
    match cli.command {
        Command::Context {
            command: ContextCommand::Validate,
        } => Project::load(&cli.root)?.validate(),
        Command::Context {
            command: ContextCommand::List,
        } => Project::load(&cli.root)?.list(),
        Command::Project {
            name,
            work,
            proxy,
            inspect,
        } => inspect_project(ProjectArguments {
            root: cli.root,
            name,
            work,
            inspect,
            route: if proxy {
                ostk_gpt_cache::launcher::Route::Proxy
            } else {
                ostk_gpt_cache::launcher::Route::Direct
            },
            codex_args: Vec::new(),
        }),
    }
}

fn inspect_project(request: ProjectArguments) -> Result<Value, Error> {
    if !request.inspect {
        return Err(Error { code: "LAUNCH_NOT_IMPLEMENTED", message: "Native import/staging and launch are not implemented; use --inspect to review context and requested Codex arguments".into() });
    }
    let preview = request.preview()?;
    let project = Project::load(&request.root)?;
    let mut output = project.inspect(&request.name, request.work.as_deref())?;
    output["launch_request"] = preview;
    if serde_json::to_vec(&output).expect("JSON serializes").len()
        > ostk_gpt_cache::project::Limits::default().output_bytes
    {
        return Err(Error {
            code: "LIMIT_EXCEEDED",
            message: "output byte limit".into(),
        });
    }
    Ok(output)
}

fn finish(result: Result<Value, Error>) {
    match result {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("{}", json!({"error": error}));
            std::process::exit(2);
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match project_request(&args) {
        Ok(Some(request)) => {
            finish(inspect_project(request));
            return;
        }
        Err(error) => {
            finish(Err(error));
            return;
        }
        Ok(None) => {}
    }
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
    finish(run(cli));
}
