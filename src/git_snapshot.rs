//! Exact read-only `.astral` snapshots from the candidate index or HEAD tree.
//! Object reads are lazy and use Project's existing declared-input budgets.
mod context;
mod entries;
mod index_file;

use crate::project::{Error, Limits, Project, ProjectSource, Result};
use context::{Context, Evidence};
use entries::Entry;
use serde::Serialize;
use sha2::Digest;
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
        // Optional freshness inputs are exact files outside .astral. Never
        // enumerate the repository or fall back to working bytes. Full snapshot
        // evidence checks bracket the owning project/lifecycle observation.
        let external;
        let entry = if path.starts_with(".astral/") || path == ".astral" {
            self.entries.get(path)
        } else {
            // Context fixes GIT_LITERAL_PATHSPECS=1 and executes at the root.
            external = match self.kind {
                SnapshotKind::Index => entries::index_path(
                    &self.context.read(
                        &[
                            "ls-files",
                            "--stage",
                            "--debug",
                            "--full-name",
                            "-z",
                            "--",
                            path,
                        ],
                        MAX_METADATA_BYTES,
                    )?,
                    Some(path),
                )?,
                SnapshotKind::Head => match self.head.as_deref() {
                    Some(head) => entries::tree_path(
                        &self.context.read(
                            &["ls-tree", "-r", "-z", "--full-tree", head, "--", path],
                            MAX_METADATA_BYTES,
                        )?,
                        Some(path),
                    )?,
                    None => BTreeMap::new(),
                },
            };
            external.get(path)
        }
        .ok_or_else(|| {
            error(
                "PATH_UNAVAILABLE",
                "declared file is absent from the Git snapshot",
            )
        })?;
        // The process reader bounds allocation and stops oversized output, so
        // a separate size-query process is unnecessary. Verify the complete Git
        // object ID as well, including its type/size header, before accepting it.
        let bytes = self
            .context
            .read(&["cat-file", "blob", &entry.oid], limit)
            .map_err(|e| {
                if e.code == "SNAPSHOT_LIMIT" {
                    error(
                        "LIMIT_EXCEEDED",
                        "snapshot blob exceeds file or aggregate byte budget",
                    )
                } else {
                    e
                }
            })?;
        let header = format!("blob {}\0", bytes.len());
        let actual = if entry.oid.len() == 40 {
            // Git's SHA-1 object format is an identity format, not a new choice
            // of authentication algorithm or a checksum for native exports.
            let mut hash = sha1::Sha1::new();
            hash.update(header.as_bytes());
            hash.update(&bytes);
            format!("{:x}", hash.finalize())
        } else {
            let mut hash = sha2::Sha256::new();
            hash.update(header.as_bytes());
            hash.update(&bytes);
            format!("{:x}", hash.finalize())
        };
        if actual != entry.oid {
            return Err(error(
                "SNAPSHOT_OBJECT_ID",
                "Git object bytes do not match the captured object identifier",
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
