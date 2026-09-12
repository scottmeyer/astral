//! Project manifests and inspection/launch data, independent of filesystem access.
//! Public types are re-exported from `project` to preserve their established paths.

use crate::native_bundle::{BundleSummary, NativeBundle};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub file_bytes: usize,
    pub native_payload_bytes: usize,
    pub total_bytes: usize,
    pub files: usize,
    pub entries: usize,
    pub graph_depth: usize,
    pub output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 1_048_576,
            native_payload_bytes: crate::native_bundle::MAX_PAYLOAD_BYTES,
            total_bytes: 16_777_216,
            files: 2_048,
            entries: 4_096,
            graph_depth: 64,
            output_bytes: 2_097_152,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub scope: String,
    pub runtime_bindings: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_status: Option<String>,
    pub id: String,
    pub name: String,
    pub description: String,
    pub core: String,
    pub projections: String,
    pub work_items: String,
    pub identity: Identity,
    pub subsystems: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubsystemManifest {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_status: Option<String>,
    pub id: String,
    pub purpose: String,
    pub readme: String,
    pub rules: Vec<String>,
    pub decisions: Vec<String>,
    pub work_items: Vec<String>,
    pub projection: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<super::freshness::Declaration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge: Vec<super::knowledge::Entry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub id: String,
    pub kind: String,
    pub scope: String,
    pub availability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionManifest {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_status: Option<String>,
    pub id: String,
    pub kind: String,
    pub subsystems: Vec<String>,
    pub handoff: String,
    pub native_payload_in_repository: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_bundle: Option<NativeBundleReference>,
    pub sources: Vec<Provenance>,
}

/// Repository-relative immutable artifact reference, never an executable locator.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBundleReference {
    pub manifest: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Open,
    InProgress,
    Blocked,
    Complete,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItem {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub status: WorkStatus,
    pub depends_on: Vec<String>,
    pub acceptance: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceHandle {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    /// When present, the hash covers this exact JSONL record, without its line ending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
}

/// Validated bytes are available locally; destination runtime binding is separate.
#[derive(Debug, Clone, Serialize)]
pub struct NativeArtifact {
    pub projection: String,
    pub availability: NativeAvailability,
    pub runtime_binding: NativeRuntimeBinding,
    pub manifest: SourceHandle,
    pub payload: SourceHandle,
    pub bundle: BundleSummary,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAvailability {
    Validated,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRuntimeBinding {
    Unbound,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Subsystem,
    Projection,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextSelection {
    pub kind: ContextKind,
    pub id: String,
    pub subsystems: Vec<String>,
    pub projection: Option<String>,
    pub work_item: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FreshDocument {
    pub source: SourceHandle,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FreshWorkItem {
    pub item: WorkItem,
    pub source: SourceHandle,
    /// Exact observed JSONL record, without its line ending.
    pub text: String,
}

/// Selected readable inputs for a fresh session, never a restored native checkpoint.
#[derive(Debug, Clone, Serialize)]
pub struct FreshContext {
    pub schema_version: u32,
    pub project_id: String,
    pub selection: ContextSelection,
    /// Also binds linked projection manifests checked by the fresh-only kind gate,
    /// so this can differ from the metadata-only inspection selection digest.
    pub selection_digest: String,
    /// Includes manifest fingerprints as well as selected document/record handles.
    pub sources: Vec<SourceHandle>,
    /// Core, selected subsystem/dependency documents, and an explicit handoff only.
    pub documents: Vec<FreshDocument>,
    pub work_item: Option<FreshWorkItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub freshness: Vec<super::freshness::Observation>,
}

/// Current selected documents and, when required, a separate immutable native
/// window. Documents never stand in for the required native state.
#[derive(Debug, Clone)]
pub struct ProjectLaunchContext {
    pub current_context: FreshContext,
    pub native: Option<NativeBundle>,
}
