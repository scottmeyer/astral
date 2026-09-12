//! Named knowledge references and literal document regions. Resolution never
//! guesses heading slugs, parses code symbols or treats prose as executable policy.
use super::freshness::{Declaration, Input, Observation, State};
use super::{Project, Result, SubsystemManifest, error, identifier, join};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Rule,
    Decision,
    Observation,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    /// Relative to the subsystem, and already selected by readme/rules/decisions.
    pub document: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_fingerprint: Option<String>,
}

fn name(value: &str) -> Result<()> {
    identifier(value)?;
    if value.len() > 128 {
        return Err(error(
            "KNOWLEDGE_ID_LIMIT",
            "knowledge reference components must be at most 128 bytes",
        ));
    }
    Ok(())
}

impl Entry {
    pub(super) fn validate(&self, subsystem: &SubsystemManifest) -> Result<()> {
        name(&subsystem.id)?;
        name(&self.id)?;
        if self.title.trim().is_empty() || self.title.len() > 1024 {
            return Err(error(
                "KNOWLEDGE_TITLE",
                "knowledge title must be nonempty and at most 1024 bytes",
            ));
        }
        if !std::iter::once(&subsystem.readme)
            .chain(&subsystem.rules)
            .chain(&subsystem.decisions)
            .any(|p| p == &self.document)
        {
            return Err(error(
                "KNOWLEDGE_DOCUMENT",
                "knowledge document must already be selected by the subsystem readme, rules or decisions",
            ));
        }
        if let Some(region) = &self.region {
            name(region)?;
        }
        Declaration {
            inputs: self.inputs.clone(),
            reviewed_fingerprint: self.reviewed_fingerprint.clone(),
        }
        .validate()
    }
}

impl SubsystemManifest {
    pub(super) fn code_inputs(&self) -> impl Iterator<Item = &String> {
        self.freshness
            .iter()
            .flat_map(|f| &f.inputs)
            .chain(self.knowledge.iter().flat_map(|k| &k.inputs))
    }
    pub(super) fn clear_reviews(&mut self) {
        if let Some(f) = &mut self.freshness {
            f.reviewed_fingerprint = None;
        }
        for entry in &mut self.knowledge {
            entry.reviewed_fingerprint = None;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub subsystem: String,
    pub entry: String,
}
impl Reference {
    pub fn parse(value: &str) -> Result<Self> {
        let (subsystem, entry) = value
            .strip_prefix("knowledge:")
            .and_then(|v| v.split_once('/'))
            .ok_or_else(|| error("KNOWLEDGE_REFERENCE", "use knowledge:SUBSYSTEM/ENTRY"))?;
        name(subsystem)?;
        name(entry)?;
        Ok(Self {
            subsystem: subsystem.into(),
            entry: entry.into(),
        })
    }
    pub(super) fn key(&self) -> String {
        format!("{}/{}", self.subsystem, self.entry)
    }
}
impl std::fmt::Display for Reference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "knowledge:{}/{}", self.subsystem, self.entry)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Document {
    pub path: String,
    pub region: Option<String>,
    pub sha256: Option<String>,
    pub bytes: Option<usize>,
    pub error_code: Option<String>,
}

/// Markers are exact standalone lines, even inside Markdown code fences. No
/// inferred Markdown/HTML semantics. Returned bytes exclude the marker lines.
pub(super) fn extract<'a>(bytes: &'a [u8], region: Option<&str>) -> Result<&'a str> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| error("KNOWLEDGE_UTF8", "knowledge document must be UTF-8"))?;
    let selected = if let Some(region) = region {
        let begin = format!("<!-- astral:begin {region} -->");
        let end = format!("<!-- astral:end {region} -->");
        let mut start = None;
        let mut finish = None;
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            let content = line.trim_end_matches('\n').trim_end_matches('\r');
            if content == begin && start.replace(offset + line.len()).is_some() {
                return Err(error(
                    "KNOWLEDGE_REGION",
                    "knowledge region markers are duplicated",
                ));
            }
            if content == end && finish.replace(offset).is_some() {
                return Err(error(
                    "KNOWLEDGE_REGION",
                    "knowledge region markers are duplicated",
                ));
            }
            offset += line.len();
        }
        match (start, finish) {
            (Some(start), Some(finish)) if start <= finish => &text[start..finish],
            _ => {
                return Err(error(
                    "KNOWLEDGE_REGION",
                    "knowledge region markers are missing or out of order",
                ));
            }
        }
    } else {
        text
    };
    if selected.trim().is_empty() {
        return Err(error(
            "KNOWLEDGE_EMPTY",
            "knowledge content must be nonempty",
        ));
    }
    Ok(selected)
}

impl Project {
    pub(super) fn observe_knowledge(
        &self,
        contents: &BTreeMap<String, Vec<u8>>,
        inputs: &BTreeMap<String, Input>,
    ) -> Result<BTreeMap<String, Observation>> {
        let mut observations = BTreeMap::new();
        for (subsystem, s) in &self.subsystems {
            for entry in &s.manifest.knowledge {
                let path = join(&s.directory, &entry.document)?;
                let content = extract(&contents[&path], entry.region.as_deref());
                let document = Document {
                    path,
                    region: entry.region.clone(),
                    sha256: content
                        .as_ref()
                        .ok()
                        .map(|text| crate::hash(text.as_bytes())),
                    bytes: content.as_ref().ok().map(|text| text.len()),
                    error_code: content.as_ref().err().map(|e| e.code.into()),
                };
                let mut declaration = entry.clone();
                declaration.reviewed_fingerprint = None;
                let mut observed_inputs: Vec<_> =
                    entry.inputs.iter().map(|p| inputs[p].clone()).collect();
                observed_inputs.sort_by(|a, b| a.path.cmp(&b.path));
                let available = document.error_code.is_none()
                    && observed_inputs.iter().all(|i| i.error_code.is_none());
                let fingerprint = available.then(|| {
                    crate::fingerprint(&json!({
                        "schema_version": 1, "scope": "knowledge",
                        "project_id": self.manifest.id, "subsystem": subsystem,
                        "entry": declaration, "document": document, "inputs": observed_inputs,
                    }))
                });
                let state = State::compare(
                    fingerprint.as_deref(),
                    entry.reviewed_fingerprint.as_deref(),
                );
                observations.insert(
                    format!("{subsystem}/{}", entry.id),
                    Observation {
                        subsystem: subsystem.clone(),
                        knowledge: Some(entry.id.clone()),
                        document: Some(document),
                        state,
                        fingerprint,
                        reviewed_fingerprint: entry.reviewed_fingerprint.clone(),
                        inputs: observed_inputs,
                    },
                );
            }
        }
        Ok(observations)
    }

    fn knowledge_entry(&self, reference: &Reference) -> Result<&Entry> {
        self.subsystems
            .get(&reference.subsystem)
            .and_then(|s| {
                s.manifest
                    .knowledge
                    .iter()
                    .find(|k| k.id == reference.entry)
            })
            .ok_or_else(|| {
                error(
                    "UNKNOWN_KNOWLEDGE",
                    "knowledge reference is not declared in this project",
                )
            })
    }

    pub fn list_knowledge(&self, selector: &str) -> Result<Value> {
        let resolved = self.resolve(selector, None)?;
        let mut rows = Vec::new();
        for subsystem in &resolved.selection.subsystems {
            for entry in &self.subsystems[subsystem].manifest.knowledge {
                let reference = Reference {
                    subsystem: subsystem.clone(),
                    entry: entry.id.clone(),
                };
                rows.push(json!({"reference":reference.to_string(), "kind":entry.kind, "title":entry.title, "observation":self.freshness[&reference.key()]}));
            }
        }
        self.output(json!({"schema_version":1, "operation":"context_knowledge", "project_id":self.manifest.id, "selection":resolved.selection, "knowledge":rows}))
    }

    pub fn show_knowledge(&self, reference: &str) -> Result<Value> {
        let reference = Reference::parse(reference)?;
        let entry = self.knowledge_entry(&reference)?;
        let observation = &self.freshness[&reference.key()];
        let path = join(
            &self.subsystems[&reference.subsystem].directory,
            &entry.document,
        )?;
        // A valid document can still be recalled when a code input is unavailable.
        let text = extract(&self.contents[&path], entry.region.as_deref())?;
        self.output(json!({"schema_version":1, "operation":"context_knowledge_show", "project_id":self.manifest.id, "reference":reference.to_string(), "kind":entry.kind, "title":entry.title, "freshness":[observation], "text":text, "scope":"Explicitly referenced project prose; not executable authority or proof of current verification."}))
    }

    pub(super) fn knowledge_freshness(&self, reference: &str) -> Result<Value> {
        let reference = Reference::parse(reference)?;
        self.knowledge_entry(&reference)?;
        self.output(json!({"schema_version":1,"operation":"context_freshness","project_id":self.manifest.id,"reference":reference.to_string(),"freshness":[self.freshness[&reference.key()]],"scope":"This entry declaration, its exact document or marked region, and its explicit code inputs only. Core, other entries, work records and native history are outside this entry baseline."}))
    }
}
