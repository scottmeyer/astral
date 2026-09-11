//! Advisory command-hook envelopes for Codex 0.154.0.
//!
//! This module does not inspect transcripts, execute commands, or establish a
//! worker's identity. Only the current checker can validate an event's claims.

use crate::project::{Error, Result};
use serde::{Deserialize, Deserializer, de::IgnoredAny};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};

pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_NOTICE_CHARS: usize = 512;
const MAX_ID_BYTES: usize = 128;
const MAX_MODEL_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSource {
    Startup,
    Resume,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    SessionStart(SessionSource),
    UserPromptSubmit,
    Stop,
    PostCompact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    Model,
    Ui,
}

/// Retained metadata is bounded and inert. `session_id` alone is not authority:
/// Codex shares a root session identity with its descendant threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: EventKind,
    pub session_id: String,
    pub cwd: PathBuf,
    pub model: String,
}

impl Event {
    pub fn key(&self) -> &'static str {
        match self.kind {
            EventKind::SessionStart(SessionSource::Startup) => "session_start:startup",
            EventKind::SessionStart(SessionSource::Resume) => "session_start:resume",
            EventKind::SessionStart(SessionSource::Compact) => "session_start:compact",
            EventKind::UserPromptSubmit => "user_prompt_submit",
            EventKind::Stop => "stop",
            EventKind::PostCompact => "post_compact",
        }
    }

    pub fn name(&self) -> &'static str {
        match self.kind {
            EventKind::SessionStart(_) => "SessionStart",
            EventKind::UserPromptSubmit => "UserPromptSubmit",
            EventKind::Stop => "Stop",
            EventKind::PostCompact => "PostCompact",
        }
    }

    pub fn audience(&self) -> Audience {
        match self.kind {
            EventKind::SessionStart(_) | EventKind::UserPromptSubmit => Audience::Model,
            EventKind::Stop | EventKind::PostCompact => Audience::Ui,
        }
    }
}

// Fields not listed here (including prompt, last_assistant_message,
// transcript_path, tool bodies, and permission_mode) are skipped by serde.
// They never become strings/Values stored in the resulting event or an error.
#[derive(Deserialize)]
struct Input {
    hook_event_name: String,
    session_id: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    source: Option<String>,
    trigger: Option<String>,
    #[serde(default, rename = "agent_id", deserialize_with = "present")]
    subagent: bool,
}

fn present<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<bool, D::Error> {
    IgnoredAny::deserialize(deserializer).map(|_| true)
}

fn invalid() -> Error {
    Error {
        code: "HOOK_INPUT_INVALID",
        message: "Codex hook metadata is malformed or unsupported.".into(),
    }
}

fn bounded(value: Option<String>, max: usize) -> Result<String> {
    let value = value.ok_or_else(invalid)?;
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(value)
}

/// Parse only the supported advisory event metadata. Unknown events, subagents,
/// and unselected SessionStart sources do not trigger a context observation.
/// Oversized or malformed input returns a static, payload-free diagnostic.
pub fn parse(bytes: &[u8]) -> Result<Option<Event>> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(Error {
            code: "HOOK_INPUT_TOO_LARGE",
            message: "Codex hook input exceeds the advisory input limit.".into(),
        });
    }
    let input: Input = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if input.hook_event_name.len() > 64 {
        return Err(invalid());
    }
    if input.subagent {
        return Ok(None);
    }
    let kind = match input.hook_event_name.as_str() {
        "SessionStart" => match input.source.as_deref() {
            Some("startup") => EventKind::SessionStart(SessionSource::Startup),
            Some("resume") => EventKind::SessionStart(SessionSource::Resume),
            Some("compact") => EventKind::SessionStart(SessionSource::Compact),
            Some(_) => return Ok(None),
            None => return Err(invalid()),
        },
        "UserPromptSubmit" => EventKind::UserPromptSubmit,
        "Stop" => EventKind::Stop,
        "PostCompact" => {
            if !matches!(input.trigger.as_deref(), Some("manual" | "auto")) {
                return Err(invalid());
            }
            EventKind::PostCompact
        }
        _ => return Ok(None),
    };
    let session_id = bounded(input.session_id, MAX_ID_BYTES)?;
    if !session_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(invalid());
    }
    let model = bounded(input.model, MAX_MODEL_BYTES)?;
    let cwd = bounded(input.cwd, MAX_PATH_BYTES)?;
    let path = Path::new(&cwd);
    if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return Err(invalid());
    }
    Ok(Some(Event {
        kind,
        session_id,
        cwd: path.to_path_buf(),
        model,
    }))
}

/// Render a concise diagnosis supplied by the checker, never document content
/// or raw event fields. There are intentionally no blocking/continuation knobs.
/// An empty diagnosis emits an empty success envelope.
pub fn render(event: &Event, notice: &str) -> Value {
    let notice: String = notice
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_NOTICE_CHARS)
        .collect();
    let notice = notice.trim();
    if notice.is_empty() {
        return json!({});
    }
    match event.audience() {
        Audience::Model => json!({
            "hookSpecificOutput": {
                "hookEventName": event.name(),
                "additionalContext": notice,
            }
        }),
        Audience::Ui => json!({"systemMessage": notice}),
    }
}
