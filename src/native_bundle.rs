//! Validation of inert Codex native replay bundles, never runtime authority.
//!
//! V1 supports a conservative subset of Codex 0.154.0 Responses Lite items.
//! Byte/hash and boundary checks do not establish decryptability, account access,
//! semantic equivalence, or the exporter-attested completeness of source history.

use crate::project::{Error, Result};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_MANIFEST_BYTES: usize = 65_536;
pub const MAX_PAYLOAD_BYTES: usize = 8_388_608;
pub const MAX_ITEMS: usize = 4_096;
pub const MAX_PARENTS: usize = 32;
pub const MAX_JSON_DEPTH: usize = 64;
pub const MAX_JSON_NODES: usize = 131_072;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub format: String,
    pub payload: PayloadManifest,
    pub compatibility: Compatibility,
    pub source: Source,
    pub capture: Capture,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadManifest {
    pub file: String,
    pub sha256: String,
    pub bytes: usize,
    pub item_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    pub runtime: String,
    pub runtime_version: String,
    pub protocol: String,
    pub provider: String,
    pub model: String,
    pub requires_tool_rebinding: bool,
    pub identity_scope: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub project_id: String,
    #[serde(deserialize_with = "required_revision")]
    pub revision: Option<String>,
    pub dirty: bool,
    pub selection_sha256: String,
    pub history_sha256: String,
}

fn required_revision<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::deserialize(d)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub boundary: String,
    pub history_complete: bool,
    pub last_checkpoint_index: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BundleSummary {
    pub manifest_sha256: String,
    pub manifest: BundleManifest,
    pub checkpoint_count: usize,
    /// Historical source roles, not instructions installed at the destination.
    pub source_instruction_roles: Vec<String>,
    pub agent_message_count: usize,
}

#[derive(Clone)]
pub struct NativeBundle {
    manifest_bytes: Vec<u8>,
    payload_bytes: Vec<u8>,
    items: Vec<Box<RawValue>>,
    summary: BundleSummary,
}

impl fmt::Debug for NativeBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeBundle")
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

fn fail(code: &'static str, message: &'static str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

fn invalid() -> Error {
    fail(
        "NATIVE_INVALID_ITEM",
        "native item does not match the supported v1 shape",
    )
}

fn hex(value: &str, lengths: &[usize]) -> bool {
    lengths.contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl NativeBundle {
    pub fn validate(manifest_bytes: &[u8], payload_bytes: &[u8]) -> Result<Self> {
        if manifest_bytes.len() > MAX_MANIFEST_BYTES || payload_bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(fail("NATIVE_LIMIT", "native bundle exceeds byte limit"));
        }
        check_json(manifest_bytes)?;
        let manifest: BundleManifest = serde_json::from_slice(manifest_bytes)
            .map_err(|_| fail("NATIVE_INVALID_MANIFEST", "invalid native bundle manifest"))?;
        let manifest_sha256 = crate::hash(manifest_bytes);
        validate_manifest(&manifest, &manifest_sha256)?;
        if manifest.payload.bytes != payload_bytes.len()
            || manifest.payload.sha256 != crate::hash(payload_bytes)
        {
            return Err(fail(
                "NATIVE_INTEGRITY",
                "native payload byte count or digest mismatch",
            ));
        }
        check_json(payload_bytes)?;
        let items: BoundedArray<MAX_ITEMS> =
            serde_json::from_slice(payload_bytes).map_err(|e| {
                collection_error(
                    e,
                    "NATIVE_INVALID_PAYLOAD",
                    "native payload must be a JSON array",
                )
            })?;
        let items: Vec<Box<RawValue>> = items.0.into_iter().map(RawValue::to_owned).collect();
        if items.is_empty() || items.len() > MAX_ITEMS {
            return Err(fail(
                "NATIVE_LIMIT",
                "native payload item count is out of bounds",
            ));
        }
        if items.len() != manifest.payload.item_count {
            return Err(fail(
                "NATIVE_INTEGRITY",
                "native payload item count mismatch",
            ));
        }
        let mut state = Window::default();
        for (index, item) in items.iter().enumerate() {
            state.item(item, index)?;
        }
        if !state.pending.is_empty() {
            return Err(fail(
                "NATIVE_PENDING_CALL",
                "native window ends with unresolved calls",
            ));
        }
        if state.last_checkpoint != Some(manifest.capture.last_checkpoint_index) {
            return Err(fail(
                "NATIVE_BOUNDARY",
                "native last checkpoint index mismatch or checkpoint absent",
            ));
        }
        Ok(Self {
            manifest_bytes: manifest_bytes.to_vec(),
            payload_bytes: payload_bytes.to_vec(),
            items,
            summary: BundleSummary {
                manifest_sha256,
                manifest,
                checkpoint_count: state.checkpoints,
                source_instruction_roles: state.instruction_roles.into_iter().collect(),
                agent_message_count: state.agent_messages,
            },
        })
    }

    pub fn manifest(&self) -> &BundleManifest {
        &self.summary.manifest
    }
    pub fn summary(&self) -> BundleSummary {
        self.summary.clone()
    }
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }
    pub fn payload_bytes(&self) -> &[u8] {
        &self.payload_bytes
    }
    /// Exact item JSON, including unknown fields and numeric spellings.
    pub fn raw_items(&self) -> &[Box<RawValue>] {
        &self.items
    }
}

fn validate_manifest(m: &BundleManifest, digest: &str) -> Result<()> {
    if m.schema_version != 1 || m.format != "astral-codex-native" {
        return Err(fail(
            "NATIVE_UNSUPPORTED_FORMAT",
            "unsupported native bundle format or schema version",
        ));
    }
    let c = &m.compatibility;
    if c.runtime != "codex"
        || c.runtime_version != "0.154.0"
        || c.protocol != "openai-responses-lite"
        || c.provider != "openai"
        || c.model != "gpt-6-astra"
        || !c.requires_tool_rebinding
        || c.identity_scope != "same-account"
    {
        return Err(fail(
            "NATIVE_UNSUPPORTED_COMPATIBILITY",
            "unsupported native runtime compatibility declaration",
        ));
    }
    if m.payload.file != "window.json"
        || !hex(&m.payload.sha256, &[64])
        || m.payload.bytes > MAX_PAYLOAD_BYTES
        || m.payload.item_count > MAX_ITEMS
        || !hex(&m.source.selection_sha256, &[64])
        || !hex(&m.source.history_sha256, &[64])
        || m.source
            .revision
            .as_ref()
            .is_some_and(|v| !hex(v, &[40, 64]))
        || m.source.project_id.is_empty()
        || m.source.project_id.len() > 256
        || matches!(m.source.project_id.as_str(), "." | "..")
        || !m
            .source
            .project_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(fail(
            "NATIVE_INVALID_MANIFEST",
            "invalid native payload or source metadata",
        ));
    }
    if !m.capture.history_complete
        || !matches!(
            m.capture.boundary.as_str(),
            "completed-turn" | "completed-compaction"
        )
    {
        return Err(fail(
            "NATIVE_BOUNDARY",
            "native bundle requires an attested completed history boundary",
        ));
    }
    let mut parents = BTreeSet::new();
    if m.parents.len() > MAX_PARENTS
        || m.parents
            .iter()
            .any(|p| !hex(p, &[64]) || p == digest || !parents.insert(p))
    {
        return Err(fail(
            "NATIVE_INVALID_PARENTS",
            "native parent identities are invalid, repeated, or excessive",
        ));
    }
    Ok(())
}

type Object<'a> = BTreeMap<String, &'a RawValue>;

// RawValue's syntax-only scan avoids numeric conversion (including 1e99999).
// Each recursive visit parses at most the byte-bounded value at depth <= 64.
// Keys are decoded before comparison, so escaped aliases cannot hide duplicates.
struct UniqueObject<'a>(Object<'a>);
impl<'de> Deserialize<'de> for UniqueObject<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = UniqueObject<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object with unique keys")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut fields = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                    if fields.len() >= MAX_JSON_NODES {
                        return Err(serde::de::Error::custom("native collection limit"));
                    }
                    if fields.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate object key"));
                    }
                }
                Ok(UniqueObject(fields))
            }
        }
        d.deserialize_map(ObjectVisitor)
    }
}

struct BoundedArray<'a, const N: usize>(Vec<&'a RawValue>);
impl<'de, const N: usize> Deserialize<'de> for BoundedArray<'de, N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct ArrayVisitor<const N: usize>;
        impl<'de, const N: usize> Visitor<'de> for ArrayVisitor<N> {
            type Value = BoundedArray<'de, N>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a bounded array")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<&'de RawValue>()? {
                    if values.len() >= N {
                        return Err(serde::de::Error::custom("native collection limit"));
                    }
                    values.push(value);
                }
                Ok(BoundedArray(values))
            }
        }
        d.deserialize_seq(ArrayVisitor::<N>)
    }
}

fn collection_error(e: serde_json::Error, code: &'static str, message: &'static str) -> Error {
    if e.to_string().starts_with("native collection limit") {
        fail("NATIVE_LIMIT", "native JSON collection exceeds limit")
    } else {
        fail(code, message)
    }
}

fn check_json(bytes: &[u8]) -> Result<()> {
    let raw: &RawValue = serde_json::from_slice(bytes).map_err(|_| {
        fail(
            "NATIVE_INVALID_JSON",
            "native bundle contains malformed JSON",
        )
    })?;
    fn visit(raw: &RawValue, depth: usize, nodes: &mut usize) -> Result<()> {
        *nodes += 1;
        if depth > MAX_JSON_DEPTH || *nodes > MAX_JSON_NODES {
            return Err(fail(
                "NATIVE_LIMIT",
                "native JSON depth or node count exceeds limit",
            ));
        }
        match raw.get().as_bytes().first() {
            Some(b'{') => {
                let object: UniqueObject = serde_json::from_str(raw.get()).map_err(|e| {
                    collection_error(
                        e,
                        "NATIVE_DUPLICATE_KEY",
                        "native JSON object contains repeated keys",
                    )
                })?;
                for value in object.0.values() {
                    visit(value, depth + 1, nodes)?;
                }
            }
            Some(b'[') => {
                let values: BoundedArray<MAX_JSON_NODES> = serde_json::from_str(raw.get())
                    .map_err(|e| {
                        collection_error(e, "NATIVE_INVALID_JSON", "native JSON array is invalid")
                    })?;
                for value in values.0 {
                    visit(value, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    visit(raw, 0, &mut 0)
}

fn object(raw: &RawValue) -> Result<Object<'_>> {
    serde_json::from_str(raw.get()).map_err(|_| invalid())
}
fn field<'a>(o: &'a Object<'a>, key: &str) -> Result<&'a RawValue> {
    o.get(key).copied().ok_or_else(invalid)
}
fn string(o: &Object<'_>, key: &str) -> Result<String> {
    serde_json::from_str(field(o, key)?.get()).map_err(|_| invalid())
}
fn nonempty(o: &Object<'_>, key: &str) -> Result<String> {
    let value = string(o, key)?;
    if value.is_empty() {
        return Err(invalid());
    }
    Ok(value)
}
fn optional_string(o: &Object<'_>, key: &str) -> Result<()> {
    if let Some(raw) = o.get(key) {
        if raw.get() != "null" {
            let _: String = serde_json::from_str(raw.get()).map_err(|_| invalid())?;
        }
    }
    Ok(())
}
fn array<'a>(o: &'a Object<'a>, key: &str) -> Result<Vec<&'a RawValue>> {
    serde_json::from_str(field(o, key)?.get()).map_err(|_| invalid())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Function,
    Custom,
    Search,
}

#[derive(Default)]
struct Window {
    seen_calls: BTreeSet<String>,
    pending: BTreeMap<String, Family>,
    checkpoints: usize,
    last_checkpoint: Option<usize>,
    instruction_roles: BTreeSet<String>,
    agent_messages: usize,
}

impl Window {
    fn call(&mut self, o: &Object<'_>, family: Family) -> Result<()> {
        let id = nonempty(o, "call_id")?;
        if !self.seen_calls.insert(id.clone()) {
            return Err(fail(
                "NATIVE_DUPLICATE_CALL",
                "native call identifier is reused",
            ));
        }
        self.pending.insert(id, family);
        Ok(())
    }
    fn output(&mut self, o: &Object<'_>, family: Family) -> Result<()> {
        let id = nonempty(o, "call_id")?;
        if self.pending.remove(&id) != Some(family) {
            return Err(fail(
                "NATIVE_ORPHAN_OUTPUT",
                "native output lacks a matching unresolved call",
            ));
        }
        Ok(())
    }
    fn item(&mut self, raw: &RawValue, index: usize) -> Result<()> {
        let o = object(raw)?;
        optional_string(&o, "id")?;
        metadata(&o)?;
        match string(&o, "type")?.as_str() {
            "message" => {
                let role = string(&o, "role")?;
                if !matches!(role.as_str(), "system" | "developer" | "user" | "assistant") {
                    return Err(invalid());
                }
                if matches!(role.as_str(), "system" | "developer") {
                    self.instruction_roles.insert(role);
                }
                if let Some(phase) = o.get("phase") {
                    if phase.get() != "null"
                        && !matches!(string(&o, "phase")?.as_str(), "commentary" | "final_answer")
                    {
                        return Err(invalid());
                    }
                }
                for content in array(&o, "content")? {
                    content_part(content, false)?;
                }
            }
            "agent_message" => {
                string(&o, "author")?;
                string(&o, "recipient")?;
                for content in array(&o, "content")? {
                    let content = object(content)?;
                    match string(&content, "type")?.as_str() {
                        "input_text" => {
                            string(&content, "text")?;
                        }
                        "encrypted_content" => {
                            string(&content, "encrypted_content")?;
                        }
                        _ => return Err(invalid()),
                    }
                }
                self.agent_messages += 1;
            }
            "reasoning" => {
                for content in array(&o, "summary")? {
                    text_part(content, &["summary_text"])?;
                }
                if o.get("content").is_some_and(|v| v.get() != "null") {
                    for content in array(&o, "content")? {
                        text_part(content, &["reasoning_text", "text"])?;
                    }
                }
                optional_string(&o, "encrypted_content")?;
            }
            "compaction" => {
                nonempty(&o, "encrypted_content")?;
                if !self.pending.is_empty() {
                    return Err(fail(
                        "NATIVE_PENDING_CALL",
                        "native checkpoint crosses unresolved calls",
                    ));
                }
                self.checkpoints += 1;
                self.last_checkpoint = Some(index);
            }
            "function_call" => {
                nonempty(&o, "name")?;
                string(&o, "arguments")?;
                optional_string(&o, "namespace")?;
                if o.get("encrypted_function_args")
                    .is_some_and(|v| v.get() != "null")
                {
                    let _: Vec<String> =
                        serde_json::from_str(field(&o, "encrypted_function_args")?.get())
                            .map_err(|_| invalid())?;
                }
                self.call(&o, Family::Function)?;
            }
            "custom_tool_call" => {
                nonempty(&o, "name")?;
                string(&o, "input")?;
                optional_string(&o, "namespace")?;
                optional_string(&o, "status")?;
                // Codex persists arbitrary optional status strings on call items;
                // the matching output establishes closure, without rewriting status.
                self.call(&o, Family::Custom)?;
            }
            "function_call_output" | "custom_tool_call_output" => {
                optional_string(&o, "name")?;
                let output = field(&o, "output")?;
                if output.get().starts_with('[') {
                    for content in array(&o, "output")? {
                        content_part(content, true)?;
                    }
                } else {
                    string(&o, "output")?;
                }
                let family = if string(&o, "type")? == "function_call_output" {
                    optional_string(&o, "namespace")?;
                    Family::Function
                } else {
                    Family::Custom
                };
                self.output(&o, family)?;
            }
            "tool_search_call" | "tool_search_output" => {
                let is_call = string(&o, "type")? == "tool_search_call";
                let execution = string(&o, "execution")?;
                if !matches!(execution.as_str(), "client" | "server") {
                    return Err(invalid());
                }
                if is_call {
                    field(&o, "arguments")?;
                    optional_string(&o, "status")?;
                } else {
                    array(&o, "tools")?;
                }
                if (!is_call || execution == "server") && string(&o, "status")? != "completed" {
                    return Err(invalid());
                }
                if execution == "client" {
                    if is_call {
                        self.call(&o, Family::Search)?;
                    } else {
                        self.output(&o, Family::Search)?;
                    }
                } else {
                    optional_string(&o, "call_id")?;
                    if is_call
                        && o.get("call_id").is_some_and(|v| v.get() != "null")
                        && !self.seen_calls.insert(nonempty(&o, "call_id")?)
                    {
                        return Err(fail(
                            "NATIVE_DUPLICATE_CALL",
                            "native call identifier is reused",
                        ));
                    }
                }
            }
            "web_search_call" => {
                if string(&o, "status")? != "completed" {
                    return Err(invalid());
                }
                if let Some(action) = o.get("action").filter(|v| v.get() != "null") {
                    let action = object(action)?;
                    match string(&action, "type")?.as_str() {
                        "search" => {
                            optional_string(&action, "query")?;
                            if action.get("queries").is_some_and(|v| v.get() != "null") {
                                let _: Vec<String> =
                                    serde_json::from_str(field(&action, "queries")?.get())
                                        .map_err(|_| invalid())?;
                            }
                        }
                        "open_page" => optional_string(&action, "url")?,
                        "find_in_page" => {
                            optional_string(&action, "url")?;
                            optional_string(&action, "pattern")?;
                        }
                        _ => return Err(invalid()),
                    }
                }
            }
            "image_generation_call" => {
                if string(&o, "status")? != "completed" {
                    return Err(invalid());
                }
                string(&o, "result")?;
                optional_string(&o, "revised_prompt")?;
            }
            _ => {
                return Err(fail(
                    "NATIVE_UNSUPPORTED_ITEM",
                    "native item kind is unsupported; no items may be dropped",
                ));
            }
        }
        Ok(())
    }
}

fn metadata(o: &Object<'_>) -> Result<()> {
    if let Some(raw) = o
        .get("internal_chat_message_metadata_passthrough")
        .filter(|v| v.get() != "null")
    {
        let metadata = object(raw)?;
        optional_string(&metadata, "turn_id")?;
        if let Some(time) = metadata.get("create_time").filter(|v| v.get() != "null") {
            // This known field is a Codex serde_json::Number. Unknown metadata
            // numbers remain syntax-only RawValue and retain arbitrary precision.
            let _: serde_json::Number = serde_json::from_str(time.get()).map_err(|_| invalid())?;
        }
    }
    Ok(())
}

fn text_part(raw: &RawValue, kinds: &[&str]) -> Result<()> {
    let o = object(raw)?;
    if !kinds.contains(&string(&o, "type")?.as_str()) {
        return Err(invalid());
    }
    string(&o, "text")?;
    Ok(())
}

fn content_part(raw: &RawValue, tool_output: bool) -> Result<()> {
    let o = object(raw)?;
    match string(&o, "type")?.as_str() {
        "input_text" => {
            string(&o, "text")?;
        }
        "output_text" if !tool_output => {
            string(&o, "text")?;
        }
        "input_image" | "input_audio" => {
            let key = if string(&o, "type")? == "input_image" {
                "image_url"
            } else {
                "audio_url"
            };
            if !nonempty(&o, key)?.starts_with("data:") {
                return Err(fail(
                    "NATIVE_EXTERNAL_REFERENCE",
                    "native media must be self-contained data URLs",
                ));
            }
            if key == "image_url"
                && o.get("detail").is_some_and(|v| v.get() != "null")
                && !matches!(
                    string(&o, "detail")?.as_str(),
                    "auto" | "low" | "high" | "original"
                )
            {
                return Err(invalid());
            }
        }
        "encrypted_content" if tool_output => {
            nonempty(&o, "encrypted_content")?;
        }
        _ => return Err(invalid()),
    }
    Ok(())
}
