use super::Worker;
use crate::project::Project;
use crate::workspace::{BindingStatus, OwnershipObservation, WorktreeBinding};
use std::path::Path;

pub(super) fn candidate(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.ends_with(".lock")
        && !value.contains("..")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

pub(super) fn observe(
    root: &Path,
    branch: Option<&str>,
    project: &Project,
    explicit: Option<&str>,
) -> Option<Worker> {
    let branch_work = branch.and_then(|b| b.strip_prefix("astral/"));
    let path_work = root.file_name().and_then(|p| p.to_str());
    let id = explicit
        .or(branch_work)
        .or(path_work)
        .filter(|id| candidate(id))?;
    let mut worker = Worker {
        work: id.into(),
        selector: None,
        ownership: "unknown".into(),
        recorded_thread: None,
        recorded_selection_digest: None,
        current_selection_digest: None,
        changed_since_staging: None,
        native_selection_changed: None,
        pending_operation: false,
        bound_to_current_checkout: false,
        diagnostics: Vec::new(),
    };
    let observed = match WorktreeBinding::observe(root, project.project_id(), id) {
        Ok(observed) => observed,
        Err(_) => {
            // A directory name alone is only a lookup hint; it does not establish
            // that a binding ought to exist, especially on an unborn repository.
            if explicit.is_none() && branch_work.is_none() {
                return None;
            }
            worker
                .diagnostics
                .push("WORKER_OBSERVATION_UNAVAILABLE".into());
            return Some(worker);
        }
    };
    if observed.plan.is_none()
        && observed.issues.is_empty()
        && explicit.is_none()
        && branch_work.is_none()
    {
        return None;
    }
    worker.selector = observed.selector.clone();
    worker.ownership = match observed.ownership {
        OwnershipObservation::Available => "available",
        OwnershipObservation::Busy => "busy",
        OwnershipObservation::Unknown => "unknown",
    }
    .into();
    if !observed.issues.is_empty() {
        worker
            .diagnostics
            .push("WORKER_BINDING_NEEDS_REVIEW".into());
    }
    let Some(plan) = observed.plan else {
        worker.diagnostics.push("WORKER_NOT_BOUND".into());
        return Some(worker);
    };
    worker.bound_to_current_checkout = plan.root == root;
    if !worker.bound_to_current_checkout {
        worker.diagnostics.push("WORKER_DIFFERENT_CHECKOUT".into());
        return Some(worker);
    }
    let metadata = plan.worker_metadata;
    worker.recorded_thread = metadata.thread_id.clone();
    worker.recorded_selection_digest = metadata.selection_digest.clone();
    worker.pending_operation = metadata.pending_save.is_some()
        || metadata.staged_worker.is_some()
        || metadata.staging_in_progress;
    // Lifetime ownership is normal during the session's own callback. Do not
    // turn an in-flight acknowledgement into a claim of interrupted operation.
    if observed.ownership != OwnershipObservation::Busy
        && (worker.pending_operation
            || plan.status != BindingStatus::Ready
            || !metadata.context_initialized)
    {
        worker
            .diagnostics
            .push("WORKER_RECOVERY_REVIEW_REQUIRED".into());
    }
    if let Some(selector) = &worker.selector {
        match project.launch_context(selector, Some(id)) {
            Ok(current) => {
                let digest = current.current_context.selection_digest;
                worker.changed_since_staging = metadata
                    .selection_digest
                    .as_deref()
                    .map(|old| old != digest);
                worker.current_selection_digest = Some(digest);
                let hash = current.native.as_ref().map(|n| n.summary().manifest_sha256);
                worker.native_selection_changed = metadata
                    .thread_id
                    .as_ref()
                    .map(|_| !metadata.accepts_selected_bundle(hash.as_deref()));
                if worker.changed_since_staging == Some(true) {
                    worker.diagnostics.push("WORKER_CONTEXT_CHANGED".into());
                }
                if worker.native_selection_changed == Some(true) {
                    worker
                        .diagnostics
                        .push("WORKER_NATIVE_SELECTION_CHANGED".into());
                }
            }
            Err(_) => worker.diagnostics.push("WORKER_CONTEXT_UNAVAILABLE".into()),
        }
    }
    Some(worker)
}
