//! Local observations for status/doctor. Nothing here starts a runtime or repairs state.

use crate::project::{Error, Project, Result, WorkStatus};
use crate::workspace::{BindingObservation, BindingStatus, OwnershipObservation, WorktreeBinding};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const MAX_PAGE_SIZE: usize = 64;
pub const DEFAULT_PAGE_SIZE: usize = 32;

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub guidance: &'static str,
}

impl From<Error> for Diagnostic {
    fn from(error: Error) -> Self {
        let guidance = guidance(error.code);
        Self {
            code: error.code.into(),
            message: error.message.chars().take(512).collect(),
            guidance,
        }
    }
}

fn guidance(code: &str) -> &'static str {
    match code {
        "WORKSPACE_CONTEXT_INCOMPLETE" => {
            "Inspect the retained worktree and binding before explicitly repairing initial context; do not overwrite the checkout."
        }
        "WORKSPACE_STAGE_INCOMPLETE" => {
            "Inspect the retained binding and Codex diagnostics to establish whether a thread was created; do not start another thread implicitly."
        }
        "WORKSPACE_SEED_CHANGED" => {
            "Preserve both native histories. Select a new work ID for an intentional new starting projection."
        }
        "WORKSPACE_BRANCH_MISMATCH" => {
            "Inspect the recorded branch and actual Git worktree association. Preserve local changes; do not reset or rebind automatically."
        }
        "WORKSPACE_INCOMPLETE" | "WORKSPACE_MISSING" => {
            "Inspect the retained Git worktree, private receipt and ownership lock before an explicit recovery operation."
        }
        "WORKSPACE_METADATA"
        | "WORKSPACE_MISMATCH"
        | "WORKSPACE_PRIVATE_STATE"
        | "WORKSPACE_UNSAFE_PATH"
        | "WORKSPACE_CHANGED" => {
            "Preserve the private binding and inspect its identity, permissions and retained artifacts. This command will not adopt or rewrite them."
        }
        "CONTEXT_PROJECT_MISMATCH" => {
            "Inspect the bound checkout's project identity before continuing this worker."
        }
        "WORK_NOT_FOUND" => "Choose a work ID listed in this checkout's validated work register.",
        _ => {
            "Resolve the reported local input or binding problem, then rerun doctor. Historical test results do not verify current state."
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextObservation {
    pub current_selection_digest: String,
    pub recorded_selection_digest: Option<String>,
    /// None means no staging baseline was recorded, not evidence of a change.
    pub changed_since_staging: Option<bool>,
    pub selected_bundle_sha256: Option<String>,
    pub recorded_saved_bundle_sha256: Option<String>,
    pub requires_proxy: bool,
}

#[derive(Debug, Serialize)]
pub struct WorkerStatus {
    pub work_id: String,
    pub title: String,
    pub work_status: WorkStatus,
    pub state: &'static str,
    pub binding: Option<BindingObservation>,
    pub context: Option<ContextObservation>,
    /// Literal suggested argv, never executed by status or doctor.
    pub resume_argv: Option<Vec<String>>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub schema_version: u32,
    pub root: PathBuf,
    pub project_id: Option<String>,
    pub total_work_items: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub workers: Vec<WorkerStatus>,
    pub diagnostics: Vec<Diagnostic>,
    pub observation_scope: &'static str,
}

impl StatusReport {
    pub fn needs_attention(&self) -> bool {
        !self.diagnostics.is_empty() || self.workers.iter().any(|w| !w.diagnostics.is_empty())
    }
}

fn error(code: &'static str, message: &str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

/// Inspect a page of current work records. Stored threads are not contacted.
pub fn collect(
    root: &Path,
    work: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<StatusReport> {
    if limit == 0 || limit > MAX_PAGE_SIZE || (work.is_some() && offset != 0) {
        return Err(error(
            "STATUS_ARGUMENTS",
            "limit must be 1..64; --work cannot be combined with a nonzero offset",
        ));
    }
    let root = root
        .canonicalize()
        .map_err(|_| error("INVALID_ROOT", "project root is unavailable"))?;
    let mut report = StatusReport {
        schema_version: 1,
        root: root.clone(),
        project_id: None,
        total_work_items: 0,
        offset,
        next_offset: None,
        workers: Vec::new(),
        diagnostics: Vec::new(),
        observation_scope: "Current checkout work records and their local bindings only; orphan bindings and receipt-only launches are not enumerated. Local observations are not atomic or ownership reservations. Runtime threads, provider compatibility, native recall, code freshness and test validity are not verified.",
    };
    let project = match Project::load(&root) {
        Ok(project) => project,
        Err(error) => {
            report.diagnostics.push(error.into());
            return Ok(report);
        }
    };
    report.project_id = Some(project.project_id().into());
    let records = match crate::work_records::read(&root) {
        Ok(records) => records,
        Err(e) => {
            report.diagnostics.push(error(e.code, e.message).into());
            return Ok(report);
        }
    };
    report.total_work_items = records.records.len();
    let selected: Vec<_> = if let Some(work) = work {
        match records.records.iter().find(|r| r.item.id == work) {
            Some(record) => vec![record],
            None => {
                report.diagnostics.push(
                    error(
                        "WORK_NOT_FOUND",
                        "work ID is absent from the current work register",
                    )
                    .into(),
                );
                return Ok(report);
            }
        }
    } else {
        let end = offset.saturating_add(limit).min(records.records.len());
        if end < records.records.len() {
            report.next_offset = Some(end);
        }
        records.records.iter().skip(offset).take(limit).collect()
    };
    for record in selected {
        let mut worker = WorkerStatus {
            work_id: record.item.id.clone(),
            title: record.item.title.chars().take(256).collect(),
            work_status: record.item.status.clone(),
            state: "unbound",
            binding: None,
            context: None,
            resume_argv: None,
            diagnostics: Vec::new(),
        };
        match WorktreeBinding::observe(&root, project.project_id(), &record.item.id) {
            Ok(observation) => {
                worker
                    .diagnostics
                    .extend(observation.issues.iter().cloned().map(Diagnostic::from));
                if let Some(plan) = &observation.plan {
                    let metadata = &plan.worker_metadata;
                    if observation.ownership == OwnershipObservation::Busy
                        && worker.diagnostics.is_empty()
                    {
                        worker.state = "owned";
                    } else if plan.status != BindingStatus::Ready && worker.diagnostics.is_empty() {
                        worker.diagnostics.push(
                            error(
                                "WORKSPACE_INCOMPLETE",
                                "worktree creation did not reach ready state",
                            )
                            .into(),
                        );
                    } else if worker.diagnostics.is_empty() {
                        if !metadata.context_initialized {
                            worker.diagnostics.push(
                                error(
                                    "WORKSPACE_CONTEXT_INCOMPLETE",
                                    "initial worker context was not recorded as installed",
                                )
                                .into(),
                            );
                        }
                        if metadata.staging_in_progress {
                            worker.diagnostics.push(
                                error(
                                    "WORKSPACE_STAGE_INCOMPLETE",
                                    "initial thread creation has an unknown outcome",
                                )
                                .into(),
                            );
                        }
                        if observation.ownership != OwnershipObservation::Busy {
                            inspect_context(&mut worker, &observation, project.project_id());
                        }
                    }
                    if worker.diagnostics.is_empty() {
                        worker.state = if observation.ownership == OwnershipObservation::Busy {
                            "owned"
                        } else if metadata.thread_id.is_some() {
                            "recorded"
                        } else {
                            "not_staged"
                        };
                    }
                }
                worker.binding = Some(observation);
            }
            Err(error) => worker.diagnostics.push(error.into()),
        }
        if !worker.diagnostics.is_empty() {
            worker.state = "attention";
            worker.resume_argv = None;
        }
        report.workers.push(worker);
    }
    Ok(report)
}

fn inspect_context(worker: &mut WorkerStatus, observation: &BindingObservation, project_id: &str) {
    let (Some(plan), Some(selector)) = (&observation.plan, &observation.selector) else {
        return;
    };
    let selected = Project::load(&plan.root).and_then(|p| {
        if p.project_id() != project_id {
            return Err(error(
                "CONTEXT_PROJECT_MISMATCH",
                "bound checkout has a different project identity",
            ));
        }
        p.launch_context(selector, Some(&worker.work_id))
    });
    match selected {
        Ok(selected) => {
            let metadata = &plan.worker_metadata;
            let bundle = selected
                .native
                .as_ref()
                .map(|b| b.summary().manifest_sha256.clone());
            if !metadata.accepts_selected_bundle(bundle.as_deref()) {
                worker.diagnostics.push(
                    error(
                        "WORKSPACE_SEED_CHANGED",
                        "selected native history differs from the recorded worker selection",
                    )
                    .into(),
                );
            }
            let requires_proxy = selected.native.is_some()
                || metadata.requires_tool_rebinding
                || metadata.seed_bundle_sha256.is_some()
                || metadata.saved_bundle_sha256.is_some();
            // Match the launcher's local preflight. Initial native import takes
            // its runtime identity from the selected bundle instead.
            if requires_proxy
                && !(metadata.thread_id.is_none() && selected.native.is_some())
                && (metadata.model.is_none() || metadata.provider.is_none())
            {
                worker.diagnostics.push(
                    error(
                        "WORKSPACE_METADATA",
                        "native continuation is missing its recorded model or provider",
                    )
                    .into(),
                );
            }
            let digest = selected.current_context.selection_digest;
            let changed = metadata
                .selection_digest
                .as_deref()
                .map(|old| old != digest);
            worker.context = Some(ContextObservation {
                current_selection_digest: digest,
                recorded_selection_digest: metadata.selection_digest.clone(),
                changed_since_staging: changed,
                selected_bundle_sha256: bundle,
                recorded_saved_bundle_sha256: metadata.saved_bundle_sha256.clone(),
                requires_proxy,
            });
            if observation.ownership == OwnershipObservation::Available
                && worker.diagnostics.is_empty()
            {
                // These are suggestions from validated local identity, not host permissions.
                let mut argv = vec![
                    "astral".into(),
                    "--root".into(),
                    plan.root.to_string_lossy().into_owned(),
                    "project".into(),
                    selector.clone(),
                    "--work".into(),
                    worker.work_id.clone(),
                ];
                if requires_proxy {
                    argv.push("--proxy".into());
                }
                worker.resume_argv = Some(argv);
            }
        }
        Err(error) => worker.diagnostics.push(error.into()),
    }
}

/// Human output escapes control characters from all repository-controlled text.
pub fn render(report: &StatusReport) -> String {
    let safe = |text: &str| {
        text.chars()
            .flat_map(char::escape_debug)
            .collect::<String>()
    };
    let mut out = format!("Astral status: {}\n", safe(&report.root.to_string_lossy()));
    for issue in &report.diagnostics {
        out.push_str(&format!(
            "{}: {}\n  {}\n",
            safe(&issue.code),
            safe(&issue.message),
            issue.guidance
        ));
    }
    for worker in &report.workers {
        out.push_str(&format!(
            "{}  {}  {}\n",
            safe(&worker.work_id),
            worker.state,
            safe(&worker.title)
        ));
        if let Some(observation) = &worker.binding {
            if let Some(plan) = &observation.plan {
                out.push_str(&format!(
                    "  {} at {}\n",
                    safe(&plan.branch),
                    safe(&plan.root.to_string_lossy())
                ));
                if let Some(thread) = &plan.worker_metadata.thread_id {
                    out.push_str(&format!(
                        "  Recorded thread: {} (runtime not checked)\n",
                        safe(thread)
                    ));
                }
            }
        }
        if let Some(context) = &worker.context {
            out.push_str(match context.changed_since_staging {
                Some(true) => "  Selected context changed since staging.\n",
                Some(false) => "  Selected context matches the recorded staging digest.\n",
                None => "  No staging digest was recorded; context change is unknown.\n",
            });
            if context.requires_proxy {
                out.push_str("  Continuation requires explicit --proxy.\n");
            }
            if let Some(saved) = &context.recorded_saved_bundle_sha256 {
                out.push_str(&format!("  Last recorded export: {}\n", safe(saved)));
            }
        }
        for issue in &worker.diagnostics {
            out.push_str(&format!(
                "  {}: {}\n    {}\n",
                safe(&issue.code),
                safe(&issue.message),
                issue.guidance
            ));
        }
        if worker.state == "owned" {
            out.push_str("  Another Astral owner was observed; wait for it to finish.\n");
        }
        if let Some(argv) = &worker.resume_argv {
            out.push_str(&format!(
                "  Suggested continuation argv (escaped): {argv:?}\n"
            ));
        }
    }
    if let Some(offset) = report.next_offset {
        out.push_str(&format!("More work records: use --offset {offset}.\n"));
    }
    out.push_str("Observation only; recorded threads, historical checks and provider execution are not verified.\n");
    out
}
