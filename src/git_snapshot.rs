//! Exact read-only `.astral` snapshots from the candidate index or HEAD tree.
//! Object reads are lazy and use Project's existing declared-input budgets.
mod context;
mod entries;
mod index_file;

use crate::project::{Error, Limits, Project, ProjectSource, Result};
use context::{Context, Evidence};
use entries::Entry;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const MAX_ENTRIES: usize = 16_384;
pub const MAX_METADATA_BYTES: usize = 4_194_304;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotKind {
    Index,
    Head,
}

#[derive(Clone)]
pub struct GitSnapshot {
    kind: SnapshotKind,
    context: Context,
    entries: BTreeMap<String, Entry>,
    head: Option<String>,
    branch: Option<String>,
    digest: String,
    evidence: Evidence,
}
impl std::fmt::Debug for GitSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitSnapshot")
            .field("kind", &self.kind)
            .field("digest", &self.digest)
            .field("entries", &self.entries.len())
            .finish()
    }
}

fn error(code: &'static str, message: &'static str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

impl GitSnapshot {
    pub fn capture(root: &Path, kind: SnapshotKind) -> Result<Self> {
        Self::capture_context(Context::discover(root)?, kind)
    }
    fn capture_context(context: Context, kind: SnapshotKind) -> Result<Self> {
        let before = context.evidence(kind)?;
        let head = context.optional(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])?;
        if head.as_deref().is_some_and(|h| !entries::oid(h)) {
            return Err(error("SNAPSHOT_FORMAT", "invalid HEAD object identifier"));
        }
        let branch = context.optional(&["symbolic-ref", "--quiet", "--short", "HEAD"])?;
        let raw = match kind {
            SnapshotKind::Index => context.read(
                &[
                    "ls-files",
                    "--stage",
                    "--debug",
                    "--full-name",
                    "-z",
                    "--",
                    ".astral",
                ],
                MAX_METADATA_BYTES,
            )?,
            SnapshotKind::Head => match head.as_deref() {
                Some(head) => context.read(
                    &["ls-tree", "-r", "-z", "--full-tree", head, "--", ".astral"],
                    MAX_METADATA_BYTES,
                )?,
                None => Vec::new(),
            },
        };
        let entries = match kind {
            SnapshotKind::Index => entries::index(&raw)?,
            SnapshotKind::Head => entries::tree(&raw)?,
        };
        let after = context.evidence(kind)?;
        if before != after {
            return Err(error(
                "SNAPSHOT_CHANGED",
                "Git context or index changed during snapshot capture",
            ));
        }
        let digest = crate::hash(
            &serde_json::to_vec(&(
                kind,
                &context.root,
                &context.common_dir,
                &context.git_dir,
                &head,
                &branch,
                &entries,
            ))
            .expect("snapshot metadata serializes"),
        );
        let snapshot = Self {
            kind,
            context,
            entries,
            head,
            branch,
            digest,
            evidence: after,
        };
        snapshot.context.verify_identity()?;
        if snapshot.head
            != snapshot
                .context
                .optional(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])?
            || snapshot.branch
                != snapshot
                    .context
                    .optional(&["symbolic-ref", "--quiet", "--short", "HEAD"])?
        {
            return Err(error(
                "SNAPSHOT_CHANGED",
                "HEAD or branch changed during snapshot capture",
            ));
        }
        Ok(snapshot)
    }
    pub fn kind(&self) -> SnapshotKind {
        self.kind
    }
    pub fn root(&self) -> &Path {
        &self.context.root
    }
    pub fn common_dir(&self) -> &Path {
        &self.context.common_dir
    }
    pub fn git_dir(&self) -> &Path {
        &self.context.git_dir
    }
    pub fn head(&self) -> Option<&str> {
        self.head.as_deref()
    }
    /// Short local branch name; None for a detached HEAD.
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }
    pub fn has_project(&self) -> bool {
        self.contains(".astral/project.toml")
    }
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
    pub fn load_project(&self) -> Result<Project> {
        self.load_project_with_limits(Limits::default())
    }
    pub fn load_project_with_limits(&self, limits: Limits) -> Result<Project> {
        self.verify_unchanged()?;
        let result = Project::load_source(self, limits);
        self.verify_unchanged()?;
        result
    }
    /// Recheck the captured repository/index environment and evidence. Detects
    /// index replacement and changes outside `.astral`, not only selected blobs.
    pub fn verify_unchanged(&self) -> Result<()> {
        self.context.verify_environment()?;
        let current = Self::capture_context(self.context.clone(), self.kind)?;
        if self.evidence != current.evidence || self.digest != current.digest {
            return Err(error(
                "SNAPSHOT_CHANGED",
                "Git snapshot evidence changed; capture the current candidate again",
            ));
        }
        Ok(())
    }
}

impl ProjectSource for GitSnapshot {
    fn read_blob(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        let entry = self.entries.get(path).ok_or_else(|| {
            error(
                "PATH_UNAVAILABLE",
                "declared file is absent from the Git snapshot",
            )
        })?;
        let size = self.context.read(&["cat-file", "-s", &entry.oid], 128)?;
        let size: usize = std::str::from_utf8(&size)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| error("SNAPSHOT_FORMAT", "invalid Git object size"))?;
        if size > limit {
            return Err(error(
                "LIMIT_EXCEEDED",
                "snapshot blob exceeds file or aggregate byte budget",
            ));
        }
        let bytes = self
            .context
            .read(&["cat-file", "blob", &entry.oid], limit)?;
        if bytes.len() != size {
            return Err(error(
                "SNAPSHOT_CHANGED",
                "Git object bytes changed during read",
            ));
        }
        Ok(bytes)
    }
    fn directory(&self, path: &str, limit: usize) -> Result<Vec<String>> {
        if self.entries.contains_key(path) {
            return Err(error("INVALID_FILE_TYPE", "snapshot directory is a file"));
        }
        let prefix = format!("{path}/");
        let mut names = BTreeSet::new();
        for key in self.entries.keys().filter_map(|p| p.strip_prefix(&prefix)) {
            if let Some(name) = key.split('/').next() {
                names.insert(name.to_owned());
                if names.len() > limit {
                    return Err(error("LIMIT_EXCEEDED", "snapshot directory entry count"));
                }
            }
        }
        if names.is_empty() {
            return Err(error(
                "PATH_UNAVAILABLE",
                "declared directory is absent from the Git snapshot",
            ));
        }
        Ok(names.into_iter().collect())
    }
}
