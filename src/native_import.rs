//! Codex 0.154.0 import boundary, distinct from the lossless bundle reader.
//!
//! `thread/inject_items` accepts typed ResponseItems. Unknown fields and
//! host-owned call metadata would be discarded by that runtime. Reject them
//! before starting a destination thread; never silently strip source fields.

use crate::codex::error;
use crate::native_bundle::NativeBundle;
use crate::project::Result;
use serde_json::Value;

fn unsupported() -> crate::project::Error {
    error(
        "NATIVE_IMPORT_UNSUPPORTED",
        "bundle contains fields that Codex 0.154.0 cannot import faithfully; the saved artifact is unchanged",
    )
}

fn fields(value: &Value, allowed: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or_else(unsupported)?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(unsupported());
    }
    Ok(())
}

fn content(value: &Value) -> Result<()> {
    let allowed: &[&str] = match value.get("type").and_then(Value::as_str) {
        Some("input_text" | "output_text" | "summary_text" | "reasoning_text" | "text") => {
            &["type", "text"]
        }
        Some("input_image") => &["type", "image_url", "detail"],
        Some("input_audio") => &["type", "audio_url"],
        Some("encrypted_content") => &["type", "encrypted_content"],
        _ => return Err(unsupported()),
    };
    fields(value, allowed)
}

// Compare decimal values without converting through floating point. Codex uses
// serde_json::Value inside tool-search arguments/results, where an oversized
// integer can otherwise be accepted and rounded to f64 during typed import.
fn decimal(value: &str) -> Option<(bool, String, i64)> {
    let negative = value.starts_with('-');
    let value = value.strip_prefix('-').unwrap_or(value);
    let (mantissa, exponent) = value.split_once(['e', 'E']).unwrap_or((value, "0"));
    let mut exponent: i64 = exponent.parse().ok()?;
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    exponent = exponent.checked_sub(i64::try_from(fraction.len()).ok()?)?;
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some((negative, "0".into(), 0));
    }
    let trimmed = digits.trim_end_matches('0');
    exponent = exponent.checked_add(i64::try_from(digits.len() - trimmed.len()).ok()?)?;
    Some((negative, trimmed.to_owned(), exponent))
}

fn numbers_roundtrip(raw: &str) -> Result<()> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
        } else if bytes[i] == b'-' || bytes[i].is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || b".eE+-".contains(&bytes[i])) {
                i += 1;
            }
            let token = &raw[start..i];
            let number: serde_json::Number =
                serde_json::from_str(token).map_err(|_| unsupported())?;
            let original = decimal(token).ok_or_else(unsupported)?;
            if Some(original) != decimal(&number.to_string()) {
                return Err(unsupported());
            }
        } else {
            i += 1;
        }
    }
    Ok(())
}

/// This gate does not rewrite bytes or claim that app-server is a raw JSON
/// store. Optional nulls/defaults and destination metadata may be normalized.
pub fn validate(bundle: &NativeBundle) -> Result<()> {
    for raw in bundle.raw_items() {
        numbers_roundtrip(raw.get())?;
        let value: Value = serde_json::from_str(raw.get()).map_err(|_| unsupported())?;
        let specific: &[&str] = match value.get("type").and_then(Value::as_str) {
            Some("compaction" | "context_compaction") => &["encrypted_content"],
            Some("message") => &["role", "content", "phase"],
            Some("agent_message") => &["author", "recipient", "content"],
            Some("reasoning") => &["summary", "content", "encrypted_content"],
            Some("function_call") => &[
                "name",
                "namespace",
                "arguments",
                "encrypted_function_args",
                "call_id",
            ],
            Some("custom_tool_call") => &["name", "namespace", "input", "call_id", "status"],
            Some("function_call_output") => &["name", "namespace", "call_id", "output"],
            Some("custom_tool_call_output") => &["name", "call_id", "output"],
            Some("tool_search_call") => &["call_id", "status", "execution", "arguments"],
            Some("tool_search_output") => &["call_id", "status", "execution", "tools"],
            Some("web_search_call") => &["status", "action"],
            Some("image_generation_call") => &["status", "revised_prompt", "result"],
            _ => return Err(unsupported()),
        };
        let mut allowed = vec!["type", "id", "internal_chat_message_metadata_passthrough"];
        allowed.extend_from_slice(specific);
        fields(&value, &allowed)?;
        if value.get("type").and_then(Value::as_str) == Some("reasoning") {
            if let Some(parts) = value.get("content").and_then(Value::as_array) {
                // Codex's should_serialize_reasoning_content drops all-text
                // legacy arrays unless at least one reasoning_text is present.
                if !parts.is_empty()
                    && !parts
                        .iter()
                        .any(|p| p.get("type").and_then(Value::as_str) == Some("reasoning_text"))
                {
                    return Err(unsupported());
                }
            }
        }
        if let Some(metadata) = value
            .get("internal_chat_message_metadata_passthrough")
            .filter(|v| !v.is_null())
        {
            fields(metadata, &["turn_id", "create_time", "content_item_kinds"])?;
            if let Some(kinds) = metadata.get("content_item_kinds").filter(|v| !v.is_null()) {
                if !kinds
                    .as_array()
                    .is_some_and(|a| a.iter().all(Value::is_string))
                {
                    return Err(unsupported());
                }
            }
        }
        for key in ["content", "summary", "output"] {
            if let Some(parts) = value.get(key).and_then(Value::as_array) {
                for part in parts {
                    content(part)?;
                }
            }
        }
        if let Some(action) = value.get("action").filter(|v| !v.is_null()) {
            let allowed: &[&str] = match action.get("type").and_then(Value::as_str) {
                Some("search") => &["type", "query", "queries"],
                Some("open_page") => &["type", "url"],
                Some("find_in_page") => &["type", "url", "pattern"],
                _ => return Err(unsupported()),
            };
            fields(action, allowed)?;
        }
    }
    Ok(())
}
