//! Hook setup's interactive presentation; mutation and stale-plan checks stay in
//! the installer shared by interactive and scripted invocations.
use astral::{
    hooks::install::{self, Action, Plan, Target},
    project::{Error, Result},
};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::{
    io::{BufRead, IsTerminal, Write},
    path::Path,
};

#[derive(Subcommand)]
pub(super) enum HooksCommand {
    /// Show a plan and ask to install it in a terminal; otherwise preview only.
    Install(Change),
    /// Show a plan and ask to remove owned configuration; retain prior bytes.
    Uninstall(Change),
    Status,
}

#[derive(Args)]
#[command(
    after_help = "A terminal invocation shows the plan and asks for confirmation. Without a terminal, or with --json, the default is a preview. Use --yes for unattended application, --dry-run for a preview, or --apply HASH for a previously reviewed plan. Add --json for formatted JSON. All application modes retain the same stale-plan and ownership checks."
)]
pub(super) struct Change {
    #[arg(value_parser = ["git", "codex"])]
    target: String,
    /// Apply the exact previously reviewed plan (compatible with existing scripts).
    #[arg(long, conflicts_with_all = ["yes", "dry_run"])]
    apply: Option<String>,
    /// Apply the current plan without prompting; conflicts still block changes.
    #[arg(long, conflicts_with = "dry_run")]
    yes: bool,
    /// Preview the plan and make no changes; add --json for structured output.
    #[arg(long)]
    dry_run: bool,
}

fn io_error(_: std::io::Error) -> Error {
    Error {
        code: "HOOK_CONFIRMATION_IO",
        message: "hook setup terminal input/output failed; inspect hooks status before retrying"
            .into(),
    }
}

fn label(target: Target) -> &'static str {
    match target {
        Target::Git => "Git",
        Target::Codex => "Codex",
    }
}

fn show_plan(plan: &Plan, output: &mut impl Write) -> std::io::Result<()> {
    let action = match plan.action {
        Action::Install => "Install",
        Action::Uninstall => "Uninstall",
    };
    writeln!(output, "{action} {} hooks", label(plan.target))?;
    // JSON strings escape control characters in paths/commands before rendering.
    writeln!(output, "Repository: {}", json!(plan.root))?;
    writeln!(output, "Command: {}", json!(plan.command))?;
    writeln!(output, "Scope: {}", plan.scope)?;
    writeln!(output, "Planned changes:")?;
    if plan.changes.is_empty() {
        writeln!(output, "  No file changes planned.")?;
    }
    for change in &plan.changes {
        writeln!(output, "  - {change}")?;
    }
    if !plan.blockers.is_empty() {
        writeln!(output, "Blocked:")?;
        for blocker in &plan.blockers {
            writeln!(output, "  - {blocker}")?;
        }
    }
    output.flush()
}

fn confirm(input: &mut impl BufRead, output: &mut impl Write) -> Result<bool> {
    super::confirmation::ask(input, output, "Apply this hook setup?").map_err(io_error)
}

pub(super) fn run(root: &Path, command: HooksCommand, json: bool) -> Result<Option<Value>> {
    let (change, action) = match command {
        HooksCommand::Status => return super::print_output(&install::status(root)?, json, root),
        HooksCommand::Install(change) => (change, Action::Install),
        HooksCommand::Uninstall(change) => (change, Action::Uninstall),
    };
    let target = if change.target == "git" {
        Target::Git
    } else {
        Target::Codex
    };
    let executable = std::env::current_exe().map_err(|_| Error {
        code: "HOOK_EXECUTABLE",
        message: "current executable unavailable".into(),
    })?;
    if let Some(expected) = change.apply {
        return super::print_output(
            &install::apply(root, &executable, target, action, &expected)?,
            json,
            root,
        );
    }
    let plan = install::plan(root, &executable, target, action)?;
    let interactive = !json && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if change.dry_run || (!change.yes && !interactive) {
        super::print_output(&json!(plan), json, root)?;
        if !change.dry_run && !json {
            writeln!(std::io::stderr().lock(), "Preview only: run in a terminal to review and confirm, or use --yes to apply this setup without prompting.").map_err(io_error)?;
        }
        return Ok(None);
    }
    if change.yes {
        return super::print_output(
            &install::apply(root, &executable, target, action, &plan.plan_sha256)?,
            json,
            root,
        );
    }
    let mut output = std::io::stderr().lock();
    show_plan(&plan, &mut output).map_err(io_error)?;
    if !plan.blockers.is_empty() {
        return Err(Error {
            code: "HOOK_INSTALL_BLOCKED",
            message: "hook setup conflicts require review; no changes were applied".into(),
        });
    }
    if !confirm(&mut std::io::stdin().lock(), &mut output)? {
        writeln!(output, "Cancelled; no hook changes.").map_err(io_error)?;
        return Ok(None);
    }
    // Never replan and silently apply a different operation after confirmation.
    let result = install::apply(root, &executable, target, action, &plan.plan_sha256)?;
    let message = if result.get("changed") == Some(&Value::Bool(false)) {
        "No changes were needed."
    } else {
        match (target, action) {
            (Target::Git, Action::Install) => "Git hooks installed.",
            (Target::Git, Action::Uninstall) => "Git hooks disabled; hook files retained.",
            (Target::Codex, Action::Install) => {
                "Codex hook configuration installed. Runtime trust is managed in Codex."
            }
            (Target::Codex, Action::Uninstall) => {
                "Unchanged Astral-owned Codex groups removed; modified and unowned groups retained."
            }
        }
    };
    writeln!(output, "{message}").map_err(io_error)?;
    Ok(None)
}
