//! Capture only a newly installed, successfully completed Codex 0.154.0 compaction.
//!
//! This is a native conversational replay export, not a host-session backup. Guardian
//! evidence, retained authorization, MCP/account provenance, runtime settings and
//! world-state diff baselines are deliberately not portable authority. No historical
//! tail is reconstructed. Callers own the worker lock and stop its writer before each
//! snapshot; filesystem checks detect observed changes, not hostile concurrent writes.

use crate::native_bundle::{MAX_ITEMS, MAX_PAYLOAD_BYTES};
use crate::project::{Error, Result};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

pub const MAX_ROLLOUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_LINES: usize = 131_072;
const MAX_NODES: usize = 524_288;
const MAX_DEPTH: usize = 64;
type Object<'a> = BTreeMap<String, &'a RawValue>;

pub struct RolloutSnapshot {
    bytes: Vec<u8>,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl fmt::Debug for RolloutSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RolloutSnapshot")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

pub struct CapturedWindow {
    pub payload_bytes: Vec<u8>,
    pub source_history_sha256: String,
    pub last_checkpoint_index: usize,
    pub item_count: usize,
}

impl fmt::Debug for CapturedWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapturedWindow")
            .field("bytes", &self.payload_bytes.len())
            .field("source_history_sha256", &self.source_history_sha256)
            .field("last_checkpoint_index", &self.last_checkpoint_index)
            .field("item_count", &self.item_count)
            .finish()
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
        "CAPTURE_INVALID_ROLLOUT",
        "rollout does not match the supported capture format",
    )
}
fn boundary() -> Error {
    fail(
        "CAPTURE_BOUNDARY",
        "new compaction is not an isolated successful completed boundary",
    )
}
fn unsupported() -> Error {
    fail(
        "CAPTURE_UNSUPPORTED_HISTORY",
        "rollout contains unsupported history or host metadata",
    )
}
fn limit() -> Error {
    fail("CAPTURE_LIMIT", "rollout exceeds capture limits")
}
fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

struct UniqueObject<'a>(Object<'a>);
impl<'de> Deserialize<'de> for UniqueObject<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = UniqueObject<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("unique object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut fields = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                    if fields.len() >= MAX_NODES || fields.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom(
                            "capture object limit or duplicate",
                        ));
                    }
                }
                Ok(UniqueObject(fields))
            }
        }
        d.deserialize_map(V)
    }
}
struct Array<'a>(Vec<&'a RawValue>);
impl<'de> Deserialize<'de> for Array<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Array<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("bounded array")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<&'de RawValue>()? {
                    if values.len() >= MAX_NODES {
                        return Err(serde::de::Error::custom("capture array limit"));
                    }
                    values.push(value);
                }
                Ok(Array(values))
            }
        }
        d.deserialize_seq(V)
    }
}
fn object(raw: &RawValue) -> Result<Object<'_>> {
    serde_json::from_str::<UniqueObject>(raw.get())
        .map(|o| o.0)
        .map_err(|_| invalid())
}
fn array(raw: &RawValue) -> Result<Vec<&RawValue>> {
    serde_json::from_str::<Array>(raw.get())
        .map(|a| a.0)
        .map_err(|_| invalid())
}
fn field<'a>(o: &Object<'a>, key: &str) -> Result<&'a RawValue> {
    o.get(key).copied().ok_or_else(invalid)
}
fn string(o: &Object<'_>, key: &str) -> Result<String> {
    serde_json::from_str(field(o, key)?.get()).map_err(|_| invalid())
}
fn null_or_absent(o: &Object<'_>, key: &str) -> bool {
    o.get(key).is_none_or(|v| v.get() == "null")
}
fn fields(o: &Object<'_>, allowed: &[&str]) -> Result<()> {
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(unsupported());
    }
    Ok(())
}
fn check(raw: &RawValue, depth: usize, nodes: &mut usize) -> Result<()> {
    *nodes += 1;
    if depth > MAX_DEPTH || *nodes > MAX_NODES {
        return Err(limit());
    }
    match raw.get().as_bytes().first() {
        Some(b'{') => {
            for value in object(raw)?.values() {
                check(value, depth + 1, nodes)?;
            }
        }
        Some(b'[') => {
            for value in array(raw)? {
                check(value, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

struct Record<'a> {
    kind: String,
    payload: &'a RawValue,
    ordinal: Option<u64>,
}
fn records<'a>(bytes: &'a [u8], nodes: &mut usize) -> Result<Vec<Record<'a>>> {
    if bytes.is_empty() || bytes.len() > MAX_ROLLOUT_BYTES {
        return Err(limit());
    }
    if !bytes.ends_with(b"\n") {
        return Err(invalid());
    }
    let mut result = Vec::new();
    for line in bytes[..bytes.len() - 1].split(|b| *b == b'\n') {
        if result.len() >= MAX_LINES {
            return Err(limit());
        }
        let raw: &RawValue = serde_json::from_slice(line).map_err(|_| invalid())?;
        check(raw, 0, nodes)?;
        let o = object(raw)?;
        fields(&o, &["timestamp", "ordinal", "type", "payload", "metadata"])?;
        if string(&o, "timestamp")?.is_empty() {
            return Err(invalid());
        }
        let ordinal = o
            .get("ordinal")
            .map(|r| serde_json::from_str::<u64>(r.get()).map_err(|_| invalid()))
            .transpose()?;
        let kind = string(&o, "type")?;
        if o.contains_key("metadata") && kind != "response_item" {
            return Err(unsupported());
        }
        result.push(Record {
            kind,
            payload: field(&o, "payload")?,
            ordinal,
        });
    }
    Ok(result)
}

fn source(records: &[Record<'_>], thread: &str) -> Result<bool> {
    if !uuid(thread) {
        return Err(invalid());
    }
    let first = records.first().ok_or_else(invalid)?;
    if first.kind != "session_meta" {
        return Err(invalid());
    }
    let meta = object(first.payload)?;
    if string(&meta, "id")? != thread || string(&meta, "cli_version")? != "0.154.0" {
        return Err(fail(
            "CAPTURE_SOURCE_MISMATCH",
            "capture source ID or runtime version differs",
        ));
    }
    if !null_or_absent(&meta, "history_base") {
        return Err(unsupported());
    }
    let paginated = match meta.get("history_mode") {
        None => false,
        Some(raw) => match serde_json::from_str::<String>(raw.get())
            .map_err(|_| invalid())?
            .as_str()
        {
            "legacy" => false,
            "paginated" => true,
            _ => return Err(unsupported()),
        },
    };
    for record in &records[1..] {
        if record.kind == "session_meta" {
            return Err(unsupported());
        }
        if record.kind == "event_msg"
            && string(&object(record.payload)?, "type")? == "thread_rolled_back"
        {
            return Err(unsupported());
        }
    }
    Ok(paginated)
}

fn metadata(compacted: &Object<'_>, count: usize) -> Result<()> {
    if let Some(raw) = compacted
        .get("replacement_history_metadata")
        .filter(|r| r.get() != "null")
    {
        let entries = array(raw)?;
        if entries.len() != count {
            return Err(invalid());
        }
        for raw in entries {
            let o = object(raw)?;
            fields(
                &o,
                &[
                    "client_authored",
                    "fallback_token_limit_override",
                    "harness_authored_configuration",
                    "compaction_model_hash",
                    "user_input_order",
                    "inherited_user_message",
                ],
            )?;
            if !null_or_absent(&o, "fallback_token_limit_override") {
                return Err(unsupported());
            }
            for key in [
                "client_authored",
                "harness_authored_configuration",
                "inherited_user_message",
            ] {
                if let Some(raw) = o.get(key) {
                    let value: bool = serde_json::from_str(raw.get()).map_err(|_| invalid())?;
                    if key == "harness_authored_configuration" && value {
                        return Err(unsupported());
                    }
                }
            }
            if !null_or_absent(&o, "compaction_model_hash") {
                let _ = string(&o, "compaction_model_hash")?;
            }
            if let Some(raw) = o.get("user_input_order").filter(|r| r.get() != "null") {
                let _: u64 = serde_json::from_str(raw.get()).map_err(|_| invalid())?;
            }
        }
    }
    Ok(())
}

/// The pure parser does not establish filesystem identity or source ownership.
/// The caller must separately validate the returned native bundle and import compatibility.
pub fn capture_bytes(
    before: &[u8],
    after: &[u8],
    thread: &str,
    turn: &str,
    item_id: &str,
) -> Result<CapturedWindow> {
    if !identifier(turn) || !uuid(item_id) {
        return Err(invalid());
    }
    if after.len() > MAX_ROLLOUT_BYTES || after.len() <= before.len() || !after.starts_with(before)
    {
        return Err(fail(
            "CAPTURE_SOURCE_CHANGED",
            "rollout is not an unchanged prefix plus appended compaction",
        ));
    }
    let mut nodes = 0;
    let old = records(before, &mut nodes)?;
    let paginated = source(&old, thread)?;
    let new = records(&after[before.len()..], &mut nodes)?;
    if old.len() + new.len() > MAX_LINES {
        return Err(limit());
    }
    let mut ordinal = None;
    for record in old.iter().chain(new.iter()) {
        if paginated {
            let next = record.ordinal.ok_or_else(invalid)?;
            if ordinal.is_some_and(|last: u64| last.checked_add(1) != Some(next)) {
                return Err(invalid());
            }
            ordinal = Some(next);
        }
    }
    let mut old_active = None;
    let mut old_turns = BTreeSet::new();
    for record in &old {
        if record.kind != "event_msg" {
            continue;
        }
        let event = object(record.payload)?;
        match string(&event, "type")?.as_str() {
            "task_started" | "turn_started" => {
                let id = string(&event, "turn_id")?;
                if old_active.is_some() || !old_turns.insert(id.clone()) {
                    return Err(boundary());
                }
                old_active = Some(id);
            }
            "task_complete" | "turn_complete" | "turn_aborted" => {
                let id = string(&event, "turn_id")?;
                if old_active.as_deref() != Some(&id) {
                    return Err(boundary());
                }
                old_active = None;
            }
            _ => {}
        }
    }
    if old_active.is_some() || old_turns.contains(turn) {
        return Err(boundary());
    }
    let mut started = false;
    let mut completed = false;
    let mut compaction_completed = false;
    let mut payload = None;
    let mut last_checkpoint = None;
    let mut item_count = 0;
    let mut world_state_seen = false;
    let mut turn_context_seen = false;
    for record in &new {
        match record.kind.as_str() {
            "compacted" => {
                if !started || completed || payload.is_some() {
                    return Err(boundary());
                }
                let o = object(record.payload)?;
                fields(
                    &o,
                    &[
                        "message",
                        "replacement_history",
                        "replacement_history_metadata",
                        "guardian_history",
                        "retained_context",
                        "mcp_resource_origins",
                        "window_number",
                        "first_window_id",
                        "previous_window_id",
                        "window_id",
                        "compaction_response_id",
                        "latest_token_usage_record",
                    ],
                )?;
                let _: u64 = serde_json::from_str(field(&o, "window_number")?.get())
                    .map_err(|_| unsupported())?;
                let replacement = field(&o, "replacement_history")?;
                if replacement.get().len() > MAX_PAYLOAD_BYTES {
                    return Err(limit());
                }
                let items = array(replacement)?;
                if items.is_empty() || items.len() > MAX_ITEMS {
                    return Err(limit());
                }
                item_count = items.len();
                metadata(&o, item_count)?;
                for (index, item) in items.iter().enumerate() {
                    if string(&object(item)?, "type")? == "compaction" {
                        last_checkpoint = Some(index);
                    }
                }
                if last_checkpoint.is_none() {
                    return Err(unsupported());
                }
                payload = Some(replacement.get().as_bytes().to_vec());
            }
            "event_msg" => {
                let event = object(record.payload)?;
                match string(&event, "type")?.as_str() {
                    "task_started" | "turn_started" => {
                        if started || completed || string(&event, "turn_id")? != turn {
                            return Err(boundary());
                        }
                        started = true;
                    }
                    "task_complete" | "turn_complete" => {
                        if !started
                            || completed
                            || !compaction_completed
                            || payload.is_none()
                            || string(&event, "turn_id")? != turn
                            || !null_or_absent(&event, "error")
                        {
                            return Err(boundary());
                        }
                        completed = true;
                    }
                    "item_completed" => {
                        let item = object(field(&event, "item")?)?;
                        if !paginated
                            || payload.is_none()
                            || completed
                            || compaction_completed
                            || string(&event, "turn_id")? != turn
                            || string(&event, "thread_id")? != thread
                            || string(&item, "type")? != "ContextCompaction"
                            || string(&item, "id")? != item_id
                        {
                            return Err(boundary());
                        }
                        compaction_completed = true;
                    }
                    "context_compacted" => {
                        if paginated || payload.is_none() || completed || compaction_completed {
                            return Err(boundary());
                        }
                        compaction_completed = true;
                    }
                    "token_count" => {}
                    "thread_settings_applied" => {
                        if completed {
                            return Err(boundary());
                        }
                        if !null_or_absent(&event, "thread_id")
                            && string(&event, "thread_id")? != thread
                        {
                            return Err(boundary());
                        }
                    }
                    _ => return Err(unsupported()),
                }
            }
            "token_usage_record" => {}
            "world_state" => {
                // A single full baseline is part of installing this checkpoint, not a tail patch.
                if payload.is_none() || completed || compaction_completed || world_state_seen {
                    return Err(boundary());
                }
                let o = object(record.payload)?;
                fields(&o, &["full", "state"])?;
                if field(&o, "full")?.get() != "true" {
                    return Err(unsupported());
                }
                let _ = object(field(&o, "state")?)?;
                world_state_seen = true;
            }
            "turn_context" => {
                if completed || compaction_completed || turn_context_seen {
                    return Err(boundary());
                }
                let o = object(record.payload)?;
                if !null_or_absent(&o, "turn_id") && string(&o, "turn_id")? != turn {
                    return Err(boundary());
                }
                turn_context_seen = true;
            }
            _ => return Err(unsupported()),
        }
    }
    if !completed {
        return Err(boundary());
    }
    Ok(CapturedWindow {
        payload_bytes: payload.ok_or_else(boundary)?,
        source_history_sha256: crate::hash(after),
        last_checkpoint_index: last_checkpoint.ok_or_else(boundary)?,
        item_count,
    })
}

pub fn capture(
    before: &RolloutSnapshot,
    after: &RolloutSnapshot,
    thread: &str,
    turn: &str,
    item_id: &str,
) -> Result<CapturedWindow> {
    #[cfg(unix)]
    if before.identity != after.identity {
        return Err(fail(
            "CAPTURE_SOURCE_CHANGED",
            "rollout file identity changed during capture",
        ));
    }
    capture_bytes(&before.bytes, &after.bytes, thread, turn, item_id)
}

/// Open an absolute source path without following any symlink component.
#[cfg(unix)]
pub fn read_snapshot(path: &Path, thread: &str) -> Result<RolloutSnapshot> {
    use std::ffi::CString;
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    use std::path::Component;
    let file_error = || {
        fail(
            "CAPTURE_SOURCE_FILE",
            "capture source must be a readable, owned, regular non-symlink file",
        )
    };
    if !path.is_absolute() {
        return Err(file_error());
    }
    let components: Vec<_> = path.components().collect();
    let mut file = File::open("/").map_err(|_| file_error())?;
    for (i, component) in components.iter().enumerate().skip(1) {
        let Component::Normal(name) = component else {
            return Err(file_error());
        };
        let name = CString::new(name.as_bytes()).map_err(|_| file_error())?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if i + 1 == components.len() {
                0
            } else {
                libc::O_DIRECTORY
            };
        // Each descriptor is opened relative to its already-open, non-symlink parent.
        let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(file_error());
        }
        file = unsafe { File::from_raw_fd(fd) };
    }
    let before = file.metadata().map_err(|_| file_error())?;
    if !before.is_file() || before.nlink() != 1 || before.uid() != unsafe { libc::geteuid() } {
        return Err(file_error());
    }
    if before.len() > MAX_ROLLOUT_BYTES as u64 {
        return Err(limit());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_ROLLOUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| file_error())?;
    let after = file.metadata().map_err(|_| file_error())?;
    if bytes.len() > MAX_ROLLOUT_BYTES {
        return Err(limit());
    }
    if before.len() != after.len()
        || before.len() != bytes.len() as u64
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
        || after.nlink() != 1
    {
        return Err(fail(
            "CAPTURE_SOURCE_CHANGED",
            "rollout changed while its bytes were read",
        ));
    }
    let parsed = records(&bytes, &mut 0)?;
    source(&parsed, thread)?;
    Ok(RolloutSnapshot {
        bytes,
        identity: (after.dev(), after.ino()),
    })
}

#[cfg(not(unix))]
pub fn read_snapshot(_path: &Path, _thread: &str) -> Result<RolloutSnapshot> {
    Err(fail(
        "CAPTURE_UNSUPPORTED_PLATFORM",
        "native capture currently requires Unix descriptor checks",
    ))
}
