//! Explicit, bounded review baselines. Equal bytes do not prove correctness.
mod review;
pub use review::review;

use super::{Project, Reader, Result, SourceHandle, error, join, relative};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Whole-project cap, independent of repository size. No recursive discovery.
pub const MAX_INPUTS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Unreviewed,
    Unchanged,
    NeedsReview,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Input {
    pub path: String,
    pub sha256: Option<String>,
    pub bytes: Option<usize>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub subsystem: String,
    pub state: State,
    pub fingerprint: Option<String>,
    pub reviewed_fingerprint: Option<String>,
    /// Includes inputs declared by the subsystem's dependency closure.
    pub inputs: Vec<Input>,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Declaration {
    pub(super) fn validate(&self) -> Result<()> {
        if self.inputs.is_empty() || self.inputs.len() > MAX_INPUTS {
            return Err(error(
                "FRESHNESS_INPUT_LIMIT",
                "freshness requires 1 to 64 explicit code file paths",
            ));
        }
        super::distinct(&self.inputs, "freshness.inputs")?;
        for path in &self.inputs {
            relative(path)?;
            if path.len() > 4096
                || path
                    .split('/')
                    .any(|p| p.eq_ignore_ascii_case(".git") || p.eq_ignore_ascii_case(".astral"))
            {
                return Err(error(
                    "FRESHNESS_INPUT_PATH",
                    "code inputs must be bounded repository-relative files outside .git and .astral",
                ));
            }
        }
        if self
            .reviewed_fingerprint
            .as_deref()
            .is_some_and(|v| !digest(v))
        {
            return Err(error(
                "FRESHNESS_REVIEW_INVALID",
                "reviewed_fingerprint must be a lowercase SHA-256 digest",
            ));
        }
        Ok(())
    }
}

impl Project {
    pub(super) fn observe_freshness(
        &self,
        reader: &mut Reader<'_>,
    ) -> Result<BTreeMap<String, Observation>> {
        let paths: BTreeSet<_> = self
            .subsystems
            .values()
            .filter_map(|s| s.manifest.freshness.as_ref())
            .flat_map(|f| f.inputs.iter())
            .collect();
        if paths.len() > MAX_INPUTS {
            return Err(error(
                "FRESHNESS_INPUT_LIMIT",
                "project declares more than 64 distinct freshness inputs",
            ));
        }
        let mut inputs = BTreeMap::new();
        for path in paths {
            // Same file/byte budgets, but no code retained in prompt documents.
            let observed = match reader.read(path) {
                Ok(bytes) => Input {
                    path: path.clone(),
                    sha256: Some(crate::hash(&bytes)),
                    bytes: Some(bytes.len()),
                    error_code: None,
                },
                Err(e) => Input {
                    path: path.clone(),
                    sha256: None,
                    bytes: None,
                    error_code: Some(e.code.into()),
                },
            };
            reader.contents.remove(path);
            inputs.insert(path.clone(), observed);
        }
        let mut observations = BTreeMap::new();
        for (id, subsystem) in &self.subsystems {
            let Some(declaration) = &subsystem.manifest.freshness else {
                continue;
            };
            let mut selected = BTreeSet::new();
            self.closure(id, &mut selected);
            let mut documents = BTreeMap::<String, &SourceHandle>::new();
            let core = join(".astral", &self.manifest.core)?;
            for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
                let path = join(&core, name)?;
                documents.insert(path.clone(), &self.sources[&path]);
            }
            let mut manifests = BTreeMap::new();
            let mut observed_inputs = BTreeMap::new();
            for dependency in selected {
                let s = &self.subsystems[&dependency];
                let mut manifest = s.manifest.clone();
                if let Some(f) = &mut manifest.freshness {
                    f.reviewed_fingerprint = None;
                    for path in &f.inputs {
                        observed_inputs.insert(path.clone(), inputs[path].clone());
                    }
                }
                for name in std::iter::once(&manifest.readme)
                    .chain(&manifest.rules)
                    .chain(&manifest.decisions)
                {
                    let path = join(&s.directory, name)?;
                    documents.insert(path.clone(), &self.sources[&path]);
                }
                manifests.insert(dependency, manifest);
            }
            let inputs: Vec<_> = observed_inputs.into_values().collect();
            let fingerprint = inputs.iter().all(|i| i.error_code.is_none()).then(|| {
                crate::fingerprint(&json!({
                    "schema_version": 1, "project": self.manifest, "subsystem": id,
                    "manifests": manifests, "documents": documents, "inputs": inputs,
                }))
            });
            let state = match (&fingerprint, &declaration.reviewed_fingerprint) {
                (None, _) => State::Unavailable,
                (Some(_), None) => State::Unreviewed,
                (Some(current), Some(reviewed)) if current == reviewed => State::Unchanged,
                _ => State::NeedsReview,
            };
            observations.insert(
                id.clone(),
                Observation {
                    subsystem: id.clone(),
                    state,
                    fingerprint,
                    reviewed_fingerprint: declaration.reviewed_fingerprint.clone(),
                    inputs,
                },
            );
        }
        Ok(observations)
    }

    pub fn freshness(&self) -> impl Iterator<Item = &Observation> {
        self.freshness.values()
    }

    pub(super) fn selected_freshness(&self, subsystems: &[String]) -> Vec<Observation> {
        subsystems
            .iter()
            .filter_map(|id| self.freshness.get(id).cloned())
            .collect()
    }

    pub fn inspect_freshness(&self, selector: &str) -> Result<Value> {
        let resolved = self.resolve(selector, None)?;
        self.output(json!({
            "schema_version": 1, "operation": "context_freshness", "project_id": self.manifest.id,
            "selection": resolved.selection, "freshness": self.selected_freshness(&resolved.selection.subsystems),
            "scope": "Declared code files, shared core documents and subsystem/dependency manifests and documents. Equal fingerprints mean unchanged since explicit review, not semantic correctness or current verification. No directory or glob discovery; unlisted files, work records, projection handoffs and native history are outside this review baseline."
        }))
    }
}
