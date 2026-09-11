//! Experimental Codex 0.154 Responses Lite ordering adapter.
//!
//! Trust boundary: a locally configured Codex client owns input[0], the generated
//! additional_tools prefix. Imported history is never searched for an inventory.
//! This is a format/provenance check, not authentication of arbitrary HTTP clients.
//! No state crosses a websocket connection; unknown response references fail closed.
use crate::{config::NativeToolBindingMode, fingerprint, hash};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Default)]
pub struct Connection {
    inventory: Option<Value>,
    model: Option<String>,
    scope: Option<String>,
    last_completed: Option<String>,
    in_flight: bool,
    compacting: bool,
    checkpoint_due: bool,
    pending_calls: HashSet<String>,
    pending_search_calls: HashSet<String>,
    output: BTreeMap<u64, OutputSummary>,
}

#[derive(Clone)]
struct OutputSummary {
    fingerprint: String,
    kind: String,
    call_id: Option<String>,
    search: Option<SearchTransition>,
    valid_checkpoint: bool,
}

#[derive(Clone)]
enum SearchTransition {
    Call(String),
    Output(String),
}

pub struct Prepared {
    pub body: Value,
    pub changed: bool,
    pub evidence: Value,
}

impl Connection {
    /// Full requests reset inventory, including an explicitly empty inventory.
    /// Incremental requests are accepted only after this connection's completion.
    pub fn prepare(&mut self, request: &Value, mode: NativeToolBindingMode) -> Result<Prepared> {
        ensure!(!self.in_flight, "NATIVE_BINDING_UNSUPPORTED_PIPELINING");
        ensure!(request.is_object(), "NATIVE_BINDING_REQUIRES_OBJECT");
        ensure!(
            request.get("type").is_none_or(|t| t == "response.create"),
            "NATIVE_BINDING_UNSUPPORTED_REQUEST_TYPE"
        );
        ensure!(
            request.get("conversation").is_none_or(Value::is_null),
            "NATIVE_BINDING_UNSUPPORTED_CONVERSATION_REFERENCE"
        );
        ensure!(
            request.get("context_management").is_none_or(Value::is_null),
            "NATIVE_BINDING_UNSUPPORTED_INLINE_ROLLING"
        );
        ensure!(
            request.get("background").is_none_or(|v| v != true),
            "NATIVE_BINDING_UNSUPPORTED_BACKGROUND"
        );
        let input = request["input"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_REQUIRES_INPUT_ARRAY"))?;
        ensure!(
            input.iter().all(Value::is_object),
            "NATIVE_BINDING_INVALID_ITEM"
        );
        ensure!(
            !input.iter().any(|i| matches!(
                i["type"].as_str(),
                Some("item_reference" | "context_compaction" | "configuration_update")
            )),
            "NATIVE_BINDING_UNSUPPORTED_RESET_OR_REFERENCE"
        );
        let model = request["model"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_REQUIRES_MODEL"))?;
        ensure!(
            self.model.as_deref().is_none_or(|bound| bound == model),
            "NATIVE_BINDING_CONNECTION_MODEL_CHANGED"
        );
        let scope = thread_scope(request)?;
        if self.inventory.is_some() || self.scope.is_some() {
            ensure!(
                self.scope == scope,
                "NATIVE_BINDING_CONNECTION_THREAD_CHANGED"
            );
        }
        let incremental = request.get("previous_response_id").filter(|v| !v.is_null());
        // Codex 0.154 compact_remote_v2_attempt appends exactly this request-only
        // control. It never replaces the native input or installs a checkpoint.
        let triggers = input
            .iter()
            .enumerate()
            .filter(|(_, i)| i["type"] == "compaction_trigger")
            .collect::<Vec<_>>();
        let compacting = !triggers.is_empty();
        if compacting {
            let metadata = turn_metadata(request)?;
            ensure!(
                metadata["request_kind"] == "compaction"
                    && scope.as_ref().is_some_and(|s| !s.is_empty()),
                "NATIVE_BINDING_UNVERIFIED_COMPACTION_REQUEST"
            );
            ensure!(
                triggers.len() == 1
                    && triggers[0].0 + 1 == input.len()
                    && triggers[0].1.as_object().unwrap().len() == 1,
                "NATIVE_BINDING_UNSUPPORTED_COMPACTION_TRIGGER"
            );
            ensure!(
                request.get("generate").is_none_or(|v| v != false),
                "NATIVE_BINDING_COMPACTION_WARMUP_UNSUPPORTED"
            );
        }
        let inventory = if let Some(previous) = incremental {
            ensure!(
                previous.as_str().is_some() && previous.as_str() == self.last_completed.as_deref(),
                "NATIVE_BINDING_UNKNOWN_PREVIOUS_RESPONSE"
            );
            ensure!(
                self.model.as_deref() == Some(model),
                "NATIVE_BINDING_INCREMENTAL_MODEL_CHANGED"
            );
            self.inventory
                .clone()
                .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_MISSING_RUNTIME_PREFIX"))?
        } else {
            let prefix = input
                .first()
                .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_MISSING_RUNTIME_PREFIX"))?;
            ensure!(
                prefix["type"] == "additional_tools"
                    && prefix["role"] == "developer"
                    && prefix["tools"].is_array(),
                "NATIVE_BINDING_MISSING_RUNTIME_PREFIX"
            );
            ensure!(
                prefix["id"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("at_") && s.len() > 6),
                "NATIVE_BINDING_UNVERIFIED_PREFIX_ID"
            );
            ensure!(
                input
                    .get(1)
                    .is_some_and(|i| i["type"] == "message" && i["role"] == "developer"),
                "NATIVE_BINDING_UNVERIFIED_PREFIX_LAYOUT"
            );
            // Keep only the actual runtime descriptor's fields; omit its already-used wire ID.
            let mut inventory = prefix.clone();
            inventory.as_object_mut().unwrap().remove("id");
            inventory
        };
        let mut checkpoints = Vec::new();
        for (n, item) in input.iter().enumerate() {
            if item["type"] == "compaction" {
                ensure!(
                    item["encrypted_content"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty()),
                    "NATIVE_BINDING_INVALID_CHECKPOINT"
                );
                checkpoints.push(n);
            }
        }
        let inherited_checkpoint = incremental.is_some() && self.checkpoint_due;
        let mut pending_calls = if incremental.is_some() {
            self.pending_calls.clone()
        } else {
            HashSet::new()
        };
        let mut pending_search_calls = if incremental.is_some() {
            self.pending_search_calls.clone()
        } else {
            HashSet::new()
        };
        for item in input {
            update_calls(&mut pending_calls, item)?;
            update_search_calls(&mut pending_search_calls, search_transition(item)?.as_ref())?;
        }
        ensure!(
            !compacting || (pending_calls.is_empty() && pending_search_calls.is_empty()),
            "NATIVE_BINDING_COMPACTION_WITH_PENDING_CALLS"
        );
        // Apart from the verified prefix, only an identical already-positioned
        // declaration is supported. Stale or arbitrary imported inventories fail.
        for (n, item) in input.iter().enumerate() {
            if item["type"] != "additional_tools" || (incremental.is_none() && n == 0) {
                continue;
            }
            let mut candidate = item.clone();
            candidate.as_object_mut().unwrap().remove("id");
            ensure!(
                candidate == inventory
                    && (checkpoints.iter().any(|c| n == c + 1 || n + 1 == *c)
                        || (inherited_checkpoint && n == 0)),
                "NATIVE_BINDING_UNTRUSTED_HISTORY_DECLARATION"
            );
        }
        let mut body = request.clone();
        let mut inserted = None;
        if let Some(last) = checkpoints.last() {
            let position = if mode == NativeToolBindingMode::RepeatBefore {
                *last
            } else {
                last + 1
            };
            let existing_position = if mode == NativeToolBindingMode::RepeatBefore {
                last.checked_sub(1)
            } else {
                Some(position)
            };
            let already_positioned =
                existing_position
                    .and_then(|n| input.get(n))
                    .is_some_and(|i| {
                        let mut candidate = i.clone();
                        if let Some(obj) = candidate.as_object_mut() {
                            obj.remove("id");
                        }
                        candidate == inventory
                    });
            if matches!(
                mode,
                NativeToolBindingMode::Rebind | NativeToolBindingMode::RepeatBefore
            ) && !already_positioned
            {
                body["input"]
                    .as_array_mut()
                    .unwrap()
                    .insert(position, inventory.clone());
                inserted = Some(position);
            }
        } else if inherited_checkpoint && mode == NativeToolBindingMode::Rebind {
            // A completed compaction can be referenced by a delta that contains
            // no checkpoint item. Its effective boundary precedes this input.
            let already_positioned = input.first().is_some_and(|item| {
                let mut candidate = item.clone();
                candidate.as_object_mut().unwrap().remove("id");
                candidate == inventory
            });
            if !already_positioned {
                body["input"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, inventory.clone());
                inserted = Some(0);
            }
        }
        let mut restored = body["input"].as_array().unwrap().clone();
        if let Some(index) = inserted {
            restored.remove(index);
        }
        ensure!(&restored == input, "NATIVE_BINDING_PRESERVATION_FAILURE");
        let evidence = json!({
            "mode":mode, "incremental":incremental.is_some(), "inserted_at":inserted,
            "compaction_trigger":compacting, "inherited_checkpoint":inherited_checkpoint,
            "pending_tool_calls":pending_calls.len() + pending_search_calls.len(),
            "pending_tool_search_calls":pending_search_calls.len(),
            "input_items":input.len(), "output_items":body["input"].as_array().unwrap().len(),
            "input_types":input.iter().map(|i| i["type"].as_str().unwrap_or("message")).collect::<Vec<_>>(),
            "input_item_hashes":input.iter().map(fingerprint).collect::<Vec<_>>(),
            "declaration_indices":input.iter().enumerate().filter_map(|(n,i)| (i["type"] == "additional_tools").then_some(n)).collect::<Vec<_>>(),
            "inventory_sha256":fingerprint(&inventory["tools"]),
            "tool_names":tool_names(&inventory["tools"]),
            "code_mode_declarations":code_mode_names(&inventory["tools"]),
            "inventory_contains":inventory_markers(&inventory["tools"]),
            "scope_sha256":scope.as_ref().map(|s| hash(s.as_bytes())),
            "input_sha256":fingerprint(&request["input"]), "preserved_input_sha256":fingerprint(&json!(restored)),
            "checkpoints_before":checkpoint_hashes(request), "checkpoints_after":checkpoint_hashes(&body),
            "preserved":true
        });
        self.inventory = Some(inventory);
        self.model = Some(model.to_string());
        if scope.is_some() {
            self.scope = scope;
        }
        self.last_completed = None;
        self.in_flight = true;
        self.compacting = compacting;
        self.checkpoint_due = false;
        self.pending_calls = pending_calls;
        self.pending_search_calls = pending_search_calls;
        self.output.clear();
        Ok(Prepared {
            body,
            changed: inserted.is_some(),
            evidence,
        })
    }

    pub fn observe_response(&mut self, event: &Value) -> Result<()> {
        if event["type"] == "response.output_item.done" {
            ensure!(self.in_flight, "NATIVE_BINDING_UNEXPECTED_OUTPUT");
            let index = event["output_index"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_MISSING_OUTPUT_INDEX"))?;
            let summary = output_summary(&event["item"])?;
            if let Some(previous) = self.output.get(&index) {
                ensure!(
                    previous.fingerprint == summary.fingerprint,
                    "NATIVE_BINDING_CONFLICTING_OUTPUT_ITEM"
                );
            }
            self.output.insert(index, summary);
        }
        if event["type"] == "response.completed" {
            ensure!(self.in_flight, "NATIVE_BINDING_UNEXPECTED_COMPLETION");
            // Some routes supply output only in the terminal event, others stream
            // output_item.done and leave the terminal output array empty.
            if self.output.is_empty() {
                if let Some(items) = event["response"]["output"].as_array() {
                    for (n, item) in items.iter().enumerate() {
                        self.output.insert(n as u64, output_summary(item)?);
                    }
                }
            } else if let Some(items) = event["response"]["output"]
                .as_array()
                .filter(|items| !items.is_empty())
            {
                ensure!(
                    items.len() == self.output.len()
                        && items.iter().enumerate().all(|(index, item)| self
                            .output
                            .get(&(index as u64))
                            .is_some_and(|summary| summary.fingerprint == fingerprint(item))),
                    "NATIVE_BINDING_CONFLICTING_COMPLETED_OUTPUT"
                );
            }
            let checkpoints = self
                .output
                .values()
                .filter(|i| i.kind == "compaction")
                .count();
            ensure!(
                !self.output.values().any(|i| i.kind == "context_compaction"),
                "NATIVE_BINDING_UNSUPPORTED_CONTEXT_COMPACTION_OUTPUT"
            );
            if self.compacting {
                ensure!(
                    self.output.len() == 1
                        && self.output.get(&0).is_some_and(|i| i.valid_checkpoint),
                    "NATIVE_BINDING_INVALID_COMPACTION_COMPLETION"
                );
                self.checkpoint_due = true;
                self.pending_calls.clear();
                self.pending_search_calls.clear();
            } else {
                ensure!(
                    checkpoints == 0,
                    "NATIVE_BINDING_UNEXPECTED_CHECKPOINT_OUTPUT"
                );
                for item in self.output.values() {
                    update_search_calls(&mut self.pending_search_calls, item.search.as_ref())?;
                    if matches!(item.kind.as_str(), "function_call" | "custom_tool_call") {
                        self.pending_calls.insert(
                            item.call_id
                                .clone()
                                .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_MISSING_CALL_ID"))?,
                        );
                    }
                }
            }
            self.last_completed = event["response"]["id"].as_str().map(str::to_string);
            ensure!(
                self.last_completed.is_some(),
                "NATIVE_BINDING_MISSING_RESPONSE_ID"
            );
            self.in_flight = false;
        } else if matches!(
            event["type"].as_str(),
            Some("error" | "response.failed" | "response.incomplete" | "response.cancelled")
        ) {
            self.inventory = None;
            self.last_completed = None;
            self.in_flight = false;
            self.checkpoint_due = false;
            self.output.clear();
            self.pending_calls.clear();
            self.pending_search_calls.clear();
            ensure!(
                event["type"] != "response.cancelled",
                "NATIVE_BINDING_UNSUPPORTED_CANCELLATION"
            );
        }
        Ok(())
    }
}

fn output_summary(item: &Value) -> Result<OutputSummary> {
    Ok(OutputSummary {
        fingerprint: fingerprint(item),
        kind: item["type"].as_str().unwrap_or("unknown").to_string(),
        call_id: item["call_id"].as_str().map(str::to_string),
        search: search_transition(item)?,
        valid_checkpoint: item["type"] == "compaction"
            && item["encrypted_content"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
    })
}

fn search_transition(item: &Value) -> Result<Option<SearchTransition>> {
    let kind = item["type"].as_str();
    if !matches!(kind, Some("tool_search_call" | "tool_search_output")) {
        return Ok(None);
    }
    // Codex 0.154 routes only execution="client" through its tool executor.
    // Native server searches can have null call IDs and need no client result.
    match item["execution"].as_str() {
        Some("server") => return Ok(None),
        Some("client") => {}
        _ => anyhow::bail!("NATIVE_BINDING_UNSUPPORTED_TOOL_SEARCH_EXECUTION"),
    }
    let id = item["call_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_MISSING_TOOL_SEARCH_CALL_ID"))?;
    if kind == Some("tool_search_call") {
        ensure!(
            item.get("arguments").is_some(),
            "NATIVE_BINDING_INVALID_TOOL_SEARCH_CALL"
        );
        Ok(Some(SearchTransition::Call(id.to_string())))
    } else {
        // Successful and aborted searches both emit this result envelope in
        // tools/context.rs; discovered schemas never replace runtime inventory.
        ensure!(
            item["status"] == "completed" && item["tools"].is_array(),
            "NATIVE_BINDING_INVALID_TOOL_SEARCH_OUTPUT"
        );
        Ok(Some(SearchTransition::Output(id.to_string())))
    }
}

fn update_search_calls(
    pending: &mut HashSet<String>,
    transition: Option<&SearchTransition>,
) -> Result<()> {
    match transition {
        Some(SearchTransition::Call(id)) => ensure!(
            pending.insert(id.clone()),
            "NATIVE_BINDING_DUPLICATE_TOOL_SEARCH_CALL"
        ),
        Some(SearchTransition::Output(id)) => {
            ensure!(
                pending.remove(id),
                "NATIVE_BINDING_UNMATCHED_TOOL_SEARCH_OUTPUT"
            )
        }
        None => {}
    }
    Ok(())
}

fn update_calls(pending: &mut HashSet<String>, item: &Value) -> Result<()> {
    match item["type"].as_str() {
        Some("function_call" | "custom_tool_call") => {
            let id = item["call_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_MISSING_CALL_ID"))?;
            pending.insert(id.to_string());
        }
        Some("function_call_output" | "custom_tool_call_output") => {
            let id = item["call_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("NATIVE_BINDING_MISSING_CALL_ID"))?;
            pending.remove(id);
        }
        _ => {}
    }
    Ok(())
}

fn thread_scope(request: &Value) -> Result<Option<String>> {
    Ok(turn_metadata(request)?["thread_id"]
        .as_str()
        .map(str::to_string))
}

fn turn_metadata(request: &Value) -> Result<Value> {
    let Some(raw) = request["client_metadata"]["x-codex-turn-metadata"].as_str() else {
        return Ok(Value::Null);
    };
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|_| anyhow::anyhow!("NATIVE_BINDING_INVALID_THREAD_METADATA"))?;
    Ok(parsed)
}

pub fn checkpoint_hashes(request: &Value) -> Vec<Value> {
    request["input"].as_array().into_iter().flatten().filter(|i| i["type"] == "compaction").map(|i| json!({
        "item_sha256":fingerprint(i), "encrypted_sha256":i["encrypted_content"].as_str().map(|s| hash(s.as_bytes()))
    })).collect()
}

fn tool_names(tools: &Value) -> Vec<String> {
    tools
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|t| {
            let name = t["name"].as_str().unwrap_or("unknown");
            if t["tools"].is_array() {
                tool_names(&t["tools"])
                    .into_iter()
                    .map(|n| format!("{name}.{n}"))
                    .collect()
            } else {
                vec![name.to_string()]
            }
        })
        .collect()
}

fn inventory_markers(tools: &Value) -> Value {
    let mut names = tool_names(tools);
    names.extend(code_mode_names(tools));
    let contains = |name: &str| {
        names
            .iter()
            .any(|n| n == name || n.ends_with(&format!(".{name}")))
    };
    json!({"exec_command":contains("exec_command"), "apply_patch":contains("apply_patch"), "view_image":contains("view_image"), "native_binding_alpha":contains("native_binding_alpha"), "native_binding_beta":contains("native_binding_beta")})
}

fn code_mode_names(value: &Value) -> Vec<String> {
    // Diagnostic only: Codex 0.154's generated declaration syntax. Do not count
    // incidental mentions of a removed tool in another tool's description.
    let mut names = Vec::new();
    match value {
        Value::String(text) => {
            for line in text.lines() {
                let Some((name, _)) = line
                    .strip_prefix("declare const tools: { ")
                    .and_then(|rest| rest.split_once('('))
                    .filter(|(name, _)| {
                        name.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                    })
                else {
                    continue;
                };
                names.push(name.to_string());
            }
        }
        Value::Array(items) => names.extend(items.iter().flat_map(code_mode_names)),
        Value::Object(fields) => names.extend(fields.values().flat_map(code_mode_names)),
        _ => {}
    }
    names.sort();
    names.dedup();
    names
}
