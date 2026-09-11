use super::super::{MAX_ROWS, Output, array, safe, scalar, text, yes};
use serde_json::Value;

pub(in super::super) fn finish(out: &mut Output, value: &Value, applied: bool) {
    out.line(format!(
        "Finish {} into {}: {}",
        safe(text(value, "work")),
        safe(text(value, "target_branch")),
        safe(&text(value, "state").replace('_', " "))
    ));
    out.field("Target checkout", &value["target_root"]);
    out.field("Worker checkout", &value["worker_root"]);
    out.field("Worker branch", &value["worker_branch"]);
    out.line(format!(
        "Committed changes to review: {}",
        array(value, "changes").len()
    ));
    for change in array(value, "changes").iter().take(MAX_ROWS) {
        out.line(format!(
            "  {}  [{}]{}",
            safe(text(change, "path")),
            safe(text(change, "category")),
            if yes(change, "both_changed") {
                " (both sides changed)"
            } else {
                ""
            }
        ));
    }
    out.more(array(value, "changes").len());
    out.field("Work-record merge", &value["records"]["outcome"]);
    for conflict in array(&value["records"], "conflicts").iter().take(MAX_ROWS) {
        out.line(format!(
            "  ! {}: {}",
            safe(text(conflict, "id")),
            safe(text(conflict, "kind"))
        ));
    }
    out.more(array(&value["records"], "conflicts").len());
    out.field("Worker record status", &value["records"]["worker_status"]);
    out.field("Target record status", &value["records"]["target_status"]);
    let context = &value["context"];
    out.field("Next starting context", &context["selector"]);
    if yes(context, "divergent_native_histories") {
        out.line("Native histories diverge; both bundles must be retained.");
    }
    if yes(context, "explicit_choice_required") && !yes(context, "explicitly_selected") {
        out.line("Choose the next context explicitly with --context SELECTOR, then generate a new preview.");
    }
    out.field("Context selection issue", &context["selection_error"]);
    let artifacts = array(value, "artifacts");
    out.line(format!(
        "Native artifact files checked: {}",
        artifacts.len()
    ));
    for artifact in artifacts
        .iter()
        .filter(|a| !yes(a, "retained_in_result"))
        .take(MAX_ROWS)
    {
        out.line(format!(
            "  ! Not retained: {}",
            safe(text(artifact, "path"))
        ));
    }
    out.strings("Blockers", array(value, "blockers"));
    out.strings("Guidance", array(value, "guidance"));
    if !applied {
        out.field("Plan hash", &value["plan_sha256"]);
        match text(value, "state") {
            "fast_forward_ready" if array(value, "blockers").is_empty() => {
                let mut args = vec!["astral", "--root", text(value, "target_root"), "finish", "--work", text(value, "work"), "--into", text(value, "target_branch")];
                if yes(context, "explicitly_selected") { args.extend(["--context", text(context, "selector")]); }
                args.extend(["--apply", text(value, "plan_sha256")]);
                out.command("Apply after review", &args);
            }
            "already_integrated" => out.line("Integration is already present; no merge is needed."),
            _ => out.line("Use ordinary Git merge/reconciliation to resolve the blockers, preserve context artifacts, then run finish again."),
        }
    }
    if applied || text(value, "state") == "already_integrated" {
        out.line("Review evidence, explicitly update the work record, and commit that update. Finish does not mark work complete.");
        out.argv("Start the reviewed context", &value["next_context_argv"]);
    } else {
        out.line("After integration, update the work record with reviewed evidence and commit it. This preview does not stage, commit, or mark work complete.");
    }
}

pub(in super::super) fn finish_result(out: &mut Output, value: &Value) {
    out.line(if yes(value, "applied") {
        "Fast-forward integration applied."
    } else {
        "Integration already recorded; no changes applied."
    });
    out.field("Reviewed plan", &value["reviewed_plan_sha256"]);
    finish(out, &value["plan"], true);
}

pub(in super::super) fn save(out: &mut Output, value: &Value) {
    let publication = &value["publication"];
    out.line(format!(
        "Saved projection {} for {}",
        safe(text(publication, "projection")),
        safe(text(value, "work"))
    ));
    out.field("Worker checkout", &value["worktree"]);
    out.field("Branch", &value["branch"]);
    for (label, key) in [
        ("Projection manifest", "projection_manifest"),
        ("Bundle manifest", "bundle_manifest"),
        ("Native payload", "payload"),
    ] {
        out.field(label, &publication[key]["path"]);
    }
    out.field("Bundle hash", &publication["bundle_manifest"]["sha256"]);
    out.line("Exported files are local changes. Review the diff and deliberately commit them for handoff; previous bundles are retained.");
    out.command(
        "Review changes",
        &["git", "-C", text(value, "worktree"), "status", "--short"],
    );
}

pub(in super::super) fn hooks_plan(out: &mut Output, value: &Value) {
    out.line(format!(
        "{} {} hooks — preview",
        safe(text(value, "action")),
        safe(text(value, "target"))
    ));
    out.field("Repository", &value["root"]);
    out.field("Hook command", &value["command"]);
    out.field("Scope", &value["scope"]);
    out.strings("Planned changes", array(value, "changes"));
    if array(value, "changes").is_empty() {
        out.line("No file changes are planned.");
    }
    if let Some(files) = value["files"].as_object() {
        out.line("Inspected files:");
        for (path, state) in files.iter().take(MAX_ROWS) {
            out.line(format!("  {}: {}", safe(path), safe(text(state, "state"))));
        }
        out.more(files.len());
    }
    out.strings("Blockers", array(value, "blockers"));
    out.field("Plan hash", &value["plan_sha256"]);
    if array(value, "blockers").is_empty() {
        out.command(
            "Apply after review",
            &[
                "astral",
                "--root",
                text(value, "root"),
                "hooks",
                text(value, "action"),
                text(value, "target"),
                "--apply",
                text(value, "plan_sha256"),
            ],
        );
    } else {
        out.line("Resolve the blockers and generate a new preview. Existing files are retained.");
    }
    if text(value, "target") == "codex" {
        out.line("Codex must resolve astral on PATH and approve/load the hook configuration. Runtime trust is not observed here.");
    }
}

pub(in super::super) fn hooks_status(out: &mut Output, value: &Value) {
    out.line("Lifecycle hooks");
    let git = &value["git"];
    out.line(format!(
        "Git: {} (registered: {})",
        if yes(git, "enabled") {
            "enabled"
        } else {
            "not enabled"
        },
        scalar(&git["registered"])
    ));
    let codex = &value["codex"];
    out.line(format!(
        "Codex: {} (registered: {}; registration enabled: {})",
        if yes(codex, "configured") {
            "configured"
        } else {
            "not fully configured"
        },
        scalar(&codex["registered"]),
        scalar(&codex["registration_enabled"])
    ));
    out.field(
        "Codex owned configuration intact",
        &codex["owned_configuration_intact"],
    );
    out.line("Codex runtime trust/loading is not observed. Review hooks in Codex with /hooks.");
    if !yes(git, "enabled") {
        out.command(
            "Review Git setup",
            &["astral", "hooks", "install", "git", "--dry-run"],
        );
    }
    if !yes(codex, "configured") {
        out.command(
            "Review Codex setup",
            &["astral", "hooks", "install", "codex", "--dry-run"],
        );
    }
}

pub(in super::super) fn hooks_applied(out: &mut Output, value: &Value) {
    let target = text(value, "target");
    let action = text(value, "action");
    out.line(format!(
        "{} hooks {}: {}",
        safe(target),
        safe(action),
        if yes(value, "changed") {
            "applied"
        } else {
            "no changes needed"
        }
    ));
    if target == "git" {
        out.field("Git registration enabled", &value["enabled"]);
        if action == "uninstall" {
            out.line("Git shims are retained; their Astral registration is disabled.");
        }
    } else {
        out.field("Codex registration enabled", &value["registration_enabled"]);
        if action == "uninstall" {
            out.line("Only unchanged owned groups were removed. Modified and unowned callbacks are retained and may still run.");
        } else {
            out.line("Review .codex/hooks.json and use /hooks in Codex to review runtime trust. Runtime loading is not verified here.");
        }
        out.field(
            "Configuration observation issue",
            &value["configuration"]["observation_error"],
        );
    }
    out.field("Scope", &value["scope"]);
    out.command("Check setup", &["astral", "hooks", "status"]);
}
