use clap::{Parser, Subcommand, error::ErrorKind};
use ostk_gpt_cache::config::Config;
use ostk_gpt_cache::launcher::{
    DEFAULT_CONTEXT, ProjectArguments, initialization_request, project_request,
};
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
    /// Ask Codex to build a best-effort .astral index in a repository without one.
    #[command(
        after_help = "Pass Codex options after --. The initializer supplies its own prompt. --non-interactive uses codex exec; otherwise Codex opens interactively. Generated files are validated after Codex exits."
    )]
    Init {
        #[arg(long)]
        non_interactive: bool,
        #[arg(long)]
        inspect: bool,
    },
    /// Run the Responses proxy.
    Proxy {
        #[command(flatten)]
        config: Box<Config>,
    },
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    /// Utilities for Git-native work items.
    Work {
        #[command(subcommand)]
        command: WorkCommand,
    },
    #[command(
        after_help = "The context defaults to project-context when NAME is omitted. An explicit NAME must come immediately after project. Astral recognizes --root, --work, --inspect and --proxy before the first literal --. Put all Codex arguments after -- if any flag value resembles an Astral option. Fresh document contexts launch directly in Codex. Native restoration and managed --proxy launch remain pending."
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

#[derive(Subcommand)]
enum WorkCommand {
    /// Generate a short random ID; printing does not reserve or create a work item.
    Id,
}

async fn run(cli: Cli) -> Result<Option<Value>, Error> {
    match cli.command {
        Command::Init {
            non_interactive,
            inspect,
        } => {
            run_init(
                ProjectArguments {
                    root: cli.root,
                    name: DEFAULT_CONTEXT.into(),
                    work: None,
                    inspect,
                    route: ostk_gpt_cache::launcher::Route::Direct,
                    codex_args: Vec::new(),
                },
                non_interactive,
            )
            .await
        }
        Command::Proxy { config } => run_proxy(*config).await,
        Command::Context {
            command: ContextCommand::Validate,
        } => Project::load(&cli.root)?.validate().map(Some),
        Command::Context {
            command: ContextCommand::List,
        } => Project::load(&cli.root)?.list().map(Some),
        Command::Work {
            command: WorkCommand::Id,
        } => {
            let project = Project::load(&cli.root)?;
            let id =
                ostk_gpt_cache::work::generate_id(project.work_ids()).map_err(|error| Error {
                    code: error.code(),
                    message: error.to_string(),
                })?;
            println!("{id}");
            Ok(None)
        }
        Command::Project {
            name,
            work,
            proxy,
            inspect,
        } => {
            run_project(ProjectArguments {
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
            .await
        }
    }
}

fn inspect_project(request: ProjectArguments) -> Result<Value, Error> {
    let preview = request.preview()?;
    let project = Project::load(&request.root)?;
    let mut output = project.inspect(&request.name, request.work.as_deref())?;
    output["launch_request"] = preview;
    Ok(output)
}

async fn run_project(request: ProjectArguments) -> Result<Option<Value>, Error> {
    if request.inspect {
        return inspect_project(request).map(Some);
    }
    let code = ostk_gpt_cache::launch::project(request).await?;
    std::process::exit(code);
}

async fn run_init(
    request: ProjectArguments,
    non_interactive: bool,
) -> Result<Option<Value>, Error> {
    if request.inspect {
        return Ok(Some(
            json!({"mode": "inspect", "operation": "initialize", "prompt_version": ostk_gpt_cache::launch::INIT_PROMPT_VERSION, "prompt": ostk_gpt_cache::launch::INIT_PROMPT, "non_interactive": non_interactive, "launch_request": request.preview()?}),
        ));
    }
    let code =
        ostk_gpt_cache::launch::initialize(&request.root, &request.codex_args, non_interactive)
            .await?;
    std::process::exit(code);
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

fn render_output(output: Value) -> Result<String, Error> {
    let text = if output.get("mode").and_then(Value::as_str) == Some("inspect") {
        serde_json::to_string_pretty(&output)
    } else {
        serde_json::to_string(&output)
    }
    .expect("JSON Value serializes");
    // Bound the actual emitted bytes, including indentation and the final newline.
    if text.len().saturating_add(1) > ostk_gpt_cache::project::Limits::default().output_bytes {
        return Err(Error {
            code: "LIMIT_EXCEEDED",
            message: "output byte limit".into(),
        });
    }
    Ok(text)
}

fn finish(result: Result<Option<Value>, Error>) {
    let result = result.and_then(|output| output.map(render_output).transpose());
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
    match initialization_request(&args) {
        Ok(Some((request, non_interactive))) => {
            finish(run_init(request, non_interactive).await);
            return;
        }
        Err(error) => {
            finish(Err(error));
            return;
        }
        Ok(None) => {}
    }
    match project_request(&args) {
        Ok(Some(request)) => {
            finish(run_project(request).await);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_is_included_in_the_output_budget() {
        let output = json!({"mode": "inspect", "rows": vec![0; 400_000]});
        assert!(
            serde_json::to_vec(&output).unwrap().len()
                < ostk_gpt_cache::project::Limits::default().output_bytes
        );
        assert_eq!(render_output(output).unwrap_err().code, "LIMIT_EXCEEDED");
    }
}
