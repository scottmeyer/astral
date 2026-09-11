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
    /// Inspect recorded workers and selected context without launching a runtime.
    Status {
        #[arg(long)]
        work: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = ostk_gpt_cache::status::DEFAULT_PAGE_SIZE)]
        limit: usize,
    },
    /// Diagnose local resume blockers; exit 1 when the inspected page needs attention.
    Doctor {
        #[arg(long)]
        work: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = ostk_gpt_cache::status::DEFAULT_PAGE_SIZE)]
        limit: usize,
    },
    /// Compact and explicitly export a stopped bound worker for Git handoff.
    Save {
        name: String,
        #[arg(long)]
        work: String,
        #[arg(long, default_value = DEFAULT_CONTEXT)]
        context: String,
        #[arg(long)]
        proxy: bool,
        #[arg(last = true, allow_hyphen_values = true)]
        codex_args: Vec<std::ffi::OsString>,
    },
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
        after_help = "The context defaults to project-context when NAME is omitted. An explicit NAME must come immediately after project. Astral recognizes --root, --work, --inspect, --proxy, --non-interactive, and --resume before the first literal --. Put all Codex arguments after -- if any flag value resembles an Astral option. --non-interactive runs codex exec resume; use exec-supported Codex options, including -c sandbox_mode and -c approval_policy. --resume names a private Astral launch receipt."
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
        #[arg(long)]
        non_interactive: bool,
        #[arg(long)]
        resume: Option<String>,
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
    /// List records and their exact-line digests for optimistic updates.
    List,
    /// Create an open work item with a merge-safe random ID.
    Create {
        title: String,
        #[arg(long = "acceptance", required = true)]
        acceptance: Vec<String>,
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
    },
    /// Update status only if the observed record digest still matches.
    Update {
        id: String,
        #[arg(long, value_parser = ["open", "in_progress", "blocked", "complete"])]
        status: String,
        #[arg(long)]
        expected: String,
    },
    /// Three-way merge by ID. Returns JSON with merged JSONL or explicit conflicts.
    Merge {
        #[arg(long)]
        base: PathBuf,
        #[arg(long)]
        ours: PathBuf,
        #[arg(long)]
        theirs: PathBuf,
    },
}

async fn run(cli: Cli) -> Result<Option<Value>, Error> {
    match cli.command {
        Command::Status {
            work,
            json,
            offset,
            limit,
        } => run_status(&cli.root, work.as_deref(), json, offset, limit, false),
        Command::Doctor {
            work,
            json,
            offset,
            limit,
        } => run_status(&cli.root, work.as_deref(), json, offset, limit, true),
        Command::Save {
            name,
            work,
            context,
            proxy,
            codex_args,
        } => ostk_gpt_cache::save::save(ostk_gpt_cache::save::SaveArguments {
            root: cli.root,
            name,
            context,
            work,
            proxy,
            codex_args,
        })
        .await
        .map(Some),
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
                    non_interactive,
                    resume: None,
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
        Command::Work { command } => run_work(&cli.root, command).map(Some),
        Command::Project {
            name,
            work,
            proxy,
            inspect,
            non_interactive,
            resume,
        } => {
            run_project(ProjectArguments {
                root: cli.root,
                name,
                work,
                inspect,
                non_interactive,
                resume,
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
    if let Some(work) = &request.work {
        let binding = match ostk_gpt_cache::workspace::WorktreeBinding::inspect(
            &request.root,
            project.project_id(),
            &request.name,
            work,
        ) {
            Ok(binding) => binding,
            Err(error) if error.code == "WORKSPACE_GIT_FAILED" => {
                // Context inspection remains useful before Git initialization.
                // Launch still requires a valid committed repository.
                let mut output = project.inspect(&request.name, Some(work))?;
                output["launch_request"] = preview;
                output["worktree"] = json!({"available":false,"error":error});
                return Ok(output);
            }
            Err(error) => return Err(error),
        };
        let mut output = if binding.existing {
            Project::load(&binding.root)?.inspect(&request.name, Some(work))?
        } else {
            project.inspect(&request.name, Some(work))?
        };
        output["launch_request"] = preview;
        output["worktree"] = json!(binding);
        return Ok(output);
    }
    let mut output = project.inspect(&request.name, None)?;
    output["launch_request"] = preview;
    Ok(output)
}

fn run_work(root: &std::path::Path, command: WorkCommand) -> Result<Value, Error> {
    use ostk_gpt_cache::work_records as records;
    let convert = |e: records::Error| Error {
        code: e.code,
        message: e.to_string(),
    };
    match command {
        WorkCommand::List => records::read(root).map(|v| json!(v)).map_err(convert),
        WorkCommand::Create {
            title,
            acceptance,
            depends_on,
        } => records::create(root, &title, &acceptance, &depends_on)
            .map(|v| json!(v.record))
            .map_err(convert),
        WorkCommand::Update {
            id,
            status,
            expected,
        } => {
            let status = serde_json::from_value(json!(status)).map_err(|_| Error {
                code: "INVALID_STATUS",
                message: "invalid work status".into(),
            })?;
            records::update(root, &id, status, &expected)
                .map(|v| json!(v.record))
                .map_err(convert)
        }
        WorkCommand::Merge { base, ours, theirs } => {
            fn read(path: &std::path::Path) -> Result<Vec<u8>, Error> {
                use std::io::Read;
                let mut options = std::fs::OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                }
                let file = options.open(path).map_err(|_| Error {
                    code: "WORK_INPUT",
                    message: "merge input unavailable".into(),
                })?;
                if !file
                    .metadata()
                    .is_ok_and(|m| m.is_file() && m.len() <= records::MAX_FILE_BYTES as u64)
                {
                    return Err(Error {
                        code: "WORK_INPUT",
                        message: "merge input must be a bounded regular file".into(),
                    });
                }
                let mut bytes = Vec::new();
                file.take(records::MAX_FILE_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Error {
                        code: "WORK_INPUT",
                        message: "merge input read failed".into(),
                    })?;
                Ok(bytes)
            }
            match records::merge(&read(&base)?, &read(&ours)?, &read(&theirs)?).map_err(convert)? {
                records::MergeResult::Merged { bytes, .. } => Ok(
                    json!({"outcome":"merged", "jsonl":String::from_utf8(bytes).expect("validated UTF-8")}),
                ),
                conflicts => {
                    println!("{}", render_output(json!(conflicts))?);
                    std::process::exit(1);
                }
            }
        }
        WorkCommand::Id => unreachable!("handled separately"),
    }
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

fn run_status(
    root: &std::path::Path,
    work: Option<&str>,
    json: bool,
    offset: usize,
    limit: usize,
    doctor: bool,
) -> Result<Option<Value>, Error> {
    let report = ostk_gpt_cache::status::collect(root, work, offset, limit)?;
    let text = if json {
        serde_json::to_string_pretty(&report).expect("status report serializes")
    } else {
        ostk_gpt_cache::status::render(&report)
    };
    if text.len().saturating_add(1) > ostk_gpt_cache::project::Limits::default().output_bytes {
        return Err(Error {
            code: "LIMIT_EXCEEDED",
            message: "status output byte limit".into(),
        });
    }
    println!("{text}");
    if doctor && report.needs_attention() {
        std::process::exit(1);
    }
    Ok(None)
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
