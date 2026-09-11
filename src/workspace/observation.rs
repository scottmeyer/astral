//! Read-only worker health observations. A successful ownership probe is released
//! before return and does not authorize a later operation without acquisition.

use super::{WorkspacePlan, WorktreeBinding, configured_root};
use crate::project::{Error, Result};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipObservation {
    Available,
    Busy,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct BindingObservation {
    pub work_id: String,
    pub selector: Option<String>,
    pub plan: Option<WorkspacePlan>,
    pub ownership: OwnershipObservation,
    pub issues: Vec<Error>,
}

impl WorktreeBinding {
    /// Observe an existing binding using trusted host configuration. Never creates
    /// a binding or retains its ownership lock.
    pub fn observe(
        source_root: &Path,
        project_id: &str,
        work_id: &str,
    ) -> Result<BindingObservation> {
        Self::observe_with_root(
            source_root,
            project_id,
            work_id,
            configured_root()?.as_deref(),
        )
    }

    pub fn observe_with_root(
        source_root: &Path,
        project_id: &str,
        work_id: &str,
        managed_root: Option<&Path>,
    ) -> Result<BindingObservation> {
        #[cfg(unix)]
        {
            unix::observe(source_root, project_id, work_id, managed_root)
        }
        #[cfg(not(unix))]
        {
            let _ = (source_root, project_id, work_id, managed_root);
            Err(super::unsupported())
        }
    }
}

#[cfg(unix)]
mod unix;
