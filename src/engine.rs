//! Pure projection planning. No inferred cache expiry and no mutation on response failure.
use crate::{
    config::{CompactionBackend, Config},
    fingerprint,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Lane {
    pub version: u32,
    pub contract: String,
    pub input_hashes: Vec<String>,
    pub cut: usize,
    pub projection: Vec<Value>,
    pub epoch: u64,
    pub last_success_ms: u64,
    pub last_roll_attempt_ms: u64,
}

pub struct Plan {
    pub next: Lane,
    pub effective: Vec<Value>,
    pub compact_input: Option<Vec<Value>>,
    pub compact_cut: usize,
    pub reason: &'static str,
}

pub fn exclusion(request: &Value) -> Option<&'static str> {
    if !request.is_object() {
        return Some("non_object");
    }
    if request
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
        || request.get("conversation").is_some_and(|v| !v.is_null())
        || request
            .get("context_management")
            .is_some_and(|v| !v.is_null())
    {
        return Some("provider_managed_context");
    }
    if request.get("background") == Some(&json!(true)) {
        return Some("background");
    }
    if !request.get("input").is_some_and(Value::is_array) {
        return Some("requires_full_input_array");
    }
    // References require server-side expansion; local history is incomplete.
    if request["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["type"] == "item_reference")
    {
        return Some("item_reference");
    }
    None
}

pub fn is_user(item: &Value) -> bool {
    item["role"] == "user" && item.get("type").is_none_or(|v| v == "message")
}

pub fn contract(request: &Value) -> String {
    let mut v = request.clone();
    let obj = v.as_object_mut().expect("validated object");
    obj.remove("input");
    // Transport choices and arbitrary metadata do not alter the rendered prompt.
    obj.remove("stream");
    obj.remove("stream_options");
    obj.remove("metadata");
    fingerprint(&v)
}

/// Find a boundary that does not strand externally executed tool calls.
/// Reasoning, encrypted content, phase and unknown item fields are never edited.
pub fn safe_cut(items: &[Value], keep_turns: usize) -> Option<usize> {
    let users: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| is_user(i))
        .map(|(n, _)| n)
        .collect();
    if users.len() <= keep_turns || !items.last().is_some_and(is_user) {
        return None;
    }
    let wanted = users[users.len() - keep_turns];
    let mut pending = HashSet::<String>::new();
    let mut boundary = None;
    for (index, item) in items.iter().enumerate() {
        if index <= wanted && index > 0 && is_user(item) && pending.is_empty() {
            boundary = Some(index);
        }
        let ty = item["type"].as_str().unwrap_or("");
        let external_call = matches!(
            ty,
            "function_call"
                | "custom_tool_call"
                | "computer_call"
                | "local_shell_call"
                | "shell_call"
                | "apply_patch_call"
        );
        let external_output = matches!(
            ty,
            "function_call_output"
                | "custom_tool_call_output"
                | "computer_call_output"
                | "local_shell_call_output"
                | "shell_call_output"
                | "apply_patch_call_output"
        );
        if external_call {
            let id = item.get("call_id").or_else(|| item.get("id"))?.as_str()?;
            if !pending.insert(id.to_owned()) {
                return None;
            }
        } else if external_output {
            let id = item["call_id"].as_str()?;
            if !pending.remove(id) {
                return None;
            }
        }
    }
    // An unresolved call anywhere means this is not a closed user-turn boundary.
    if pending.is_empty() { boundary } else { None }
}

pub fn bytes(items: &[Value]) -> usize {
    serde_json::to_vec(items).unwrap().len()
}

/// The provider checkpoints the whole effective input; no tool pair is sliced.
pub fn complete_tool_return(items: &[Value]) -> bool {
    let output = |ty: &str| {
        matches!(
            ty,
            "function_call_output"
                | "custom_tool_call_output"
                | "computer_call_output"
                | "local_shell_call_output"
                | "shell_call_output"
                | "apply_patch_call_output"
        )
    };
    if !items
        .last()
        .and_then(|i| i["type"].as_str())
        .is_some_and(output)
    {
        return false;
    }
    let mut pending = HashSet::new();
    let mut seen = HashSet::new();
    for item in items {
        let ty = item["type"].as_str().unwrap_or("");
        if matches!(
            ty,
            "function_call"
                | "custom_tool_call"
                | "computer_call"
                | "local_shell_call"
                | "shell_call"
                | "apply_patch_call"
        ) {
            let Some(id) = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            else {
                return false;
            };
            if !seen.insert(id) || !pending.insert(id) {
                return false;
            }
        } else if output(ty) {
            let Some(id) = item["call_id"].as_str() else {
                return false;
            };
            if !pending.remove(id) {
                return false;
            }
        }
    }
    pending.is_empty()
}

pub fn plan(request: &Value, previous: &Lane, now: u64, cfg: &Config, force: bool) -> Plan {
    let raw = request["input"].as_array().expect("validated array");
    let hashes: Vec<String> = raw.iter().map(fingerprint).collect();
    let signature = contract(request);
    let first = previous.contract.is_empty();
    let changed = !first && signature != previous.contract;
    let diverged = !first && !hashes.starts_with(&previous.input_hashes);
    let bad_state = previous.cut > raw.len() || previous.cut > previous.input_hashes.len();
    let reset = changed || diverged || bad_state;
    let mut next = if reset {
        Lane {
            epoch: previous.epoch + 1,
            ..Default::default()
        }
    } else {
        previous.clone()
    };
    next.version = 1;
    next.contract = signature;
    next.input_hashes = hashes;
    let mut effective = next.projection.clone();
    effective.extend_from_slice(&raw[next.cut..]);
    let reason = if changed {
        "contract_reset"
    } else if diverged || bad_state {
        "history_reset"
    } else {
        "append"
    };
    let mut result = Plan {
        next,
        effective,
        compact_input: None,
        compact_cut: 0,
        reason,
    };
    // A client reset is authoritative. Forward its new history once before trying another roll.
    if reset {
        return result;
    }
    let pressure = bytes(&result.effective) >= cfg.roll_bytes;
    let idle = cfg.idle_roll_seconds > 0
        && previous.last_success_ms > 0
        && now.saturating_sub(previous.last_success_ms)
            >= cfg.idle_roll_seconds.saturating_mul(1000);
    if !force && !pressure && !idle {
        return result;
    }
    if !force
        && previous.last_roll_attempt_ms > 0
        && now.saturating_sub(previous.last_roll_attempt_ms)
            < cfg.min_roll_seconds.saturating_mul(1000)
    {
        result.reason = "roll_cooldown";
        return result;
    }
    if cfg.compaction_backend == CompactionBackend::Inline
        && cfg.inline_tool_boundaries
        && complete_tool_return(raw)
    {
        if bytes(&result.effective) >= cfg.min_compact_bytes || force {
            result.compact_input = Some(result.effective.clone());
            result.compact_cut = raw.len();
            result.next.last_roll_attempt_ms = now;
            result.reason = "complete_tool_roll";
        }
        return result;
    }
    let Some(cut) = safe_cut(raw, cfg.keep_recent_turns) else {
        result.reason = "defer_active_turn";
        return result;
    };
    if cut <= result.next.cut {
        return result;
    }
    let mut prefix = result.next.projection.clone();
    prefix.extend_from_slice(&raw[result.next.cut..cut]);
    if bytes(&prefix) < cfg.min_compact_bytes && !force {
        return result;
    }
    result.reason = if force {
        "forced_roll"
    } else if pressure {
        "pressure_roll"
    } else {
        "idle_heuristic_roll"
    };
    result.compact_input = Some(prefix);
    result.compact_cut = cut;
    result.next.last_roll_attempt_ms = now;
    result
}

impl Plan {
    pub fn accept(
        &mut self,
        output: &[Value],
        raw: &[Value],
        min_savings: f64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!output.is_empty(), "empty compact output");
        anyhow::ensure!(
            output.iter().any(|i| i["type"] == "compaction"
                && i["encrypted_content"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())),
            "compact output has no encrypted compaction item"
        );
        let before = bytes(
            self.compact_input
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no roll planned"))?,
        );
        anyhow::ensure!(
            (bytes(output) as f64) < before as f64 * (1.0 - min_savings),
            "insufficient compaction savings"
        );
        self.next.projection = output.to_vec(); // Entire canonical output, including retained items.
        self.next.cut = self.compact_cut;
        self.next.epoch += 1;
        self.effective = output.to_vec();
        self.effective.extend_from_slice(&raw[self.compact_cut..]);
        Ok(())
    }
}

/// Adopt an authoritative inline checkpoint only after the complete response.
/// The client next replays its original input plus the unmodified response output.
pub fn adopt_inline(
    lane: &mut Lane,
    output: &[Value],
    effective_bytes: usize,
    min_savings: f64,
) -> anyhow::Result<usize> {
    let positions: Vec<usize> = output
        .iter()
        .enumerate()
        .filter_map(|(index, item)| (item["type"] == "compaction").then_some(index))
        .collect();
    let Some(&index) = positions.last() else {
        return Ok(0);
    };
    let checkpoint = &output[index];
    anyhow::ensure!(
        checkpoint["encrypted_content"]
            .as_str()
            .is_some_and(|v| !v.is_empty()),
        "inline checkpoint has no encrypted content"
    );
    let before = effective_bytes.saturating_add(bytes(&output[..=index]));
    anyhow::ensure!(
        (bytes(std::slice::from_ref(checkpoint)) as f64) < before as f64 * (1.0 - min_savings),
        "insufficient inline checkpoint savings"
    );
    // Include the output prefix in the expected original history. The next request
    // must replay it exactly before the cut can safely skip it.
    lane.input_hashes
        .extend(output[..=index].iter().map(fingerprint));
    lane.cut = lane.input_hashes.len();
    lane.projection = vec![checkpoint.clone()];
    lane.epoch += positions.len() as u64;
    Ok(positions.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    fn cfg() -> Config {
        Config::parse_from([
            "test",
            "--roll-bytes",
            "300",
            "--min-compact-bytes",
            "10",
            "--keep-recent-turns",
            "1",
            "--min-roll-seconds",
            "0",
        ])
    }
    fn u(s: &str) -> Value {
        json!({"role":"user", "content":s})
    }
    fn transcript() -> Vec<Value> {
        vec![
            u(&"A".repeat(500)),
            json!({"type":"message", "role":"assistant", "phase":"final_answer", "content":"done"}),
            u("next"),
        ]
    }
    fn compacted() -> Vec<Value> {
        vec![
            u("retained"),
            json!({"type":"compaction", "encrypted_content":"opaque"}),
        ]
    }
    #[test]
    fn projection_is_frozen_and_tail_appends_across_turns() {
        let mut r = json!({"model":"gpt-5.5", "input":transcript()});
        let mut p = plan(&r, &Lane::default(), 1000, &cfg(), false);
        p.accept(&compacted(), r["input"].as_array().unwrap(), 0.1)
            .unwrap();
        let first = p.effective.clone();
        r["input"]
            .as_array_mut()
            .unwrap()
            .extend([json!({"role":"assistant", "content":"ok"}), u("again")]);
        let mut c = cfg();
        c.roll_bytes = 1_000_000;
        let q = plan(&r, &p.next, 2000, &c, false);
        assert!(q.effective.starts_with(&first));
        assert_eq!(q.next.projection, compacted());
        assert!(q.compact_input.is_none());
    }
    #[test]
    fn mid_tool_cycle_never_rolls_or_changes_items() {
        let mut r = json!({"model":"gpt-5.6", "input":transcript()});
        r["input"].as_array_mut().unwrap().extend([
            json!({"type":"reasoning", "id":"r", "encrypted_content":"secret", "summary":[]}),
            json!({"type":"function_call", "call_id":"c", "name":"read", "arguments":"{}"}),
            json!({"type":"function_call_output", "call_id":"c", "output":"file"}),
        ]);
        let p = plan(&r, &Lane::default(), 1000, &cfg(), true);
        assert!(p.compact_input.is_none());
        assert_eq!(json!(p.effective), r["input"]);
    }
    #[test]
    fn all_pending_calls_must_close_before_roll() {
        let a = vec![
            u("a"),
            json!({"type":"function_call","call_id":"1"}),
            json!({"type":"function_call","call_id":"2"}),
            json!({"type":"function_call_output","call_id":"1"}),
            u("b"),
        ];
        assert_eq!(safe_cut(&a, 1), None);
        let a = vec![
            u("a"),
            json!({"type":"function_call_output","call_id":"missing"}),
            u("b"),
        ];
        assert_eq!(safe_cut(&a, 1), None);
    }
    #[test]
    fn edited_history_and_tools_reset_without_reusing_projection() {
        let mut r = json!({"model":"gpt-5.5", "input":transcript(), "tools":[]});
        let mut p = plan(&r, &Lane::default(), 1000, &cfg(), false);
        p.accept(&compacted(), r["input"].as_array().unwrap(), 0.1)
            .unwrap();
        r["input"][0] = u("edited");
        let q = plan(&r, &p.next, 2000, &cfg(), true);
        assert_eq!(q.reason, "history_reset");
        assert!(q.next.projection.is_empty());
        r["tools"] = json!([{"type":"function", "name":"read"}]);
        assert_eq!(
            plan(&r, &p.next, 2000, &cfg(), true).reason,
            "contract_reset"
        );
    }
    #[test]
    fn same_length_content_changes_reset_and_rolls_are_recursive() {
        let mut r = json!({"input":transcript()});
        let mut p = plan(&r, &Lane::default(), 1000, &cfg(), true);
        p.accept(&compacted(), r["input"].as_array().unwrap(), 0.1)
            .unwrap();
        r["input"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":"B".repeat(500)}),
            u("third"),
        ]);
        let q = plan(&r, &p.next, 2000, &cfg(), true);
        assert!(q.compact_input.unwrap().starts_with(&compacted()));
        r["input"][0]["content"] = json!("Z".repeat(500));
        assert_eq!(
            plan(&r, &p.next, 2000, &cfg(), false).reason,
            "history_reset"
        );
    }
}
