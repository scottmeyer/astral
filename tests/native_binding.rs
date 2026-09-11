use clap::{CommandFactory, Parser};
use ostk_gpt_cache::{
    config::{Config, NativeToolBindingMode},
    native_binding::Connection,
};
use serde_json::{Value, json};

fn prefix(name: &str) -> Value {
    json!({"type":"additional_tools","role":"developer","id":"at_runtime","tools":[{"type":"function","name":name,"parameters":{"type":"object"}}]})
}
fn checkpoint(id: &str) -> Value {
    json!({"type":"compaction","id":id,"encrypted_content":"synthetic+/=opaque\u{0000}bytes","unknown_native_field":{"nested":[1,true,null]}})
}
fn full(name: &str, tail: Vec<Value>) -> Value {
    let mut input = vec![
        prefix(name),
        json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"runtime base"}]}),
    ];
    input.extend(tail);
    json!({"model":"test-model","input":input,"stream":true,"tool_choice":"auto","unknown_request_field":[1,2]})
}
fn completed(state: &mut Connection, id: &str) {
    state
        .observe_response(&json!({"type":"response.completed","response":{"id":id}}))
        .unwrap();
}
fn apply(body: &Value) -> ostk_gpt_cache::native_binding::Prepared {
    Connection::default()
        .prepare(body, NativeToolBindingMode::Rebind)
        .unwrap()
}

#[test]
fn no_checkpoint_is_unchanged() {
    let request = full(
        "current",
        vec![json!({"type":"message","role":"user","content":"hello"})],
    );
    let out = apply(&request);
    assert!(!out.changed);
    assert_eq!(out.body, request);
}

#[test]
fn last_checkpoint_and_every_native_field_are_preserved() {
    let request = full(
        "current",
        vec![
            checkpoint("first"),
            checkpoint("last"),
            json!({"type":"custom_tool_call","call_id":"unchanged-call","name":"exec","input":"code"}),
            json!({"type":"custom_tool_call_output","call_id":"unchanged-call","output":"result"}),
            json!({"type":"reasoning","encrypted_content":"retain me","summary":[]}),
            json!({"type":"message","role":"assistant","phase":"final_answer","content":"tail"}),
        ],
    );
    let out = apply(&request);
    assert_eq!(out.body["input"][4]["type"], "additional_tools");
    let mut restored = out.body.clone();
    restored["input"].as_array_mut().unwrap().remove(4);
    assert_eq!(restored, request);
    assert_eq!(
        out.evidence["checkpoints_before"],
        out.evidence["checkpoints_after"]
    );
    assert_eq!(
        out.evidence["input_sha256"],
        out.evidence["preserved_input_sha256"]
    );
}

#[test]
fn one_checkpoint_is_idempotent_and_already_positioned_is_unchanged() {
    let request = full("current", vec![checkpoint("one")]);
    let once = apply(&request);
    let twice = apply(&once.body);
    assert!(once.changed);
    assert!(!twice.changed);
    assert_eq!(twice.body, once.body);
    assert!(twice.body["input"][3].get("id").is_none());
    assert_eq!(twice.body["input"][0]["id"], "at_runtime");
}

#[test]
fn latest_full_prefix_replaces_inventory_including_removal() {
    let mut state = Connection::default();
    state
        .prepare(&full("removed", vec![]), NativeToolBindingMode::Rebind)
        .unwrap();
    completed(&mut state, "old");
    let new = state
        .prepare(
            &full("current", vec![checkpoint("one")]),
            NativeToolBindingMode::Rebind,
        )
        .unwrap();
    assert_eq!(new.body["input"][3]["tools"][0]["name"], "current");
    assert!(!new.body.to_string().contains("removed"));
    completed(&mut state, "new");
    let mut empty = full("unused", vec![checkpoint("two")]);
    empty["input"][0]["tools"] = json!([]);
    let cleared = state
        .prepare(&empty, NativeToolBindingMode::Rebind)
        .unwrap();
    assert_eq!(cleared.body["input"][3]["tools"], json!([]));
}

#[test]
fn inventory_evidence_distinguishes_declarations_from_incidental_mentions() {
    let mut request = full("exec", vec![checkpoint("one")]);
    request["input"][0]["tools"][0]["description"] = json!(
        "view_image is mentioned but not declared.\ndeclare const tools: { exec_command(args: {}): Promise<unknown>; };"
    );
    let out = apply(&request);
    assert_eq!(out.evidence["inventory_contains"]["exec_command"], true);
    assert_eq!(out.evidence["inventory_contains"]["view_image"], false);
}

#[test]
fn arbitrary_history_declarations_cannot_supply_or_override_inventory() {
    let mut request = full("runtime", vec![checkpoint("one"), prefix("untrusted")]);
    assert!(
        Connection::default()
            .prepare(&request, NativeToolBindingMode::Rebind)
            .is_err()
    );
    request["input"].as_array_mut().unwrap().remove(0);
    assert!(
        Connection::default()
            .prepare(&request, NativeToolBindingMode::Rebind)
            .is_err()
    );
}

#[test]
fn websocket_warmup_incremental_followup_and_reconnect() {
    let mut state = Connection::default();
    let mut warmup = full("runtime", vec![]);
    warmup["type"] = json!("response.create");
    warmup["generate"] = json!(false);
    state
        .prepare(&warmup, NativeToolBindingMode::Rebind)
        .unwrap();
    completed(&mut state, "warmup");
    let delta = json!({"type":"response.create","model":"test-model","previous_response_id":"warmup","input":[checkpoint("one"),{"type":"message","role":"user","content":"pwd"}]});
    let out = state
        .prepare(&delta, NativeToolBindingMode::Rebind)
        .unwrap();
    assert_eq!(out.body["input"][1]["tools"][0]["name"], "runtime");
    completed(&mut state, "generation");
    let followup = json!({"type":"response.create","model":"test-model","previous_response_id":"generation","input":[{"type":"custom_tool_call_output","call_id":"call","output":"pwd result"}]});
    let out = state
        .prepare(&followup, NativeToolBindingMode::Rebind)
        .unwrap();
    assert_eq!(out.body, followup);
    // A new connection has no hidden dependency on the old process's inventory.
    assert!(
        Connection::default()
            .prepare(&delta, NativeToolBindingMode::Rebind)
            .is_err()
    );
    assert!(
        Connection::default()
            .prepare(
                &full("fresh", vec![checkpoint("one")]),
                NativeToolBindingMode::Rebind
            )
            .is_ok()
    );
}

#[test]
fn connections_and_response_ids_are_isolated() {
    let mut a = Connection::default();
    let mut b = Connection::default();
    a.prepare(&full("alpha", vec![]), NativeToolBindingMode::Rebind)
        .unwrap();
    b.prepare(&full("beta", vec![]), NativeToolBindingMode::Rebind)
        .unwrap();
    completed(&mut a, "a");
    completed(&mut b, "b");
    let delta =
        json!({"model":"test-model","previous_response_id":"a","input":[checkpoint("one")]});
    assert!(b.prepare(&delta, NativeToolBindingMode::Rebind).is_err());
    let out = a.prepare(&delta, NativeToolBindingMode::Rebind).unwrap();
    assert_eq!(out.body["input"][1]["tools"][0]["name"], "alpha");
}

#[test]
fn changing_or_omitting_bound_thread_scope_is_rejected() {
    let mut request = full("runtime", vec![]);
    request["client_metadata"] = json!({"x-codex-turn-metadata":"{\"thread_id\":\"alpha\"}"});
    let mut state = Connection::default();
    state
        .prepare(&request, NativeToolBindingMode::Rebind)
        .unwrap();
    completed(&mut state, "a");
    let mut delta =
        json!({"model":"test-model","previous_response_id":"a","input":[checkpoint("one")]});
    assert!(
        state
            .prepare(&delta, NativeToolBindingMode::Rebind)
            .is_err()
    );
    delta["client_metadata"] = json!({"x-codex-turn-metadata":"{\"thread_id\":\"beta\"}"});
    assert!(
        state
            .prepare(&delta, NativeToolBindingMode::Rebind)
            .is_err()
    );
    delta["client_metadata"] = request["client_metadata"].clone();
    assert!(state.prepare(&delta, NativeToolBindingMode::Rebind).is_ok());
}

#[test]
fn failures_pipelining_and_unsupported_representations_fail_closed() {
    let mut state = Connection::default();
    let request = full("runtime", vec![]);
    state
        .prepare(&request, NativeToolBindingMode::Rebind)
        .unwrap();
    assert!(
        state
            .prepare(&request, NativeToolBindingMode::Rebind)
            .is_err()
    );
    state
        .observe_response(&json!({"type":"response.failed"}))
        .unwrap();
    assert!(state.prepare(&json!({"model":"test-model","previous_response_id":"unknown","input":[checkpoint("one")]}),NativeToolBindingMode::Rebind).is_err());
    for item in [
        json!({"type":"item_reference","id":"hidden"}),
        json!({"type":"context_compaction"}),
        json!({"type":"configuration_update"}),
        json!({"type":"compaction","encrypted_content":""}),
    ] {
        assert!(
            Connection::default()
                .prepare(&full("runtime", vec![item]), NativeToolBindingMode::Rebind)
                .is_err()
        );
    }
    let mut changed = request.clone();
    changed["background"] = json!(true);
    assert!(
        Connection::default()
            .prepare(&changed, NativeToolBindingMode::Rebind)
            .is_err()
    );
    changed = request;
    changed["model"] = json!("other-model");
    assert!(
        state
            .prepare(&changed, NativeToolBindingMode::Rebind)
            .is_err()
    );
}

#[test]
fn explicit_modes_preserve_negative_controls() {
    let request = full("runtime", vec![checkpoint("one")]);
    let observed = Connection::default()
        .prepare(&request, NativeToolBindingMode::Observe)
        .unwrap();
    assert_eq!(observed.body, request);
    let before = Connection::default()
        .prepare(&request, NativeToolBindingMode::RepeatBefore)
        .unwrap();
    assert_eq!(before.body["input"][2]["type"], "additional_tools");
    assert_eq!(before.body["input"][3]["type"], "compaction");
    let twice = Connection::default()
        .prepare(&before.body, NativeToolBindingMode::RepeatBefore)
        .unwrap();
    assert!(!twice.changed);
    assert_eq!(twice.body, before.body);
}

#[test]
fn semantic_cli_name_keeps_the_hidden_legacy_alias_and_disabled_default() {
    assert_eq!(
        Config::parse_from(["proxy"]).native_tool_binding,
        NativeToolBindingMode::Disabled
    );
    for (value, expected) in [
        ("disabled", NativeToolBindingMode::Disabled),
        ("observe", NativeToolBindingMode::Observe),
        ("repeat-before", NativeToolBindingMode::RepeatBefore),
        ("rebind", NativeToolBindingMode::Rebind),
    ] {
        for flag in ["--native-tool-binding", "--ast000-compat"] {
            let config = Config::parse_from(["proxy", "--mode", "passthrough", flag, value]);
            assert_eq!(config.native_tool_binding, expected);
            config.validate().unwrap();
        }
    }
    let help = Config::command().render_long_help().to_string();
    assert!(help.contains("--native-tool-binding"));
    assert!(!help.contains("--ast000-compat"));
}

#[test]
fn configuration_requires_no_rolling_and_local_listener() {
    assert!(
        Config::parse_from(["proxy", "--native-tool-binding", "rebind"])
            .validate()
            .is_err()
    );
    assert!(
        Config::parse_from([
            "proxy",
            "--native-tool-binding",
            "rebind",
            "--mode",
            "passthrough",
            "--listen",
            "0.0.0.0:8888"
        ])
        .validate()
        .is_err()
    );
    assert!(
        Config::parse_from([
            "proxy",
            "--native-tool-binding",
            "rebind",
            "--mode",
            "passthrough"
        ])
        .validate()
        .is_ok()
    );
}

fn metadata(request: &mut Value, kind: &str) {
    request["client_metadata"] = json!({"x-codex-turn-metadata":
        json!({"thread_id":"fixture-thread", "request_kind":kind}).to_string()});
}

fn compact_request() -> Value {
    let mut request = full(
        "current",
        vec![checkpoint("old"), json!({"type":"compaction_trigger"})],
    );
    metadata(&mut request, "compaction");
    request
}

fn native_completed(state: &mut Connection, id: &str) {
    state
        .observe_response(
            &json!({"type":"response.output_item.done","output_index":0,"item":checkpoint(id)}),
        )
        .unwrap();
    state
        .observe_response(&json!({"type":"response.completed","response":{"id":id,"output":[]}}))
        .unwrap();
}

fn delta(previous: &str, input: Vec<Value>) -> Value {
    let mut request = json!({"model":"test-model","previous_response_id":previous,"input":input});
    metadata(&mut request, "turn");
    request
}

#[test]
fn completed_native_compaction_rebinds_the_next_delta_once() {
    let mut state = Connection::default();
    let request = compact_request();
    let out = state
        .prepare(&request, NativeToolBindingMode::Rebind)
        .unwrap();
    assert_eq!(
        out.body["input"].as_array().unwrap().last().unwrap(),
        &json!({"type":"compaction_trigger"})
    );
    native_completed(&mut state, "new");
    let tail = json!({"type":"message","role":"user","content":"continue"});
    let input = delta("new", vec![tail.clone()]);
    let mut replay = state.clone();
    let out = state
        .prepare(&input, NativeToolBindingMode::Rebind)
        .unwrap();
    assert_eq!(out.body["input"][0]["tools"][0]["name"], "current");
    assert_eq!(out.body["input"][1], tail);
    assert_eq!(out.evidence["inherited_checkpoint"], true);
    assert!(
        !replay
            .prepare(&out.body, NativeToolBindingMode::Rebind)
            .unwrap()
            .changed
    );
    completed(&mut state, "continued");
    let followup = delta("continued", vec![]);
    assert_eq!(
        state
            .prepare(&followup, NativeToolBindingMode::Rebind)
            .unwrap()
            .body,
        followup
    );
}

#[test]
fn repeated_native_cycles_and_cold_resume_use_current_inventory() {
    let mut state = Connection::default();
    for id in ["first", "second"] {
        let request = compact_request();
        state
            .prepare(&request, NativeToolBindingMode::Rebind)
            .unwrap();
        native_completed(&mut state, id);
        let out = state
            .prepare(
                &delta(id, vec![checkpoint(id)]),
                NativeToolBindingMode::Rebind,
            )
            .unwrap();
        assert_eq!(out.body["input"][1]["type"], "additional_tools");
        completed(&mut state, "work");
    }
    let mut cold = full("replacement", vec![checkpoint("second")]);
    metadata(&mut cold, "turn");
    assert_eq!(
        apply(&cold).body["input"][3]["tools"][0]["name"],
        "replacement"
    );
    cold["input"][0]["tools"] = json!([]);
    assert_eq!(
        state
            .prepare(&cold, NativeToolBindingMode::Rebind)
            .unwrap()
            .body["input"][3]["tools"],
        json!([])
    );
}

#[test]
fn unverified_or_malformed_compaction_controls_are_rejected() {
    let valid = compact_request();
    for mutation in 0..6 {
        let mut request = valid.clone();
        match mutation {
            0 => {
                request.as_object_mut().unwrap().remove("client_metadata");
            }
            1 => metadata(&mut request, "turn"),
            2 => {
                request["input"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"type":"message"}));
            }
            3 => {
                request["input"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"type":"compaction_trigger"}));
            }
            4 => request["input"][3]["unknown"] = json!(true),
            _ => request["generate"] = json!(false),
        }
        assert!(
            Connection::default()
                .prepare(&request, NativeToolBindingMode::Rebind)
                .is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn compaction_waits_for_pending_call_outputs_in_full_and_incremental_history() {
    let call = json!({"type":"function_call","call_id":"pending","name":"exec","arguments":"{}"});
    let output = json!({"type":"function_call_output","call_id":"pending","output":"done"});
    let mut request = compact_request();
    request["input"]
        .as_array_mut()
        .unwrap()
        .insert(3, call.clone());
    assert!(
        Connection::default()
            .prepare(&request, NativeToolBindingMode::Rebind)
            .is_err()
    );
    request["input"]
        .as_array_mut()
        .unwrap()
        .insert(4, output.clone());
    assert!(
        Connection::default()
            .prepare(&request, NativeToolBindingMode::Rebind)
            .is_ok()
    );
    let mut state = Connection::default();
    let mut initial = full("current", vec![]);
    metadata(&mut initial, "turn");
    state
        .prepare(&initial, NativeToolBindingMode::Rebind)
        .unwrap();
    state
        .observe_response(
            &json!({"type":"response.completed","response":{"id":"call","output":[call]}}),
        )
        .unwrap();
    let mut request = delta("call", vec![json!({"type":"compaction_trigger"})]);
    metadata(&mut request, "compaction");
    assert!(
        state
            .prepare(&request, NativeToolBindingMode::Rebind)
            .is_err()
    );
    request["input"].as_array_mut().unwrap().insert(0, output);
    assert!(
        state
            .prepare(&request, NativeToolBindingMode::Rebind)
            .is_ok()
    );
}

#[test]
fn invalid_native_completion_never_establishes_a_checkpoint_reference() {
    for outputs in [
        json!([]),
        json!([checkpoint("a"), checkpoint("b")]),
        json!([{"type":"compaction","encrypted_content":""}]),
        json!([{"type":"context_compaction","encrypted_content":"opaque"}]),
    ] {
        let mut state = Connection::default();
        state
            .prepare(&compact_request(), NativeToolBindingMode::Rebind)
            .unwrap();
        assert!(
            state
                .observe_response(
                    &json!({"type":"response.completed","response":{"id":"bad","output":outputs}})
                )
                .is_err()
        );
        assert!(
            state
                .prepare(&delta("bad", vec![]), NativeToolBindingMode::Rebind)
                .is_err()
        );
    }
    let mut state = Connection::default();
    state
        .prepare(&compact_request(), NativeToolBindingMode::Rebind)
        .unwrap();
    state
        .observe_response(
            &json!({"type":"response.output_item.done","output_index":0,"item":checkpoint("a")}),
        )
        .unwrap();
    assert!(state.observe_response(&json!({"type":"response.completed","response":{"id":"conflict","output":[checkpoint("b")]}})).is_err());
}

#[test]
fn compaction_failure_cancellation_and_other_session_cannot_supply_inventory() {
    for event in [
        "response.failed",
        "response.incomplete",
        "response.cancelled",
    ] {
        let mut state = Connection::default();
        state
            .prepare(&compact_request(), NativeToolBindingMode::Rebind)
            .unwrap();
        let result = state.observe_response(&json!({"type":event}));
        assert_eq!(result.is_err(), event == "response.cancelled");
        assert!(
            state
                .prepare(&delta("uncommitted", vec![]), NativeToolBindingMode::Rebind)
                .is_err()
        );
    }
    let mut state = Connection::default();
    state
        .prepare(&compact_request(), NativeToolBindingMode::Rebind)
        .unwrap();
    native_completed(&mut state, "good");
    let request = delta("good", vec![]);
    assert!(
        Connection::default()
            .prepare(&request, NativeToolBindingMode::Rebind)
            .is_err()
    );
    let mut changed = request.clone();
    changed["model"] = json!("other-model");
    assert!(
        state
            .prepare(&changed, NativeToolBindingMode::Rebind)
            .is_err()
    );
    changed = request;
    changed["client_metadata"] = json!({"x-codex-turn-metadata":"{\"thread_id\":\"other\"}"});
    assert!(
        state
            .prepare(&changed, NativeToolBindingMode::Rebind)
            .is_err()
    );
}

fn client_search_call(id: &str) -> Value {
    json!({"type":"tool_search_call","call_id":id,"execution":"client","arguments":{"query":"calendar","limit":1}})
}

fn client_search_output(id: &str) -> Value {
    json!({"type":"tool_search_output","call_id":id,"execution":"client","status":"completed","tools":[]})
}

fn compact_with_search(items: Vec<Value>) -> Value {
    let mut request = compact_request();
    request["input"].as_array_mut().unwrap().splice(3..3, items);
    request
}

#[test]
fn native_client_search_must_finish_before_full_history_compaction() {
    let call = client_search_call("search");
    let request = compact_with_search(vec![call.clone()]);
    let error = Connection::default()
        .prepare(&request, NativeToolBindingMode::Rebind)
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "NATIVE_BINDING_COMPACTION_WITH_PENDING_CALLS"
    );

    let mut output = client_search_output("search");
    output["tools"] = json!([{"type":"function","name":"discovered","parameters":{}}]);
    let request = compact_with_search(vec![call, output.clone()]);
    let prepared = apply(&request);
    assert_eq!(prepared.evidence["pending_tool_calls"], 0);
    assert_eq!(prepared.evidence["pending_tool_search_calls"], 0);
    assert_eq!(prepared.body["input"][3]["tools"][0]["name"], "current");
    assert_eq!(prepared.body["input"][5], output);
    let mut restored = prepared.body;
    restored["input"].as_array_mut().unwrap().remove(3);
    assert_eq!(restored, request);
}

#[test]
fn native_client_search_must_finish_before_incremental_compaction() {
    for streamed in [false, true] {
        let mut state = Connection::default();
        let mut initial = full("current", vec![]);
        metadata(&mut initial, "turn");
        state
            .prepare(&initial, NativeToolBindingMode::Rebind)
            .unwrap();
        let call = client_search_call("search");
        if streamed {
            state
                .observe_response(
                    &json!({"type":"response.output_item.done","output_index":0,"item":call}),
                )
                .unwrap();
            completed(&mut state, "search-response");
        } else {
            state.observe_response(&json!({"type":"response.completed","response":{"id":"search-response","output":[call]}})).unwrap();
        }
        let mut compact = delta(
            "search-response",
            vec![json!({"type":"compaction_trigger"})],
        );
        metadata(&mut compact, "compaction");
        let error = state
            .prepare(&compact, NativeToolBindingMode::Rebind)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "NATIVE_BINDING_COMPACTION_WITH_PENDING_CALLS"
        );

        // Neither a result of the wrong native kind nor a server result can
        // discharge the client search merely by reusing its call ID.
        for wrong in [
            json!({"type":"function_call_output","call_id":"search","output":"done"}),
            json!({"type":"custom_tool_call_output","call_id":"search","output":"done"}),
            json!({"type":"tool_search_output","execution":"server","call_id":"search","status":"completed","tools":[]}),
            client_search_output("other-search"),
        ] {
            let mut request = compact.clone();
            request["input"].as_array_mut().unwrap().insert(0, wrong);
            assert!(
                state
                    .clone()
                    .prepare(&request, NativeToolBindingMode::Rebind)
                    .is_err()
            );
        }

        compact["input"]
            .as_array_mut()
            .unwrap()
            .insert(0, client_search_output("search"));
        let prepared = state
            .prepare(&compact, NativeToolBindingMode::Rebind)
            .unwrap();
        assert_eq!(prepared.evidence["pending_tool_calls"], 0);
        native_completed(&mut state, "compacted");
        let continued = state
            .prepare(&delta("compacted", vec![]), NativeToolBindingMode::Rebind)
            .unwrap();
        assert_eq!(continued.body["input"][0]["tools"][0]["name"], "current");
    }
}

#[test]
fn malformed_client_search_pairs_cannot_authorize_compaction() {
    let call = client_search_call("search");
    let output = client_search_output("search");
    let mut missing_id = output.clone();
    missing_id.as_object_mut().unwrap().remove("call_id");
    let mut empty_id = output.clone();
    empty_id["call_id"] = json!("");
    let mut unfinished = output.clone();
    unfinished["status"] = json!("in_progress");
    let mut missing_tools = output.clone();
    missing_tools.as_object_mut().unwrap().remove("tools");
    let mut unknown_execution = output.clone();
    unknown_execution["execution"] = json!("other");
    let mut missing_execution = output.clone();
    missing_execution
        .as_object_mut()
        .unwrap()
        .remove("execution");
    for malformed in [
        missing_id,
        empty_id,
        unfinished,
        missing_tools,
        unknown_execution,
        missing_execution,
        client_search_output("wrong-id"),
    ] {
        let request = compact_with_search(vec![call.clone(), malformed]);
        assert!(
            Connection::default()
                .prepare(&request, NativeToolBindingMode::Rebind)
                .is_err()
        );
    }
    for items in [
        vec![call.clone(), call.clone(), output.clone()],
        vec![call.clone(), output.clone(), output.clone()],
        vec![output.clone(), call.clone()],
        vec![
            json!({"type":"function_call","call_id":"search","name":"exec","arguments":"{}"}),
            output,
        ],
    ] {
        assert!(
            Connection::default()
                .prepare(&compact_with_search(items), NativeToolBindingMode::Rebind)
                .is_err()
        );
    }
}

#[test]
fn malformed_native_client_search_calls_reject_input_and_response() {
    for mutation in 0..4 {
        let mut call = client_search_call("search");
        match mutation {
            0 => call["call_id"] = json!(null),
            1 => call["call_id"] = json!(""),
            2 => call["execution"] = json!("unknown"),
            _ => {
                call.as_object_mut().unwrap().remove("arguments");
            }
        }
        assert!(
            Connection::default()
                .prepare(
                    &compact_with_search(vec![call.clone()]),
                    NativeToolBindingMode::Rebind
                )
                .is_err()
        );
        for streamed in [false, true] {
            let mut state = Connection::default();
            state
                .prepare(&full("current", vec![]), NativeToolBindingMode::Rebind)
                .unwrap();
            let event = if streamed {
                json!({"type":"response.output_item.done","output_index":0,"item":call})
            } else {
                json!({"type":"response.completed","response":{"id":"bad","output":[call]}})
            };
            assert!(state.observe_response(&event).is_err());
        }
    }
}

#[test]
fn server_tool_search_does_not_require_a_client_result() {
    // Matching Codex protocol tests explicitly allow server searches without
    // call IDs. Their execution is provider-owned, not a pending local action.
    let call = json!({"type":"tool_search_call","execution":"server","call_id":null,"status":"completed","arguments":{"paths":["crm"]}});
    let output = json!({"type":"tool_search_output","execution":"server","call_id":null,"status":"completed","tools":[]});
    assert!(
        Connection::default()
            .prepare(
                &compact_with_search(vec![call.clone(), output.clone()]),
                NativeToolBindingMode::Rebind
            )
            .is_ok()
    );
    let mut state = Connection::default();
    let mut initial = full("current", vec![]);
    metadata(&mut initial, "turn");
    state
        .prepare(&initial, NativeToolBindingMode::Rebind)
        .unwrap();
    state.observe_response(&json!({"type":"response.completed","response":{"id":"server-search","output":[call,output]}})).unwrap();
    let mut compact = delta("server-search", vec![json!({"type":"compaction_trigger"})]);
    metadata(&mut compact, "compaction");
    assert!(
        state
            .prepare(&compact, NativeToolBindingMode::Rebind)
            .is_ok()
    );
}
