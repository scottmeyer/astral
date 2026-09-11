#[path = "workflow/plans.rs"]
mod plans;
use super::{MAX_ROWS, Output, array, safe, scalar, text, yes};
pub(super) use plans::{finish, finish_result, hooks_applied, hooks_plan, hooks_status, save};
use serde_json::Value;

pub(super) fn lifecycle(out: &mut Output, value: &Value) {
    out.line(format!("Context check: {}", safe(text(value, "root"))));
    out.field("Requested scope", &value["scope"]);
    out.field("Branch", &value["branch"]);
    if value["head"].is_null() {
        out.line("HEAD: no commit yet.");
    }
    for (label, key) in [
        ("Staged", "index"),
        ("Committed", "committed"),
        ("Working", "worktree"),
    ] {
        let context = &value[key];
        out.line(format!(
            "{label} context: {} ({} files)",
            safe(text(context, "availability")),
            scalar(&context["observed_files"])
        ));
        out.field("  Problem", &context["error_code"]);
    }
    let worker = &value["worker"];
    if !worker.is_null() {
        out.line(format!(
            "Worker {}: ownership {}",
            safe(text(worker, "work")),
            safe(text(worker, "ownership"))
        ));
        out.field("  Selected context", &worker["selector"]);
        out.field(
            "  Context changed since staging",
            &worker["changed_since_staging"],
        );
        out.field(
            "  Native selection changed",
            &worker["native_selection_changed"],
        );
        if yes(worker, "pending_operation") {
            out.line("  A staging/save acknowledgement is pending; this check does not clear it.");
        }
        if !yes(worker, "bound_to_current_checkout") {
            out.line("  The worker is not validated against this checkout.");
        }
    }
    let diagnostics = array(value, "diagnostics");
    if diagnostics.is_empty() {
        out.line("No context differences or binding diagnostics were reported.");
    } else {
        out.line("Needs review:");
        lifecycle_diagnostics(out, value, diagnostics);
    }
    out.line("Current local context observation; no worker state was changed.");
}

fn lifecycle_diagnostics(out: &mut Output, value: &Value, diagnostics: &[Value]) {
    let mut working_diff = false;
    let mut staged_diff = false;
    let mut validate = false;
    let mut inspect_worker = false;
    let mut recover = false;
    for diagnostic in diagnostics.iter().take(MAX_ROWS) {
        let code = diagnostic.as_str().unwrap_or("UNKNOWN_DIAGNOSTIC");
        let description = match code {
            "UNSTAGED_CONTEXT_DIFFERENCE" => {
                working_diff = true;
                "Working context differs from the index. Review the changes and stage intended context alongside its code."
            }
            "UNCOMMITTED_CONTEXT_DIFFERENCE" => {
                staged_diff = true;
                "Staged context differs from HEAD. Review the candidate before committing."
            }
            "INDEX_CONTEXT_INVALID" => {
                staged_diff = true;
                validate = true;
                "Staged context is invalid or incomplete. Fix the intended working context, then stage its complete references and files."
            }
            "COMMITTED_CONTEXT_INVALID" => {
                validate = true;
                "Committed context is invalid or incomplete. Repair and validate the working context, then review and commit the correction."
            }
            "WORKTREE_CONTEXT_INVALID" => {
                validate = true;
                "Working context is invalid or incomplete. Validate it to locate the document, manifest, or reference that needs repair."
            }
            "UNBORN_HEAD" => {
                "This repository has no commit yet. Review and commit the initial code and context before creating a bound worker."
            }
            "WORKER_OBSERVATION_UNAVAILABLE" => {
                inspect_worker = true;
                "The worker binding could not be inspected. Check its recorded location, ownership, and local storage before continuing."
            }
            "WORKER_BINDING_NEEDS_REVIEW" => {
                inspect_worker = true;
                recover = true;
                "The recorded worker binding has an identity or storage problem. Preserve the checkout and review its diagnostics."
            }
            "WORKER_NOT_BOUND" => {
                inspect_worker = true;
                "No validated worker binding was found for this work ID. Inspect status before choosing or starting a worker."
            }
            "WORKER_DIFFERENT_CHECKOUT" => {
                inspect_worker = true;
                "This worker is bound to a different checkout. Inspect its recorded path and preserve changes in both checkouts."
            }
            "WORKER_RECOVERY_REVIEW_REQUIRED" => {
                inspect_worker = true;
                recover = true;
                "Worker staging or saving is incomplete or uncertain. Preview recovery before attempting continuation; do not repeat an unknown operation."
            }
            "WORKER_CONTEXT_CHANGED" => {
                inspect_worker = true;
                recover = true;
                "Selected context changed since the worker was staged. Review the difference and any recovery proposal before continuing."
            }
            "WORKER_NATIVE_SELECTION_CHANGED" => {
                inspect_worker = true;
                "The selected native history differs from the worker's recorded history. Preserve both bundles; use a new work ID for an intentional new starting projection."
            }
            "WORKER_CONTEXT_UNAVAILABLE" => {
                inspect_worker = true;
                validate = true;
                "The worker's selected context or work record cannot be resolved. Validate the current declarations and inspect worker status."
            }
            _ => {
                "An additional context diagnostic needs review. Use --json to inspect the complete report."
            }
        };
        out.line(format!("  ! {description} ({})", safe(code)));
    }
    out.more(diagnostics.len());
    let root = text(value, "root");
    if working_diff {
        out.command(
            "Review unstaged context",
            &["git", "-C", root, "diff", "--", ".astral"],
        );
    }
    if staged_diff {
        out.command(
            "Review staged context",
            &["git", "-C", root, "diff", "--cached", "--", ".astral"],
        );
    }
    if validate {
        out.command(
            "Validate working context",
            &["astral", "--root", root, "context", "validate"],
        );
    }
    let work = text(&value["worker"], "work");
    for (needed, label, command) in [
        (inspect_worker, "Inspect worker", "status"),
        (recover, "Preview recovery", "recover"),
    ] {
        if needed {
            let mut args = vec!["astral", "--root", root, command];
            if !work.is_empty() {
                args.extend(["--work", work]);
            }
            out.command(label, &args);
        }
    }
}

fn binding_row(out: &mut Output, value: &Value) {
    let observation = &value["observation"]["observation"];
    out.line(format!(
        "  {}: ownership {}",
        safe(text(value, "work")),
        safe(text(observation, "ownership"))
    ));
    if !yes(value, "in_current_register") {
        out.line("    Work ID is absent from this checkout's current register.");
    }
    out.field("    Context", &observation["selector"]);
    out.field("    Checkout", &observation["plan"]["root"]);
    out.field("    Binding state", &observation["plan"]["status"]);
    if !value["error"].is_null() {
        out.issues(std::slice::from_ref(&value["error"]));
    }
    out.issues(array(observation, "issues"));
}

pub(super) fn recovery_inventory(out: &mut Output, value: &Value) {
    let rows = array(value, "bindings");
    out.line(format!(
        "Recovery inventory: {} recorded workers; showing {} from offset {}.",
        scalar(&value["total_bindings"]),
        rows.len(),
        scalar(&value["offset"])
    ));
    for row in rows.iter().take(MAX_ROWS) {
        binding_row(out, row);
    }
    out.more(rows.len());
    if rows.is_empty() {
        out.line("No worker bindings on this page.");
    }
    out.issues(array(value, "issues"));
    if let Some(next) = value["next_offset"].as_u64() {
        out.command(
            "Next worker page",
            &["astral", "recover", "--offset", &next.to_string()],
        );
    }
    let launches = &value["launches"];
    if !launches.is_null() {
        out.line(format!(
            "Launch receipts: {} observations in {}",
            scalar(&launches["total_observations"]),
            safe(text(launches, "state_root"))
        ));
        for launch in array(launches, "launches").iter().take(MAX_ROWS) {
            out.line(format!(
                "  {}: {} / {}",
                safe(text(launch, "launch_id")),
                safe(text(launch, "ownership")),
                safe(text(launch, "continuation"))
            ));
            out.field("    Context", &launch["receipt"]["selector"]);
            out.field("    Checkout", &launch["receipt"]["workspace"]);
            out.issues(array(launch, "issues"));
        }
        out.more(array(launches, "launches").len());
        out.issues(array(launches, "issues"));
        out.field(
            "Foreign receipts excluded",
            &launches["excluded_foreign_launches"],
        );
        out.field("Unrecognized entries", &launches["unrecognized_entries"]);
        if let Some(next) = launches["next_offset"].as_u64() {
            out.command(
                "Next launch page",
                &[
                    "astral",
                    "recover",
                    "--launch-state-root",
                    text(launches, "state_root"),
                    "--offset",
                    &next.to_string(),
                ],
            );
        }
    }
    out.example("Preview a worker repair with astral recover --work ID. Nothing was repaired, resumed, or retried.",
        "Preview a worker repair (replace ID; no repair or resume yet)", &["astral", "recover", "--work", "ID"]);
}

pub(super) fn recovery_plan(out: &mut Output, value: &Value) {
    out.line(format!(
        "Recovery preview for {}: {}",
        safe(text(value, "work")),
        safe(&text(value, "classification").replace('_', " "))
    ));
    out.field("Context", &value["selector"]);
    let observation = &value["binding"]["observation"];
    out.field("Ownership", &observation["ownership"]);
    out.field("Checkout", &observation["plan"]["root"]);
    out.field("Proposed repair", &value["repair"]);
    out.issues(array(observation, "issues"));
    out.issues(array(value, "issues"));
    out.field("Next step", &value["guidance"]);
    out.field("Plan hash", &value["plan_sha256"]);
    if value["repair"].is_string() {
        out.command(
            "Apply from the same project checkout after review",
            &[
                "astral",
                "recover",
                "--work",
                text(value, "work"),
                "--apply",
                text(value, "plan_sha256"),
            ],
        );
    } else {
        out.line(
            "No repair is offered by this preview. Resolve the reported issue and preview again.",
        );
    }
    out.line("No runtime success is inferred. Recovery never repeats an uncertain operation.");
}

pub(super) fn recovered(out: &mut Output, value: &Value) {
    out.line(format!(
        "Recovery for {}: {}",
        safe(text(value, "work")),
        safe(&text(value, "outcome").replace('_', " "))
    ));
    out.field("Recorded repair", &value["repair"]);
    out.line("This records local bookkeeping only; runtime execution was not verified.");
    out.command("Next", &["astral", "status", "--work", text(value, "work")]);
}
