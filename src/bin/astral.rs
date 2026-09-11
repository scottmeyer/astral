use astral::config::Config;
use astral::launcher::{
    DEFAULT_CONTEXT, ProjectArguments, initialization_request, project_request,
};
use astral::project::{Error, Project};
use clap::{Parser, Subcommand, error::ErrorKind};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[path = "astral/arguments.rs"]
mod arguments;
#[path = "astral/hooks_cli.rs"]
mod hooks_cli;
#[path = "astral/output.rs"]
mod output;
#[path = "astral/picker.rs"]
mod picker;

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
    /// Emit formatted JSON for Astral results and diagnostics.
    #[arg(long, global = true)]
    json: bool,
    /// Print a report without opening the context picker.
    #[arg(long, global = true, conflicts_with = "json")]
    plain: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Observe committed, staged and working context without changing worker state.
    Lifecycle {
        #[command(subcommand)]
        command: LifecycleCommand,
    },
    /// Review and confirm opt-in lifecycle hook setup.
    Hooks {
        #[command(subcommand)]
        command: hooks_cli::HooksCommand,
    },
    /// Advisory hook entry points; installers normally invoke these.
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },
    #[command(hide = true)]
    HookCheck {
        #[arg(long, value_parser = ["index", "head", "worktree"])]
        scope: String,
        #[arg(long)]
        audience: String,
        #[arg(long)]
        session_scope: String,
        #[arg(long)]
        claimed_session: Option<String>,
        #[arg(long)]
        require_git_registration: bool,
        #[arg(long)]
        force_notice: bool,
    },
    /// Invoke real git commit, preserving its index, hooks, signing and exit status.
    #[command(
        after_help = "Pass literal Git arguments after --, for example: astral commit -- -am 'Update context'. Lifecycle advice requires installed Git hooks; Git decides which hooks run."
    )]
    Commit {
        #[arg(last = true, allow_hyphen_values = true)]
        git_args: Vec<std::ffi::OsString>,
    },
    /// Preview interrupted operations; apply only the exact reviewed plan hash.
    Recover {
        #[arg(long)]
        work: Option<String>,
        #[arg(long, requires = "work")]
        apply: Option<String>,
        /// Explicit trusted receipt root for inventory; never inferred from project files.
        #[arg(long, conflicts_with = "work")]
        launch_state_root: Option<PathBuf>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 32)]
        limit: usize,
    },
    /// Preview branch completion; explicitly apply only a reviewed fast-forward.
    Finish {
        #[arg(long)]
        work: String,
        #[arg(long)]
        into: String,
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        apply: Option<String>,
    },
    /// Inspect recorded workers and selected context without launching a runtime.
    Status {
        #[arg(long)]
        work: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = astral::status::DEFAULT_PAGE_SIZE)]
        limit: usize,
    },
    /// Diagnose local resume blockers; exit 1 when the inspected page needs attention.
    Doctor {
        #[arg(long)]
        work: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = astral::status::DEFAULT_PAGE_SIZE)]
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
    /// List or validate the repository's available contexts.
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
        after_help = "The context defaults to project-context when NAME is omitted. Use --pick to search contexts and issues, then review launch in a terminal. An explicit NAME must come immediately after project. Astral recognizes --root, --work, --inspect, --json, --plain, --pick, --proxy, --non-interactive, and --resume before the first literal --. Put all Codex arguments after -- if any flag value resembles an Astral option. --non-interactive runs codex exec resume; use exec-supported Codex options, including -c sandbox_mode and -c approval_policy. --resume names a private Astral launch receipt."
    )]
    /// Launch a context, resume a worker, or preview with --inspect.
    Project {
        #[arg(default_value = DEFAULT_CONTEXT)]
        name: String,
        /// Search contexts and work items, then review launch interactively.
        #[arg(long)]
        pick: bool,
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
enum LifecycleCommand {
    Check {
        #[arg(long, default_value = "worktree", value_parser = ["index", "head", "worktree"])]
        scope: String,
        #[arg(long)]
        work: Option<String>,
    },
}
#[derive(Subcommand)]
enum HookCommand {
    Git {
        event: String,
        #[arg(long)]
        manual: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<std::ffi::OsString>,
    },
    Codex,
}

fn lifecycle_scope(value: &str) -> astral::lifecycle::Scope {
    use astral::lifecycle::Scope;
    match value {
        "index" => Scope::Index,
        "head" => Scope::Head,
        _ => Scope::Worktree,
    }
}

#[derive(Subcommand)]
enum ContextCommand {
    /// Check the index, documents, work records and native bundle references.
    Validate,
    /// Pick a context in a terminal; use --plain or --json for a report.
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
    /// Three-way merge by ID; use --json to retrieve merged JSONL or conflicts.
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
        Command::Lifecycle {
            command: LifecycleCommand::Check { scope, work },
        } => Ok(Some(json!(astral::lifecycle::check(
            &cli.root,
            lifecycle_scope(&scope),
            work.as_deref()
        )?))),
        Command::Hooks { command } => hooks_cli::run(&cli.root, command, cli.json),
        Command::Hook { command } => {
            match command {
                HookCommand::Git {
                    event,
                    manual,
                    args: _,
                } => astral::hooks::git_callback(&cli.root, &event, manual),
                HookCommand::Codex => astral::hooks::codex_callback(),
            }
            Ok(None)
        }
        Command::HookCheck {
            scope,
            audience,
            session_scope,
            claimed_session,
            require_git_registration,
            force_notice,
        } => print_output(
            &json!(astral::hooks::check_callback(
                &cli.root,
                lifecycle_scope(&scope),
                &audience,
                &session_scope,
                claimed_session.as_deref(),
                require_git_registration,
                force_notice
            )?),
            true,
            &cli.root,
        ),
        Command::Commit { git_args } => {
            let mut command = std::process::Command::new("git");
            command
                .arg("-C")
                .arg(&cli.root)
                .arg("commit")
                .args(git_args);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                let error = command.exec();
                Err(Error {
                    code: "GIT_COMMIT_EXEC",
                    message: format!("cannot invoke git commit: {error}"),
                })
            }
            #[cfg(not(unix))]
            {
                let status = command.status().map_err(|_| Error {
                    code: "GIT_COMMIT_EXEC",
                    message: "cannot invoke git commit".into(),
                })?;
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Command::Recover {
            work,
            apply,
            launch_state_root,
            offset,
            limit,
        } => {
            let value = if let Some(work) = work {
                if offset != 0 || limit != 32 {
                    return Err(Error {
                        code: "CLI_USAGE",
                        message: "pagination applies only to recovery inventory".into(),
                    });
                }
                match apply {
                    Some(expected) => astral::recovery::apply(&cli.root, &work, &expected)?,
                    None => json!(astral::recovery::plan(&cli.root, &work)?),
                }
            } else {
                astral::recovery::inventory(&cli.root, launch_state_root.as_deref(), offset, limit)?
            };
            Ok(Some(value))
        }
        Command::Finish {
            work,
            into,
            context,
            apply,
        } => {
            let request = astral::completion::FinishRequest {
                root: cli.root,
                work,
                into,
                context,
            };
            let value = match apply {
                Some(expected) => json!(astral::completion::apply(&request, &expected)?),
                None => json!(astral::completion::plan(&request)?),
            };
            Ok(Some(value))
        }
        Command::Status {
            work,
            offset,
            limit,
        } => run_status(&cli.root, work.as_deref(), cli.json, offset, limit, false),
        Command::Doctor {
            work,
            offset,
            limit,
        } => run_status(&cli.root, work.as_deref(), cli.json, offset, limit, true),
        Command::Save {
            name,
            work,
            context,
            proxy,
            codex_args,
        } => astral::save::save(astral::save::SaveArguments {
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
                    route: astral::launcher::Route::Direct,
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
        } => {
            if !cli.json && !cli.plain && picker::available() {
                picker::run(ProjectArguments::parse(cli.root, Vec::new())?).await
            } else {
                Project::load(&cli.root)?.list().map(Some)
            }
        }
        Command::Work {
            command: WorkCommand::Id,
        } => {
            let project = Project::load(&cli.root)?;
            let id = astral::work::generate_id(project.work_ids()).map_err(|error| Error {
                code: error.code(),
                message: error.to_string(),
            })?;
            Ok(Some(json!({"id":id})))
        }
        Command::Work { command } => run_work(&cli.root, command, cli.json).map(Some),
        Command::Project {
            name,
            pick,
            work,
            proxy,
            inspect,
            non_interactive,
            resume,
        } => {
            let request = ProjectArguments {
                root: cli.root,
                name,
                work,
                inspect,
                non_interactive,
                resume,
                route: if proxy {
                    astral::launcher::Route::Proxy
                } else {
                    astral::launcher::Route::Direct
                },
                codex_args: Vec::new(),
            };
            run_project_mode(request, pick, cli.json, cli.plain).await
        }
    }
}

fn inspect_project(request: ProjectArguments) -> Result<Value, Error> {
    let preview = request.preview()?;
    let project = Project::load(&request.root)?;
    if let Some(work) = &request.work {
        let binding = match astral::workspace::WorktreeBinding::inspect(
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

fn run_work(root: &std::path::Path, command: WorkCommand, json: bool) -> Result<Value, Error> {
    use astral::work_records as records;
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
                    print_output(&json!(conflicts), json, root)?;
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
    let code = astral::launch::project(request).await?;
    std::process::exit(code);
}

async fn run_project_mode(
    request: ProjectArguments,
    pick: bool,
    json: bool,
    plain: bool,
) -> Result<Option<Value>, Error> {
    if pick {
        if json || plain || request.inspect || request.non_interactive || request.resume.is_some() {
            return Err(Error { code: "CLI_USAGE", message: "--pick cannot be combined with --json, --plain, --inspect, --non-interactive or --resume; use a direct astral project command for those modes".into() });
        }
        picker::run(request).await
    } else {
        run_project(request).await
    }
}

async fn run_init(
    request: ProjectArguments,
    non_interactive: bool,
) -> Result<Option<Value>, Error> {
    if request.inspect {
        return Ok(Some(
            json!({"mode": "inspect", "operation": "initialize", "prompt_version": astral::launch::INIT_PROMPT_VERSION, "prompt": astral::launch::INIT_PROMPT, "non_interactive": non_interactive, "launch_request": request.preview()?}),
        ));
    }
    let code =
        astral::launch::initialize(&request.root, &request.codex_args, non_interactive).await?;
    std::process::exit(code);
}

async fn run_proxy(config: Config) -> Result<Option<Value>, Error> {
    astral::server::run(config)
        .await
        .map(|()| None)
        .map_err(|error| Error {
            code: "PROXY_FAILED",
            message: error.to_string(),
        })
}

fn render_output(value: &Value, json: bool, root: &Path) -> Result<String, Error> {
    let text = if json {
        serde_json::to_string_pretty(value).expect("JSON Value serializes")
    } else {
        output::render_in(value, root)?
    };
    // Bound the actual emitted bytes, including indentation and the final newline.
    if text.len().saturating_add(1) > astral::project::Limits::default().output_bytes {
        return Err(Error {
            code: "LIMIT_EXCEEDED",
            message: "output byte limit".into(),
        });
    }
    Ok(text)
}

fn print_output(output: &Value, json: bool, root: &Path) -> Result<Option<Value>, Error> {
    let text = render_output(output, json, root)?;
    println!("{text}");
    Ok(None)
}

fn run_status(
    root: &std::path::Path,
    work: Option<&str>,
    json: bool,
    offset: usize,
    limit: usize,
    doctor: bool,
) -> Result<Option<Value>, Error> {
    let report = astral::status::collect(root, work, offset, limit)?;
    let text = if json {
        serde_json::to_string_pretty(&report).expect("status report serializes")
    } else {
        astral::status::render(&report)
    };
    if text.len().saturating_add(1) > astral::project::Limits::default().output_bytes {
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

fn finish(result: Result<Option<Value>, Error>, json: bool, root: &Path) {
    let result = result.and_then(|output| {
        output
            .map(|value| render_output(&value, json, root))
            .transpose()
    });
    match result {
        Ok(Some(output)) => println!("{output}"),
        Ok(None) => {}
        Err(error) => {
            if json {
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"error": error}))
                        .expect("error serializes")
                );
            } else {
                eprintln!("{}", output::error_in(&error, root));
            }
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (presentation, presentation_args) = arguments::presentation(&args);
    let json = presentation.json;
    if json && presentation.plain {
        finish(
            Err(Error {
                code: "CLI_USAGE",
                message: "--plain cannot be combined with --json".into(),
            }),
            json,
            Path::new("."),
        );
        return;
    }
    match initialization_request(&presentation_args) {
        Ok(Some((request, non_interactive))) => {
            let root = request.root.clone();
            finish(run_init(request, non_interactive).await, json, &root);
            return;
        }
        Err(error) => {
            finish(Err(error), json, Path::new("."));
            return;
        }
        Ok(None) => {}
    }
    match project_request(&presentation_args) {
        Ok(Some(request)) => {
            let root = request.root.clone();
            finish(
                run_project_mode(request, presentation.pick, json, presentation.plain).await,
                json,
                &root,
            );
            return;
        }
        Err(error) => {
            finish(Err(error), json, Path::new("."));
            return;
        }
        Ok(None) => {}
    }
    // Preserve the former proxy CLI's direct option form under its new name.
    let direct_proxy = args.is_empty()
        || presentation_args.first().is_some_and(|arg| {
            arg.as_encoded_bytes().starts_with(b"-")
                && !["--help", "-h", "--version", "-V", "--root", "--"]
                    .iter()
                    .any(|reserved| arg == reserved)
                && !arg.as_encoded_bytes().starts_with(b"--root=")
        });
    let cli = if direct_proxy {
        Config::try_parse_from(std::iter::once(OsString::from("astral")).chain(presentation_args))
            .map(|config| Cli {
                root: PathBuf::from("."),
                json,
                plain: presentation.plain,
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
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &json!({"status": "help", "message": e.to_string()})
                        )
                        .expect("help serializes")
                    );
                } else {
                    print!("{e}");
                }
                return;
            }
            if json {
                eprintln!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"error": {"code": "CLI_USAGE", "message": e.to_string()}})
                    )
                    .expect("error serializes")
                );
            } else {
                eprint!("{e}");
            }
            std::process::exit(2);
        }
    };
    let json = cli.json;
    let root = cli.root.clone();
    finish(run(cli).await, json, &root);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation_is_included_in_the_output_budget() {
        let output = json!({"mode": "inspect", "rows": vec![0; 400_000]});
        assert!(
            serde_json::to_vec(&output).unwrap().len()
                < astral::project::Limits::default().output_bytes
        );
        assert_eq!(
            render_output(&output, true, Path::new("."))
                .unwrap_err()
                .code,
            "LIMIT_EXCEEDED"
        );
    }
}
