//! Shared read-only context observations for commands and advisory lifecycle hooks.
//! Events never authorize changes to a worker's acknowledgement or Git state.
mod binding;

use crate::git_snapshot::{GitSnapshot, SnapshotKind};
use crate::project::{Project, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Index,
    Head,
    Worktree,
}
impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Head => "head",
            Self::Worktree => "worktree",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    pub availability: String,
    pub project_id: Option<String>,
    pub digest: Option<String>,
    pub observed_files: usize,
    pub error_code: Option<String>,
}
impl Context {
    fn observe(present: bool, project: &Result<Project>) -> Self {
        match project {
            Ok(project) => {
                let handles: Vec<_> = project.observed_sources().collect();
                Self {
                    availability: "valid".into(),
                    project_id: Some(project.project_id().into()),
                    digest: Some(crate::hash(
                        &serde_json::to_vec(&handles).expect("source handles serialize"),
                    )),
                    observed_files: handles.len(),
                    error_code: None,
                }
            }
            Err(error) => Self {
                availability: if present { "invalid" } else { "absent" }.into(),
                project_id: None,
                digest: None,
                observed_files: 0,
                error_code: Some(error.code.into()),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worker {
    pub work: String,
    pub selector: Option<String>,
    pub ownership: String,
    pub recorded_thread: Option<String>,
    pub recorded_selection_digest: Option<String>,
    pub current_selection_digest: Option<String>,
    pub changed_since_staging: Option<bool>,
    pub native_selection_changed: Option<bool>,
    pub pending_operation: bool,
    pub bound_to_current_checkout: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub operation: String,
    pub scope: Scope,
    pub root: PathBuf,
    pub repository_id: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub index_snapshot_sha256: String,
    pub head_snapshot_sha256: String,
    pub index: Context,
    pub committed: Context,
    pub worktree: Context,
    pub worker: Option<Worker>,
    pub diagnostics: Vec<String>,
    pub fingerprint: String,
    pub observation_scope: String,
}

/// Explicit checks write no observation cache. Callbacks may independently cache
/// a notice fingerprint; that cache is never read as proof of current state.
pub fn check(root: &Path, scope: Scope, work: Option<&str>) -> Result<Report> {
    if work.is_some_and(|id| !binding::candidate(id)) {
        return Err(crate::project::Error {
            code: "INVALID_WORK_ID",
            message: "work identifier is invalid".into(),
        });
    }
    let index = GitSnapshot::capture(root, SnapshotKind::Index)?;
    let head = GitSnapshot::capture(root, SnapshotKind::Head)?;
    // This observation owns both snapshots and verifies them again before
    // returning any report. Load their captured blobs directly here, avoiding
    // the standalone loader's additional before/after recaptures for each
    // project. The final full checks below cover this entire observation,
    // including changes during blob loading and worker inspection.
    let index_project = Project::load_source(&index, Default::default());
    let head_project = Project::load_source(&head, Default::default());
    let working_project = Project::load(index.root());
    let present = std::fs::symlink_metadata(index.root().join(".astral/project.toml"))
        .map_or_else(|e| e.kind() != std::io::ErrorKind::NotFound, |_| true);
    let mut report = Report {
        schema_version: 1,
        operation: "lifecycle_check".into(),
        scope,
        root: index.root().into(),
        repository_id: crate::hash(index.common_dir().as_os_str().as_encoded_bytes()),
        head: index.head().map(Into::into),
        branch: index.branch().map(Into::into),
        index_snapshot_sha256: index.digest().into(),
        head_snapshot_sha256: head.digest().into(),
        index: Context::observe(index.has_project(), &index_project),
        committed: Context::observe(head.has_project(), &head_project),
        worktree: Context::observe(present, &working_project),
        worker: None,
        diagnostics: Vec::new(),
        fingerprint: String::new(),
        observation_scope: "Bounded current Git index, HEAD and declared working context; no runtime, test, semantic freshness or completed-operation verification. Worker lookup uses an explicit ID or a validated current path/branch candidate, not a session ID. Hook events and notification caches are not authority.".into(),
    };
    for (context, code) in [
        (&report.index, "INDEX_CONTEXT_INVALID"),
        (&report.committed, "COMMITTED_CONTEXT_INVALID"),
        (&report.worktree, "WORKTREE_CONTEXT_INVALID"),
    ] {
        if context.availability == "invalid" {
            report.diagnostics.push(code.into());
        }
    }
    if report.index.digest != report.worktree.digest {
        report
            .diagnostics
            .push("UNSTAGED_CONTEXT_DIFFERENCE".into());
    }
    if report.index.digest != report.committed.digest {
        report
            .diagnostics
            .push("UNCOMMITTED_CONTEXT_DIFFERENCE".into());
    }
    if report.head.is_none() {
        report.diagnostics.push("UNBORN_HEAD".into());
    }
    if let Ok(project) = &working_project {
        report.worker = binding::observe(index.root(), index.branch(), project, work);
        if let Some(worker) = &report.worker {
            report.diagnostics.extend(worker.diagnostics.clone());
        }
    } else if work.is_some() {
        report.diagnostics.push("WORKER_CONTEXT_UNAVAILABLE".into());
    }
    // Protect the snapshot claims independently from the per-file working-tree
    // observations. A later edit still requires a fresh observation.
    index.verify_unchanged()?;
    head.verify_unchanged()?;
    report.diagnostics.sort();
    report.diagnostics.dedup();
    report.fingerprint = crate::hash(&serde_json::to_vec(&report).expect("report serializes"));
    Ok(report)
}

/// Only fixed diagnostic text enters a Codex developer-message hook response.
/// Never interpolate project prose, paths, event text or parser error bodies.
pub fn notice(report: &Report, claimed_session: Option<&str>) -> String {
    if let (Some(session), Some(worker)) = (claimed_session, &report.worker) {
        if worker
            .recorded_thread
            .as_deref()
            .is_some_and(|id| id != session)
        {
            return "Astral advisory: this session identity does not match the recorded worker. Inspect astral status before continuing; no session or work binding was changed.".into();
        }
    }
    let selected = match report.scope {
        Scope::Index => &report.index,
        Scope::Head => &report.committed,
        Scope::Worktree => &report.worktree,
    };
    if selected.availability != "valid" {
        return format!(
            "Astral advisory: {} project context is unavailable or invalid. Inspect astral lifecycle check. This observation does not block the operation or repair context.",
            report.scope.name()
        );
    }
    if report
        .worker
        .as_ref()
        .is_some_and(|w| !w.bound_to_current_checkout || !w.diagnostics.is_empty())
    {
        return "Astral advisory: the recorded worker or selected context needs review. Inspect astral status and astral recover before an explicit continuation. No receipt was changed and no operation was retried.".into();
    }
    if !report.diagnostics.is_empty() {
        return "Astral advisory: committed, staged or working context differs. Inspect astral lifecycle check before the next handoff. A commit is not a native save, and changed context invalidates assumptions about earlier verification.".into();
    }
    format!(
        "Astral advisory: {} project context validated against current local evidence. This does not verify tests, live sessions or native recall. Save, recovery and branch completion remain explicit.",
        report.scope.name()
    )
}
