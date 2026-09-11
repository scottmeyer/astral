//! Explicit recovery of private worker bookkeeping, with exact receipt CAS.
use super::{BindingObservation, FileIdentity, WorkerMetadata, WorktreeBinding, configured_root};
use crate::project::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_BINDING_INVENTORY: usize = 4096;
#[derive(Debug, Clone, Serialize)]
pub struct BindingInventory {
    pub repository_id: String,
    pub work_ids: Vec<String>,
    pub issues: Vec<Error>,
}
#[derive(Debug, Clone, Serialize)]
pub struct RecoveryPreview {
    pub observation: BindingObservation,
    pub receipt_sha256: Option<String>,
    pub creation_recoverable: bool,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreationProof {
    pub schema_version: u32,
    pub root_identity: FileIdentity,
    pub git_dir: PathBuf,
    pub git_identity: FileIdentity,
}
impl WorktreeBinding {
    pub fn inventory(source: &Path, project: &str) -> Result<BindingInventory> {
        Self::inventory_with_root(source, project, configured_root()?.as_deref())
    }
    pub fn inventory_with_root(
        source: &Path,
        project: &str,
        managed: Option<&Path>,
    ) -> Result<BindingInventory> {
        #[cfg(unix)]
        {
            unix::inventory(source, project, managed)
        }
        #[cfg(not(unix))]
        {
            let _ = (source, project, managed);
            Err(super::unsupported())
        }
    }
    pub fn recovery_preview(source: &Path, project: &str, work: &str) -> Result<RecoveryPreview> {
        Self::recovery_preview_with_root(source, project, work, configured_root()?.as_deref())
    }
    pub fn recovery_preview_with_root(
        source: &Path,
        project: &str,
        work: &str,
        managed: Option<&Path>,
    ) -> Result<RecoveryPreview> {
        #[cfg(unix)]
        {
            unix::preview(source, project, work, managed)
        }
        #[cfg(not(unix))]
        {
            let _ = (source, project, work, managed);
            Err(super::unsupported())
        }
    }
    pub fn recover_creation(
        source: &Path,
        project: &str,
        work: &str,
        expected_hash: &str,
    ) -> Result<Self> {
        Self::recover_creation_with_root(
            source,
            project,
            work,
            expected_hash,
            configured_root()?.as_deref(),
        )
    }
    pub fn recover_creation_with_root(
        source: &Path,
        project: &str,
        work: &str,
        expected_hash: &str,
        managed: Option<&Path>,
    ) -> Result<Self> {
        Self::recover_creation_recorded_with_root(
            source,
            project,
            work,
            expected_hash,
            None,
            managed,
        )
    }
    pub fn recover_creation_recorded(
        source: &Path,
        project: &str,
        work: &str,
        expected_hash: &str,
        plan_sha256: &str,
    ) -> Result<Self> {
        Self::recover_creation_recorded_with_root(
            source,
            project,
            work,
            expected_hash,
            Some(plan_sha256),
            configured_root()?.as_deref(),
        )
    }
    pub fn recover_creation_recorded_with_root(
        source: &Path,
        project: &str,
        work: &str,
        expected_hash: &str,
        plan_sha256: Option<&str>,
        managed: Option<&Path>,
    ) -> Result<Self> {
        #[cfg(unix)]
        {
            unix::recover_creation(source, project, work, expected_hash, plan_sha256, managed)
        }
        #[cfg(not(unix))]
        {
            let _ = (source, project, work, expected_hash, plan_sha256, managed);
            Err(super::unsupported())
        }
    }
    /// Exact stored bytes, including legacy omitted defaults; never reserialized.
    pub fn receipt_digest(&self) -> Result<String> {
        #[cfg(unix)]
        {
            self.storage.expected_hash.clone().ok_or_else(|| {
                super::fail(
                    "WORKSPACE_INCOMPLETE",
                    "owned receipt has not been published",
                )
            })
        }
        #[cfg(not(unix))]
        {
            Err(super::unsupported())
        }
    }
    /// Caller proves the repair; this primitive validates ownership/CAS and archives old bytes.
    pub fn update_worker_metadata_expected(
        &mut self,
        metadata: WorkerMetadata,
        expected_hash: &str,
    ) -> Result<()> {
        #[cfg(unix)]
        {
            unix::update_metadata(self, metadata, expected_hash)
        }
        #[cfg(not(unix))]
        {
            let _ = (metadata, expected_hash);
            Err(super::unsupported())
        }
    }
}
#[cfg(unix)]
mod unix;
