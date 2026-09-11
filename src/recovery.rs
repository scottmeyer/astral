//! Explicit, previewed repair of destination-local worker bookkeeping.
//!
//! Recovery does not infer runtime success, repeat an RPC, or modify a checkout.
//! Only a recorded acknowledgement or a validated published artifact can justify
//! repairing an interrupted operation. Unknown outcomes remain blocked.

mod evidence;
pub use evidence::{PendingSave, StagedWorker};

use crate::project::{Error, Project, Result};
use crate::workspace::{BindingStatus, OwnershipObservation, WorktreeBinding};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

fn fail(code: &'static str, message: &str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Repair {
    CompleteCreation,
    AcceptCurrentContext,
    RecordStagedWorker,
    RecordPublishedSave,
}

#[derive(Debug, Serialize)]
pub struct RecoveryPlan {
    pub schema_version: u32,
    pub work: String,
    pub selector: Option<String>,
    pub binding: crate::workspace::RecoveryPreview,
    pub classification: String,
    pub repair: Option<Repair>,
    pub evidence: Value,
    pub proposed_metadata: Option<crate::workspace::WorkerMetadata>,
    pub issues: Vec<Error>,
    pub guidance: String,
    pub plan_sha256: String,
}

/// Inventory uses the repository's private binding root, and scans launch receipts
/// only when the caller explicitly supplies their trusted local root.
pub fn inventory(
    root: &Path,
    launch_root: Option<&Path>,
    offset: usize,
    limit: usize,
) -> Result<Value> {
    if limit == 0 || limit > 64 {
        return Err(fail("RECOVERY_PAGE", "limit must be between 1 and 64"));
    }
    let project = Project::load(root)?;
    let bindings = WorktreeBinding::inventory(root, project.project_id())?;
    let records: std::collections::BTreeSet<_> = project.work_ids().collect();
    let selected = bindings.work_ids.iter().skip(offset).take(limit);
    let mut observations = Vec::new();
    for id in selected {
        let observation = WorktreeBinding::recovery_preview(root, project.project_id(), id);
        observations.push(match observation {
            Ok(observation) => json!({"work":id,"in_current_register":records.contains(id.as_str()),"observation":observation}),
            Err(error) => json!({"work":id,"in_current_register":records.contains(id.as_str()),"error":error}),
        });
    }
    let total = bindings.work_ids.len();
    let next = offset.saturating_add(observations.len());
    let launches = launch_root
        .map(|path| {
            crate::launch_state::LaunchState::inventory_in(
                path,
                root,
                project.project_id(),
                offset,
                limit,
            )
        })
        .transpose()?;
    Ok(
        json!({"schema_version":1,"operation":"recovery_inventory","repository_id":bindings.repository_id,
        "total_bindings":total,"offset":offset,"next_offset":(next < total).then_some(next),
        "bindings":observations,"issues":bindings.issues,"launches":launches,
        "scope":"Explicit local observations only; unknown outcomes are retained. No runtime, repair, or filesystem discovery outside the selected roots."}),
    )
}

pub fn plan(root: &Path, work: &str) -> Result<RecoveryPlan> {
    build_plan(root, work, false)
}

fn build_plan(root: &Path, work: &str, owned: bool) -> Result<RecoveryPlan> {
    let project = Project::load(root)?;
    let mut binding = WorktreeBinding::recovery_preview(root, project.project_id(), work)?;
    // The caller owns this exact binding, then compares its receipt hash before
    // writing. Normalize only the ownership observation for repeatable planning.
    if owned {
        binding.observation.ownership = OwnershipObservation::Available;
    }
    let mut report = RecoveryPlan {
        schema_version: 1,
        work: work.into(),
        selector: binding.observation.selector.clone(),
        classification: "blocked".into(),
        repair: None,
        evidence: Value::Null,
        proposed_metadata: None,
        issues: Vec::new(),
        guidance: String::new(),
        plan_sha256: String::new(),
        binding,
    };
    classify(&mut report, project.project_id())?;
    report.plan_sha256 =
        crate::hash(&serde_json::to_vec(&report).expect("recovery plan serializes"));
    Ok(report)
}

fn classify(report: &mut RecoveryPlan, project_id: &str) -> Result<()> {
    if report.binding.observation.ownership != OwnershipObservation::Available {
        report.guidance = "Wait for the owner, or inspect the reported identity/storage error; recovery cannot override ownership.".into();
        return Ok(());
    }
    let Some(binding) = &report.binding.observation.plan else {
        report.classification = "unbound".into();
        report.guidance = "No validated binding was found; recovery does not adopt an orphan branch or create a worker.".into();
        return Ok(());
    };
    if binding.status != BindingStatus::Ready {
        if report.binding.creation_recoverable {
            report.classification = "creation_acknowledged".into();
            report.repair = Some(Repair::CompleteCreation);
            report.guidance = "Record the proven worktree creation as ready; preserve the checkout and re-inspect its initial context.".into();
        } else {
            report.classification = "creation_unproven".into();
            report.guidance = "The retained creation has no matching durable identity proof. Preserve its artifacts; recovery will not adopt it.".into();
        }
        return Ok(());
    }
    if !report.binding.observation.issues.is_empty() {
        report.guidance =
            "Resolve the reported identity or storage mismatch before recovery.".into();
        return Ok(());
    }
    let metadata = &binding.worker_metadata;
    let mut next = metadata.clone();
    let selector = report
        .selector
        .as_deref()
        .ok_or_else(|| fail("RECOVERY_SELECTOR", "missing stored selector"))?;
    let current = match Project::load(&binding.root)
        .and_then(|p| p.launch_context(selector, Some(&report.work)))
    {
        Ok(current) => current,
        Err(error) => {
            report.classification = "context_incomplete".into();
            report.issues.push(error);
            report.guidance = "Review and repair the retained checkout's declared context/work record with ordinary edits, then preview again. No source files are overwritten by recovery.".into();
            return Ok(());
        }
    };
    if current.current_context.project_id != project_id {
        report.issues.push(fail(
            "RECOVERY_PROJECT_MISMATCH",
            "the bound checkout's project identity changed",
        ));
        return Ok(());
    }
    let selected = current.native.as_ref().map(|v| v.summary().manifest_sha256);
    report.evidence = json!({"selection_digest":current.current_context.selection_digest,"selected_bundle_sha256":selected});
    if let Some(staged) = &metadata.staged_worker {
        if !metadata.staging_in_progress
            || metadata
                .thread_id
                .as_deref()
                .is_some_and(|id| id != staged.thread_id)
            || selected != staged.selected_bundle_sha256
        {
            report.issues.push(fail(
                "RECOVERY_STAGE_MISMATCH",
                "staged acknowledgement differs from the bound worker or selected native history",
            ));
            return Ok(());
        }
        next.thread_id = Some(staged.thread_id.clone());
        next.model = Some(staged.model.clone());
        next.provider = Some(staged.provider.clone());
        next.selection_digest = Some(staged.selection_digest.clone());
        next.seed_bundle_sha256 = staged.seed_bundle_sha256.clone();
        next.selected_bundle_sha256 = staged.selected_bundle_sha256.clone();
        next.selected_bundle_recorded = true;
        next.staging_in_progress = false;
        next.staged_worker = None;
        report.classification = "staging_acknowledged".into();
        report.repair = Some(Repair::RecordStagedWorker);
        report.guidance = "Record the acknowledged worker and context injection. This does not verify that the runtime is still available or repeat an injection.".into();
    } else if metadata.staging_in_progress {
        report.classification = "staging_unknown".into();
        report.guidance = "Thread creation or injection may have completed, but no durable acknowledgement was retained. Keep the binding and investigate runtime evidence; recovery will not clear this marker or create another thread.".into();
        return Ok(());
    } else if let Some(pending) = &metadata.pending_save {
        let publication = match Project::load(&binding.root)
            .and_then(|p| p.launch_context(&format!("projection:{}", pending.projection), None))
        {
            Ok(publication) => publication,
            Err(error) => {
                report.issues.push(error);
                report.classification = "publication_unknown".into();
                report.guidance = "The expected publication is unavailable. Retain the pending save and staging artifacts; recovery does not repeat compaction or discard this attempt.".into();
                return Ok(());
            }
        };
        let Some(bundle) = publication.native else {
            report.classification = "publication_unknown".into();
            report.guidance = "The named projection does not contain the pending native export. Retain the save attempt and inspect its publication artifacts.".into();
            return Ok(());
        };
        if bundle.summary().manifest_sha256 != pending.bundle_sha256
            || bundle.manifest().source.selection_sha256 != pending.selection_digest
            || metadata.thread_id.is_none()
            || (selected != pending.selected_bundle_sha256
                && selected.as_deref() != Some(pending.bundle_sha256.as_str()))
        {
            report.classification = "publication_mismatch".into();
            report.issues.push(fail("RECOVERY_PUBLICATION_MISMATCH", "the published artifact or selected native history differs from the retained save evidence"));
            report.guidance = "Preserve both the publication and receipt; a different bundle is not evidence of this save's completion.".into();
            return Ok(());
        }
        report.evidence["publication"] = json!({"projection":pending.projection,"bundle_sha256":pending.bundle_sha256,"source_selection_digest":pending.selection_digest});
        next.saved_bundle_sha256 = Some(pending.bundle_sha256.clone());
        next.selection_digest = Some(pending.selection_digest.clone());
        next.selected_bundle_sha256 = selected;
        next.selected_bundle_recorded = true;
        next.requires_tool_rebinding = true;
        next.pending_save = None;
        report.classification = "publication_validated".into();
        report.repair = Some(Repair::RecordPublishedSave);
        report.guidance = "Record the exact validated publication in the worker receipt; preserve every export and keep explicit proxy routing required.".into();
    } else if !metadata.context_initialized {
        if metadata.thread_id.is_some() {
            report.issues.push(fail(
                "RECOVERY_CONTEXT_MISMATCH",
                "an uninitialized context already has a worker",
            ));
            return Ok(());
        }
        next.context_initialized = true;
        report.classification = "context_ready_for_review".into();
        report.repair = Some(Repair::AcceptCurrentContext);
        report.guidance = "Accept the current validated context/work record as the initial context. This changes only the private receipt; inspect the retained checkout before applying.".into();
    } else {
        report.classification = "no_repair_needed".into();
        report.guidance = "No supported interrupted operation needs repair. Use status/doctor before continuing the recorded worker.".into();
        return Ok(());
    }
    next.validate()?;
    report.proposed_metadata = Some(next);
    Ok(())
}

pub fn apply(root: &Path, work: &str, expected: &str) -> Result<Value> {
    if !evidence::digest(expected) {
        return Err(fail(
            "RECOVERY_EXPECTED",
            "--apply requires the full preview plan SHA-256",
        ));
    }
    let before = plan(root, work)?;
    let project = Project::load(root)?;
    let selector = before.selector.as_deref().ok_or_else(|| {
        fail(
            "RECOVERY_UNAVAILABLE",
            "there is no validated binding to repair",
        )
    })?;
    if before.binding.observation.ownership != OwnershipObservation::Available {
        return Err(fail(
            "RECOVERY_OWNED",
            "recovery requires available ownership",
        ));
    }
    if before
        .binding
        .observation
        .plan
        .as_ref()
        .is_some_and(|b| b.worker_metadata.last_recovery_sha256.as_deref() == Some(expected))
    {
        let binding = WorktreeBinding::open_existing(root, project.project_id(), selector, work)?;
        if binding.worker_metadata().last_recovery_sha256.as_deref() == Some(expected) {
            return Ok(
                json!({"operation":"recover","work":work,"outcome":"already_recorded","plan_sha256":expected,"runtime_verified":false}),
            );
        }
    }
    if before.plan_sha256 != expected {
        return Err(fail(
            "RECOVERY_STALE",
            "recovery preview changed; inspect a new plan",
        ));
    }
    let repair = before.repair.as_ref().ok_or_else(|| {
        fail(
            "RECOVERY_UNPROVEN",
            "this outcome has no supported, proven repair",
        )
    })?;
    let receipt_sha = before
        .binding
        .receipt_sha256
        .as_deref()
        .ok_or_else(|| fail("RECOVERY_UNPROVEN", "missing receipt evidence"))?;
    if matches!(repair, Repair::CompleteCreation) {
        let _binding = WorktreeBinding::recover_creation_recorded(
            root,
            project.project_id(),
            work,
            receipt_sha,
            expected,
        )?;
        return Ok(
            json!({"operation":"recover","work":work,"outcome":"creation_recorded","inspect_context_next":true,"runtime_verified":false}),
        );
    }
    let mut binding = WorktreeBinding::open_existing(root, project.project_id(), selector, work)?;
    if binding.receipt_digest()? != receipt_sha {
        return Err(fail(
            "RECOVERY_STALE",
            "binding changed before ownership was acquired",
        ));
    }
    let under_lock = build_plan(root, work, true)?;
    if under_lock.plan_sha256 != expected {
        return Err(fail(
            "RECOVERY_STALE",
            "recovery inputs changed before repair",
        ));
    }
    let mut metadata = under_lock
        .proposed_metadata
        .ok_or_else(|| fail("RECOVERY_UNPROVEN", "no proven metadata update"))?;
    metadata.last_recovery_sha256 = Some(expected.into());
    binding.update_worker_metadata_expected(metadata, receipt_sha)?;
    Ok(
        json!({"operation":"recover","work":work,"outcome":"repaired","repair":repair,"plan_sha256":expected,"runtime_verified":false,"checkout_changed":false}),
    )
}
