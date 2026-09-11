use clap::{Parser, Subcommand, error::ErrorKind};
use ostk_gpt_cache::config::Config;
use ostk_gpt_cache::launcher::{DEFAULT_CONTEXT, ProjectArguments, project_request};
use ostk_gpt_cache::project::{Error, Project};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "astral",
    version,
    about = "Responses proxy and Git-native project context",
    after_help = "Run astral proxy for the proxy server. With no arguments, or direct proxy options such as --listen, astral also starts the proxy. Project inspection does not start a proxy or Codex."
)]
struct Cli {
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the Responses proxy.
    Proxy {
        #[command(flatten)]
        config: Box<Config>,
    },
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    #[command(
        after_help = "The context defaults to project-context when NAME is omitted. An explicit NAME must come immediately after project. Astral recognizes --root, --work, --inspect and --proxy before the first literal --. Put all Codex arguments after -- if any flag value resembles an Astral option. Direct Codex is the requested default; --proxy explicitly requests local proxy routing. Native launch is not implemented yet."
    )]
    Project {
        #[arg(default_value = DEFAULT_CONTEXT)]
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

async fn run(cli: Cli) -> Result<Option<Value>, Error> {
    if matches!(cli.command, Command::Project { inspect: false, .. }) {
        return Err(Error { code: "LAUNCH_NOT_IMPLEMENTED", message: "Native launch is not implemented; use project NAME --inspect for read-only context inspection".into() });
    }
    match cli.command {
        Command::Proxy { config } => run_proxy(*config).await,
        Command::Context {
            command: ContextCommand::Validate,
        } => Project::load(&cli.root)?.validate().map(Some),
        Command::Context {
            command: ContextCommand::List,
        } => Project::load(&cli.root)?.list().map(Some),
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
        })
        .map(Some),
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

async fn run_proxy(config: Config) -> Result<Option<Value>, Error> {
    ostk_gpt_cache::server::run(config)
        .await
        .map(|()| None)
        .map_err(|error| Error {
            code: "PROXY_FAILED",
            message: error.to_string(),
        })
}

fn finish(result: Result<Option<Value>, Error>) {
    match result {
        Ok(Some(output)) => println!("{output}"),
        Ok(None) => {}
        Err(error) => {
            eprintln!("{}", json!({"error": error}));
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match project_request(&args) {
        Ok(Some(request)) => {
            finish(inspect_project(request).map(Some));
            return;
        }
        Err(error) => {
            finish(Err(error));
            return;
        }
        Ok(None) => {}
    }
    // Preserve the former proxy CLI's direct option form under its new name.
    let direct_proxy = args.is_empty()
        || args.first().is_some_and(|arg| {
            arg.as_encoded_bytes().starts_with(b"-")
                && !["--help", "-h", "--version", "-V", "--root", "--"]
                    .iter()
                    .any(|reserved| arg == reserved)
                && !arg.as_encoded_bytes().starts_with(b"--root=")
        });
    let cli = if direct_proxy {
        Config::try_parse().map(|config| Cli {
            root: PathBuf::from("."),
            command: Command::Proxy {
                config: Box::new(config),
            },
        })
    } else {
        Cli::try_parse()
    };
    let cli = match cli {
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
    finish(run(cli).await);
}
