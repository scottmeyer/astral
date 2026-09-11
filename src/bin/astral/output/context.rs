use super::{MAX_ROWS, Output, array, safe, scalar, text, yes};
use serde_json::Value;
#[path = "context/freshness.rs"]
mod freshness;
pub(super) use freshness::{freshness, freshness_rows};

pub(super) fn validate(out: &mut Output, value: &Value) {
    out.line(format!(
        "Context valid: {}",
        safe(text(value, "project_id"))
    ));
    out.line(format!(
        "{} subsystems, {} projections, {} work items; {} files checked.",
        scalar(&value["subsystems"]),
        scalar(&value["projections"]),
        scalar(&value["work_items"]),
        scalar(&value["observed_files"])
    ));
    out.field(
        "Native bundles validated locally",
        &value["native_artifacts"],
    );
    freshness_rows(out, value);
    out.command("Next", &["astral", "context", "list"]);
}

pub(super) fn list(out: &mut Output, value: &Value) {
    out.line(format!("Contexts in {}", safe(text(value, "project_id"))));
    let rows = array(value, "contexts");
    if rows.is_empty() {
        out.line("No named contexts are registered.");
    }
    for row in rows.iter().take(MAX_ROWS) {
        out.line(format!(
            "  {}  ({})",
            safe(text(row, "selector")),
            safe(text(row, "kind"))
        ));
    }
    out.more(rows.len());
    if let Some(first) = rows.first() {
        out.command(
            "Inspect a context",
            &["astral", "project", text(first, "selector"), "--inspect"],
        );
    }
    out.line("From the same project checkout, use a selector with astral project; add --work ID for a bound worker.");
}

fn launch_preview(out: &mut Output, value: &Value) {
    let request = &value["launch_request"];
    out.field("Requested route", &request["requested_route"]);
    if !request.is_null() {
        out.line(if yes(request, "non_interactive") {
            "Requested interface: non-interactive Codex."
        } else {
            "Requested interface: interactive Codex."
        });
    }
    out.field("Resume receipt", &request["resume"]);
    if !array(request, "codex_args").is_empty() {
        out.arguments("Explicit Codex arguments", &request["codex_args"]);
    }
    out.line("Inspection only; no runtime was launched or bound.");
}

pub(super) fn initialize(out: &mut Output, value: &Value) {
    out.line("Project initialization preview");
    out.line("Codex will inspect the repository and propose a .astral context index. Astral validates the result after Codex exits.");
    launch_preview(out, value);
    out.field("Initializer prompt version", &value["prompt_version"]);
    out.line("Use --json to review the complete initializer prompt before launch.");
    out.line("To initialize, repeat this command without --inspect, preserving your explicit Codex arguments.");
}

pub(super) fn inspect(out: &mut Output, value: &Value) {
    let selected = &value["selection"];
    out.line(format!(
        "Context: {}:{} in {}",
        safe(text(selected, "kind")),
        safe(text(selected, "id")),
        safe(text(value, "project_id"))
    ));
    out.strings("Included subsystems", array(selected, "subsystems"));
    out.field("Selected work", &selected["work_item"]);
    out.field("Selection digest", &value["selection_digest"]);
    freshness_rows(out, value);
    let sources = array(value, "sources");
    out.line(format!("Sources: {} declared files/records", sources.len()));
    for source in sources.iter().take(MAX_ROWS) {
        out.line(format!(
            "  {} ({} bytes)",
            safe(text(source, "path")),
            scalar(&source["bytes"])
        ));
    }
    out.more(sources.len());
    let native = array(value, "native_artifacts");
    if native.is_empty() {
        out.line("Native artifacts: none selected.");
    } else {
        out.line(format!(
            "Native artifacts: {} validated locally; runtime remains unbound.",
            native.len()
        ));
        for artifact in native.iter().take(MAX_ROWS) {
            out.line(format!(
                "  {}: {}",
                safe(text(artifact, "projection")),
                safe(text(&artifact["bundle"], "manifest_sha256"))
            ));
        }
        out.more(native.len());
        out.line("Native launch requires explicit --proxy on the supported Codex route.");
    }
    let worktree = &value["worktree"];
    if !worktree.is_null() {
        if worktree.get("error").is_some() {
            out.line("Worker checkout unavailable:");
            out.issues(std::slice::from_ref(&worktree["error"]));
        } else {
            out.field("Worker checkout", &worktree["root"]);
            out.field("Worker branch", &worktree["branch"]);
            out.line(if yes(worktree, "existing") {
                "Uses the existing worker binding."
            } else {
                "The worker checkout will be created on launch."
            });
        }
    }
    launch_preview(out, value);
    out.line("To launch, repeat this command without --inspect. Keep the selected work ID, route, and explicit Codex arguments.");
}

pub(super) fn work_list(out: &mut Output, value: &Value) {
    let records = array(value, "records");
    out.line(format!(
        "Work items: {} ({})",
        records.len(),
        safe(text(value, "path"))
    ));
    if records.is_empty() {
        out.example(
            "No work items yet. Create one with astral work create TITLE --acceptance CRITERION.",
            "No work items yet; create one (replace TITLE and CRITERION)",
            &[
                "astral",
                "work",
                "create",
                "TITLE",
                "--acceptance",
                "CRITERION",
            ],
        );
    }
    for record in records.iter().take(MAX_ROWS) {
        let item = &record["item"];
        out.line(format!(
            "  {}  [{}] {}",
            safe(text(item, "id")),
            safe(text(item, "status")),
            safe(text(item, "title"))
        ));
        out.line(format!("    Digest: {}", safe(text(record, "digest"))));
    }
    out.more(records.len());
    out.example(
        "Update a status with astral work update ID --status STATUS --expected DIGEST.",
        "Update status (replace ID, STATUS, and DIGEST)",
        &[
            "astral",
            "work",
            "update",
            "ID",
            "--status",
            "STATUS",
            "--expected",
            "DIGEST",
        ],
    );
    out.line("STATUS: open, in_progress, blocked, or complete. Commit reviewed record changes with your code.");
}

pub(super) fn work_record(out: &mut Output, value: &Value) {
    let item = &value["item"];
    out.line(format!(
        "Work item {}: {}",
        safe(text(item, "id")),
        safe(text(item, "title"))
    ));
    out.field("Status", &item["status"]);
    out.field("Record digest", &value["digest"]);
    out.strings("Acceptance", array(item, "acceptance"));
    out.strings("Depends on", array(item, "depends_on"));
    out.strings("Evidence", array(item, "evidence"));
    for (label, key) in [("Result", "result"), ("Remaining", "remaining")] {
        out.field(label, &item[key]);
    }
    out.line("The work register was updated locally. Review and commit the change when ready.");
    out.command(
        "Inspect worker",
        &["astral", "status", "--work", text(item, "id")],
    );
}

pub(super) fn work_merge(out: &mut Output, value: &Value) {
    if text(value, "outcome") == "merged" {
        let count = text(value, "jsonl")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        out.line(format!("Work records merge cleanly ({count} records)."));
        out.line("No file was written. Repeat the same merge command with --json to retrieve the merged jsonl field; review it before replacing the register.");
    } else {
        out.line("Work-record merge needs review:");
        let conflicts = array(value, "conflicts");
        for conflict in conflicts.iter().take(MAX_ROWS) {
            out.line(format!(
                "  {}: {}",
                safe(text(conflict, "id")),
                safe(text(conflict, "kind"))
            ));
        }
        out.more(conflicts.len());
        out.line("Resolve these records across base, ours, and theirs, then rerun the merge. No partial merged file was produced.");
    }
}
