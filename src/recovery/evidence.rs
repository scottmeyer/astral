//! Bounded private acknowledgements; no commands, credentials or opaque payloads.
use crate::project::{Error, Result};
use serde::{Deserialize, Serialize};

pub(super) fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PendingSave {
    pub projection: String,
    pub bundle_sha256: String,
    pub selection_digest: String,
    pub selected_bundle_sha256: Option<String>,
}

impl PendingSave {
    pub fn validate(&self) -> Result<()> {
        if self.projection.is_empty()
            || self.projection.len() > 256
            || self.projection == "."
            || self.projection == ".."
            || !self
                .projection
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || !digest(&self.bundle_sha256)
            || !digest(&self.selection_digest)
            || self
                .selected_bundle_sha256
                .as_deref()
                .is_some_and(|v| !digest(v))
        {
            return Err(Error {
                code: "WORKSPACE_METADATA",
                message: "invalid pending save evidence".into(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StagedWorker {
    pub thread_id: String,
    pub model: String,
    pub provider: String,
    pub selection_digest: String,
    pub seed_bundle_sha256: Option<String>,
    pub selected_bundle_sha256: Option<String>,
}

impl StagedWorker {
    pub fn validate(&self) -> Result<()> {
        // Reuse the destination metadata contract without recursively including
        // acknowledgement fields in the validation object.
        crate::workspace::WorkerMetadata {
            thread_id: Some(self.thread_id.clone()),
            model: Some(self.model.clone()),
            provider: Some(self.provider.clone()),
            selection_digest: Some(self.selection_digest.clone()),
            seed_bundle_sha256: self.seed_bundle_sha256.clone(),
            selected_bundle_sha256: self.selected_bundle_sha256.clone(),
            ..Default::default()
        }
        .validate()
    }
}
