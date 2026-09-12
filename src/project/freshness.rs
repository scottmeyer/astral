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
impl State {
    pub(super) fn compare(current: Option<&str>, reviewed: Option<&str>) -> Self {
        match (current, reviewed) {
            (None, _) => Self::Unavailable,
            (Some(_), None) => Self::Unreviewed,
            (Some(current), Some(reviewed)) if current == reviewed => Self::Unchanged,
            _ => Self::NeedsReview,
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<super::knowledge::Document>,
    pub state: State,
    pub fingerprint: Option<String>,
    pub reviewed_fingerprint: Option<String>,
    /// Entry inputs, or the subsystem's complete declared dependency closure.
    pub inputs: Vec<Input>,
}
impl Observation {
    pub fn reference(&self) -> String {
        match &self.knowledge {
            Some(entry) => format!("knowledge:{}/{entry}", self.subsystem),
            None => format!("subsystem:{}", self.subsystem),
        }
    }
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
            .flat_map(|s| s.manifest.code_inputs())
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
                manifest.clear_reviews();
                for path in manifest.code_inputs() {
                    observed_inputs.insert(path.clone(), inputs[path].clone());
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
            let state = State::compare(
                fingerprint.as_deref(),
                declaration.reviewed_fingerprint.as_deref(),
            );
            observations.insert(
                id.clone(),
                Observation {
                    subsystem: id.clone(),
                    knowledge: None,
                    document: None,
                    state,
                    fingerprint,
                    reviewed_fingerprint: declaration.reviewed_fingerprint.clone(),
                    inputs,
                },
            );
        }
        observations.extend(self.observe_knowledge(&reader.contents, &inputs)?);
        Ok(observations)
    }

    pub fn freshness(&self) -> impl Iterator<Item = &Observation> {
        self.freshness.values()
    }

    pub(super) fn selected_freshness(&self, subsystems: &[String]) -> Vec<Observation> {
        self.freshness
            .values()
            .filter(|observation| subsystems.contains(&observation.subsystem))
            .cloned()
            .collect()
    }

    pub fn inspect_freshness(&self, selector: &str) -> Result<Value> {
        if selector.starts_with("knowledge:") {
            return self.knowledge_freshness(selector);
        }
        let resolved = self.resolve(selector, None)?;
        self.output(json!({
            "schema_version": 1, "operation": "context_freshness", "project_id": self.manifest.id,
            "selection": resolved.selection, "freshness": self.selected_freshness(&resolved.selection.subsystems),
            "scope": "Subsystem baselines cover declared code files, shared core and subsystem/dependency manifests and documents. Knowledge entries cover only their declaration, selected text and explicit code inputs. Equal fingerprints mean unchanged since explicit review, not semantic correctness or current verification. No directory or glob discovery; unlisted files, work records, projection handoffs and native history are outside these baselines."
        }))
    }
}
