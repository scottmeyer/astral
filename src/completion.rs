//! Reviewed completion of a bound worker. Only explicit fast-forward apply mutates Git.
mod git;
mod review;

use crate::project::{Error, Project, Result};
use crate::workspace::{BindingStatus, WorktreeBinding};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub use review::{Artifact, Change, ContextReview, RecordReview};
pub const MAX_PATHS: usize = 4096;
pub const MAX_OUTPUT_BYTES: usize = 2_097_152;

#[derive(Debug, Clone)]
pub struct FinishRequest {
    pub root: PathBuf,
    pub work: String,
    pub into: String,
    /// Reviewed next starting selector; never rewrites a manifest or worker binding.
    pub context: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FinishState {
    FastForwardReady,
    ManualIntegrationRequired,
    AlreadyIntegrated,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinishPlan {
    pub schema_version: u32,
    pub operation: &'static str,
    pub plan_sha256: String,
    pub state: FinishState,
    pub ancestry_integrated: bool,
    pub target_root: PathBuf,
    pub target_branch: String,
    pub target_commit: String,
    pub worker_root: PathBuf,
    pub worker_branch: String,
    pub worker_commit: String,
    pub binding_base_commit: String,
    pub repository_id: String,
    pub work: String,
    pub binding_selector: String,
    pub worker_metadata_sha256: String,
    pub merge_base: String,
    pub changes: Vec<Change>,
    pub records: RecordReview,
    pub context: ContextReview,
    pub artifacts: Vec<Artifact>,
    pub blockers: Vec<&'static str>,
    pub next_context_argv: Option<Vec<String>>,
    pub guidance: Vec<&'static str>,
    pub observation_scope: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct FinishResult {
    pub schema_version: u32,
    pub operation: &'static str,
    pub applied: bool,
    pub reviewed_plan_sha256: String,
    pub plan: FinishPlan,
    pub work_status_updated: bool,
}

fn error(code: &'static str, message: impl Into<String>) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

fn acquire(request: &FinishRequest) -> Result<(PathBuf, String, WorktreeBinding)> {
    let root = request
        .root
        .canonicalize()
        .map_err(|_| error("INVALID_ROOT", "target checkout is unavailable"))?;
    git::target(&root, &request.into)?;
    let project = Project::load(&root)?;
    let observed = WorktreeBinding::observe(&root, project.project_id(), &request.work)?;
    if let Some(issue) = observed.issues.first() {
        return Err(issue.clone());
    }
    let selector = observed
        .selector
        .ok_or_else(|| error("FINISH_UNBOUND", "work ID has no existing worker binding"))?;
    if observed
        .plan
        .as_ref()
        .is_none_or(|p| p.status != BindingStatus::Ready)
    {
        return Err(error(
            "FINISH_INCOMPLETE_WORKER",
            "worker creation is incomplete; inspect or recover it before finishing",
        ));
    }
    let binding =
        WorktreeBinding::open_existing(&root, project.project_id(), &selector, &request.work)?;
    let metadata = binding.worker_metadata();
    if !metadata.context_initialized
        || metadata.staging_in_progress
        || metadata.thread_id.is_none()
        || metadata.staged_worker.is_some()
        || metadata.pending_save.is_some()
    {
        return Err(error(
            "FINISH_INCOMPLETE_WORKER",
            "worker staging or save is incomplete or uncertain; recover it before finishing",
        ));
    }
    Ok((root, selector, binding))
}

/// No-write plan, holding only the existing ownership lock during observation.
/// It neither contacts Codex nor reserves the target checkout after return.
pub fn plan(request: &FinishRequest) -> Result<FinishPlan> {
    let (root, selector, binding) = acquire(request)?;
    inspect_locked(request, &root, &selector, &binding)
}

fn inspect_locked(
    request: &FinishRequest,
    root: &Path,
    selector: &str,
    binding: &WorktreeBinding,
) -> Result<FinishPlan> {
    git::target(root, &request.into)?;
    git::clean(root)?;
    git::clean(binding.root())?;
    let target_commit = git::head(root)?;
    let worker_commit = git::head(binding.root())?;
    let target_tree = git::tree(root, &target_commit)?;
    let worker_tree = git::tree(root, &worker_commit)?;
    let base_tree = git::tree(root, binding.base_commit())?;
    git::ignored_collisions(root, &worker_tree)?;
    let merge_base = git::text(root, &["merge-base", &target_commit, &worker_commit])?;
    let common_tree = git::tree(root, &merge_base)?;
    let target_project = Project::load(root)?;
    let worker_project = Project::load(binding.root())?;
    git::committed_sources(root, &target_project, &target_tree)?;
    git::committed_sources(binding.root(), &worker_project, &worker_tree)?;
    if target_project.project_id() != worker_project.project_id() {
        return Err(error(
            "FINISH_PROJECT_MISMATCH",
            "worker and target project identities differ",
        ));
    }
    let selected = worker_project.launch_context(selector, Some(&request.work))?;
    let selected_hash = selected
        .native
        .as_ref()
        .map(|b| b.summary().manifest_sha256);
    if !binding
        .worker_metadata()
        .accepts_selected_bundle(selected_hash.as_deref())
    {
        return Err(error(
            "WORKSPACE_SEED_CHANGED",
            "worker native history differs from its recorded selection",
        ));
    }
    let integrated = merge_base == worker_commit;
    let fast_forward = merge_base == target_commit;
    let mut blockers = Vec::new();
    if !integrated && !fast_forward {
        blockers.push("DIVERGED_GIT_HISTORY");
    }
    let records = review::records(
        root,
        &merge_base,
        &target_commit,
        &worker_commit,
        &request.work,
    )?;
    if records.outcome != "merged" {
        blockers.push("WORK_RECORD_RECONCILIATION_REQUIRED");
    }
    let (context, artifacts) = review::contexts(
        &target_project,
        &worker_project,
        &target_tree,
        &worker_tree,
        selector,
        request.context.as_deref(),
        integrated,
    )?;
    if context.explicit_choice_required {
        blockers.push("NATIVE_CONTEXT_CHOICE_REQUIRED");
    }
    if context.selected_digest.is_none() {
        blockers.push("NEXT_CONTEXT_UNAVAILABLE");
    }
    if artifacts.iter().any(|a| !a.retained_in_result) {
        blockers.push("NATIVE_ARTIFACT_RETENTION_REQUIRED");
    }
    if fast_forward && target_tree.keys().any(|p| !worker_tree.contains_key(p)) {
        blockers.push("TRACKED_DELETIONS_REQUIRE_MANUAL_INTEGRATION");
    }
    if integrated
        && worker_tree.keys().chain(base_tree.keys()).any(|p| {
            p.as_str() != records.path
                && base_tree.get(p) != worker_tree.get(p)
                && worker_tree.get(p) != target_tree.get(p)
        })
    {
        blockers.push("WORKER_CHANGES_NOT_RETAINED_EXACTLY");
    }
    if integrated && records.target_status.is_none() {
        blockers.push("WORK_RECORD_NOT_RETAINED");
    }
    let state = if !blockers.is_empty() {
        FinishState::ManualIntegrationRequired
    } else if integrated {
        FinishState::AlreadyIntegrated
    } else {
        FinishState::FastForwardReady
    };
    let next_context_argv = if blockers.is_empty() {
        let mut argv = vec![
            "astral".into(),
            "--root".into(),
            git::path_text(root)?,
            "project".into(),
            context.selector.clone(),
        ];
        if context.selected_native_sha256.is_some() {
            argv.push("--proxy".into());
        }
        Some(argv)
    } else {
        None
    };
    let mut report = FinishPlan {
        schema_version: 1,
        operation: "finish_plan",
        plan_sha256: String::new(),
        state,
        ancestry_integrated: integrated,
        target_root: root.into(),
        target_branch: request.into.clone(),
        target_commit,
        worker_root: binding.root().into(),
        worker_branch: binding.branch().into(),
        worker_commit,
        binding_base_commit: binding.base_commit().into(),
        repository_id: binding.repository_id().into(),
        work: request.work.clone(),
        binding_selector: selector.into(),
        worker_metadata_sha256: crate::hash(
            &serde_json::to_vec(binding.worker_metadata()).expect("metadata serializes"),
        ),
        merge_base,
        changes: review::changes(&common_tree, &target_tree, &worker_tree, &records.path)?,
        records,
        context,
        artifacts,
        blockers,
        next_context_argv,
        guidance: vec![
            "Review committed code, readable decisions, work records and native context choice. Historical tests are not current verification.",
            "Divergence, deletions or unresolved evidence require ordinary Git merge/reconciliation. Finish never stages, commits, saves, resets, pushes or deletes branches/worktrees.",
            "After integration, explicitly update the work record with reviewed evidence using astral work update, then commit that update. Finish never marks work complete.",
            "The suggested next context starts a new session; it does not rebind the original worker. Replan after ordinary commits or merges.",
        ],
        observation_scope: "Committed object IDs and bounded local binding observations under a cooperating Astral worker lock; not an atomic Git transaction, live runtime check, test verification or semantic native-history reconciliation.",
    };
    let bytes = serde_json::to_vec(&report).expect("plan serializes");
    if bytes.len() > MAX_OUTPUT_BYTES.saturating_sub(64) {
        return Err(error(
            "FINISH_LIMIT",
            "completion plan exceeds output limit",
        ));
    }
    report.plan_sha256 = crate::hash(&bytes);
    git::unchanged(root, &request.into, &report.target_commit)?;
    git::unchanged(binding.root(), binding.branch(), &report.worker_commit)?;
    Ok(report)
}

/// Apply only the reviewed fast-forward. Ordinary Git/Codex writers must be
/// stopped by the caller; they do not honor the Astral worker ownership lock.
pub fn apply(request: &FinishRequest, expected_plan_sha256: &str) -> Result<FinishResult> {
    if expected_plan_sha256.len() != 64
        || !expected_plan_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            "FINISH_PLAN_HASH",
            "apply requires a lowercase SHA-256 from a reviewed plan",
        ));
    }
    let (root, selector, binding) = acquire(request)?;
    let reviewed = inspect_locked(request, &root, &selector, &binding)?;
    // A verified no-op may be observed after the prior command completed but its
    // caller lost the result. A stale hash never authorizes a new mutation.
    if reviewed.state == FinishState::AlreadyIntegrated {
        return Ok(FinishResult {
            schema_version: 1,
            operation: "finish_apply",
            applied: false,
            reviewed_plan_sha256: expected_plan_sha256.into(),
            plan: reviewed,
            work_status_updated: false,
        });
    }
    if reviewed.plan_sha256 != expected_plan_sha256 {
        return Err(error(
            "FINISH_PLAN_CHANGED",
            "completion evidence changed; review a new plan before applying",
        ));
    }
    if reviewed.state == FinishState::ManualIntegrationRequired {
        return Err(error(
            "FINISH_MANUAL_REQUIRED",
            "plan requires ordinary Git integration or explicit context reconciliation",
        ));
    }
    git::unchanged(&root, &request.into, &reviewed.target_commit)?;
    git::unchanged(binding.root(), binding.branch(), &reviewed.worker_commit)?;
    git::fast_forward(&root, &reviewed.worker_commit)?;
    let after = inspect_locked(request, &root, &selector, &binding).map_err(|_| error(
        "FINISH_VERIFY_REQUIRED", "integration may have completed; retain both checkouts and replan before any further operation"))?;
    if after.state != FinishState::AlreadyIntegrated {
        return Err(error(
            "FINISH_VERIFY_REQUIRED",
            "integration requires review; retain both checkouts and replan",
        ));
    }
    Ok(FinishResult {
        schema_version: 1,
        operation: "finish_apply",
        applied: true,
        reviewed_plan_sha256: expected_plan_sha256.into(),
        plan: after,
        work_status_updated: false,
    })
}
