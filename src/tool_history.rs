//! Deterministic, model-independent reduction of completed text tool results.
//! Never drops messages, calls, tool definitions, reasoning or non-text content.
use crate::{archive::Archive, config::Config, providers::Wire};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Default, Debug, Serialize)]
pub struct Report {
    pub archived: usize,
    pub bytes_saved: usize,
    pub unavailable: usize,
    pub skipped: Option<&'static str>,
}

struct Candidate {
    path: String,
    name: String,
    index: usize,
}

fn user(message: &Value, wire: Wire) -> bool {
    if message["role"] != "user" {
        return false;
    }
    if wire != Wire::Messages {
        return true;
    }
    match &message["content"] {
        Value::String(_) => true,
        Value::Array(blocks) => blocks.iter().any(|b| b["type"] != "tool_result"),
        _ => false,
    }
}

fn collect(
    body: &Value,
    wire: Wire,
    keep: usize,
    keep_tools: usize,
) -> Result<Vec<Candidate>, &'static str> {
    // Let the provider/harness own server-side histories and explicit compaction.
    if ["previous_response_id", "conversation", "context_management"]
        .iter()
        .any(|key| body.get(*key).is_some_and(|v| !v.is_null()))
        || body["background"] == true
    {
        return Err("provider_managed_history");
    }
    let field = if wire == Wire::Responses {
        "input"
    } else {
        "messages"
    };
    let messages = body[field].as_array().ok_or("requires_explicit_history")?;
    if messages.iter().any(|m| {
        matches!(
            m["type"].as_str(),
            Some("item_reference" | "compaction_trigger" | "configuration_update")
        )
    }) {
        return Err("native_control_or_reference");
    }
    let turns: Vec<_> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| user(m, wire))
        .map(|(i, _)| i)
        .collect();
    let cutoff = if turns.len() > keep {
        turns[turns.len() - keep]
    } else {
        0
    };
    let last_assistant = messages
        .iter()
        .rposition(|m| {
            m["role"] == "assistant"
                || (wire == Wire::Responses
                    && matches!(
                        m["type"].as_str(),
                        Some("function_call" | "custom_tool_call" | "reasoning")
                    ))
        })
        .unwrap_or(0);
    let mut calls: HashMap<String, (usize, String)> = HashMap::new();
    let mut results = HashSet::new();
    let mut result_indices = Vec::new();
    let mut candidates = Vec::new();
    for (i, message) in messages.iter().enumerate() {
        let mut add_call = |id: &Value, name: &Value| -> Result<(), &'static str> {
            let id = id
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("invalid_tool_id")?;
            if calls
                .insert(id.into(), (i, name.as_str().unwrap_or("tool").into()))
                .is_some()
            {
                return Err("duplicate_tool_call");
            }
            Ok(())
        };
        match wire {
            Wire::Responses
                if matches!(
                    message["type"].as_str(),
                    Some("function_call" | "custom_tool_call")
                ) =>
            {
                add_call(&message["call_id"], &message["name"])?
            }
            Wire::Messages => {
                if let Some(blocks) = message["content"].as_array() {
                    for block in blocks {
                        if block["type"] == "tool_use" {
                            if message["role"] != "assistant" {
                                return Err("invalid_tool_role");
                            }
                            add_call(&block["id"], &block["name"])?;
                        }
                    }
                }
            }
            Wire::Chat => {
                if let Some(tools) = message["tool_calls"].as_array() {
                    if message["role"] != "assistant" {
                        return Err("invalid_tool_role");
                    }
                    for tool in tools {
                        add_call(&tool["id"], &tool["function"]["name"])?;
                    }
                }
            }
            _ => {}
        }
        let mut add_result = |id: &Value, path: String, failed: bool| -> Result<(), &'static str> {
            let id = id.as_str().ok_or("invalid_tool_result_id")?;
            let (call_index, name) = calls.get(id).ok_or("unmatched_tool_result")?;
            if !results.insert(id.to_owned()) {
                return Err("duplicate_tool_result");
            }
            if *call_index >= i {
                return Err("invalid_tool_order");
            }
            result_indices.push(i);
            if !failed {
                candidates.push(Candidate {
                    path,
                    name: name.clone(),
                    index: i,
                });
            }
            Ok(())
        };
        match wire {
            Wire::Responses
                if matches!(
                    message["type"].as_str(),
                    Some("function_call_output" | "custom_tool_call_output")
                ) =>
            {
                add_result(
                    &message["call_id"],
                    format!("/{field}/{i}/output"),
                    message["is_error"] == true,
                )?;
            }
            Wire::Messages => {
                if let Some(blocks) = message["content"].as_array() {
                    for (j, block) in blocks.iter().enumerate() {
                        if block["type"] == "tool_result" {
                            if message["role"] != "user" {
                                return Err("invalid_tool_role");
                            }
                            add_result(
                                &block["tool_use_id"],
                                format!("/{field}/{i}/content/{j}/content"),
                                block["is_error"] == true,
                            )?;
                        }
                    }
                }
            }
            Wire::Chat if message["role"] == "tool" => add_result(
                &message["tool_call_id"],
                format!("/{field}/{i}/content"),
                message["is_error"] == true,
            )?,
            _ => {}
        }
    }
    // Incomplete batches are left entirely intact, including older results.
    if calls.len() != results.len() {
        return Err("pending_tool_calls");
    }
    let tool_cutoff = if keep_tools > 0 && result_indices.len() > keep_tools {
        result_indices[result_indices.len() - keep_tools]
    } else {
        0
    };
    // Never replace outputs from the latest assistant's batch, even if that
    // parallel batch is larger than the configured recent-result count.
    candidates.retain(|c| c.index < cutoff || (c.index < tool_cutoff && c.index < last_assistant));
    Ok(candidates)
}

pub(crate) fn explicit_failure(text: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|value| {
        value["is_error"] == true
            || value["isError"] == true
            || value["success"] == false
            || value["ok"] == false
            || value["failed"].as_u64().is_some_and(|n| n > 0)
            || value["exit_code"].as_i64().is_some_and(|n| n != 0)
            || value
                .get("error")
                .is_some_and(|v| !v.is_null() && v != false)
    })
}

/// Only plain text or arrays of unannotated text blocks can be replaced.
fn text_payload(value: &Value, wire: Wire) -> Option<(String, &'static str, String)> {
    if let Some(text) = value.as_str() {
        return Some((text.into(), "utf8", text.into()));
    }
    let blocks = value.as_array()?;
    if blocks.is_empty()
        || !blocks.iter().all(|b| {
            (b["type"] == "text" || (wire == Wire::Responses && b["type"] == "input_text"))
                && b["text"].is_string()
                && b.as_object()
                    .is_some_and(|o| o.keys().all(|k| k == "type" || k == "text"))
        })
    {
        return None;
    }
    let display = blocks
        .iter()
        .map(|b| b["text"].as_str().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    Some((serde_json::to_string(value).unwrap(), "json", display))
}

pub(crate) fn preview(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.into();
    }
    let mut head = budget / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - budget / 2;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!(
        "{}\n[... {} bytes omitted ...]\n{}",
        &text[..head],
        tail - head,
        &text[tail..]
    )
}

pub fn reduce(
    body: &mut Value,
    wire: Wire,
    config: &Config,
    archive: &Archive,
    scope: &str,
) -> Report {
    let mut report = Report::default();
    let candidates = match collect(
        body,
        wire,
        config.keep_recent_turns,
        config.keep_recent_tool_results,
    ) {
        Ok(candidates) => candidates,
        Err(reason) => {
            report.skipped = Some(reason);
            return report;
        }
    };
    for candidate in candidates {
        let Some(value) = body.pointer(&candidate.path) else {
            continue;
        };
        let Some((original, format, display)) = text_payload(value, wire) else {
            continue;
        };
        if original.len() < config.tool_result_bytes
            || explicit_failure(&display)
            || display.starts_with("[astral archived tool output;")
        {
            continue;
        }
        let handle = match archive.save(scope, format, &original) {
            Ok(handle) => handle,
            Err(_) => {
                report.unavailable += 1;
                continue;
            }
        };
        // Serialize the tool label as data; no raw tool-name newlines in the marker.
        let label = serde_json::to_string(&candidate.name).unwrap();
        let replacement = format!(
            "[astral archived tool output; tool={label}; original_bytes={}; format={format}; handle={handle}]\nExact original output: astral_recall(handle=\"{handle}\") MCP tool, or `astral recall {handle}`. Search with astral_search. Retrieval reads the saved output without rerunning the tool. This preview is incomplete.\n{}",
            original.len(),
            preview(&display, config.tool_preview_bytes)
        );
        let replacement = Value::String(replacement);
        let before = serde_json::to_vec(value).unwrap().len();
        let after = serde_json::to_vec(&replacement).unwrap().len();
        if after < before {
            *body.pointer_mut(&candidate.path).unwrap() = replacement;
            report.archived += 1;
            report.bytes_saved += before - after;
        }
    }
    report
}
