#!/usr/bin/env python3
"""Private, disposable Codex app-server controls for the opt-in AST-000 adapter.

No credential reads, transcript rewrites, raw response logs, or plaintext handoff.
Each recall runs on an ephemeral fork so its answer cannot contaminate later
probes of the parent. Only hashes and verification booleans leave memory.
"""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import queue
import sys
import time

sys.dont_write_bytecode = True


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def stable(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def without_opaque(value):
    if isinstance(value, dict):
        return {k: without_opaque(v) for k, v in value.items() if k != "encrypted_content"}
    if isinstance(value, list):
        return [without_opaque(v) for v in value]
    return value


def capture_native_history(rows):
    """Installed 0.154 rollout shape; fail rather than guess inherited history.

    Preserve the full source records and compaction metadata privately, alongside
    the native model-item window usable by thread/inject_items. Runtime settings
    and metadata sidecars are not injected as conversational messages.
    """
    if not rows or rows[0].get("type") != "session_meta":
        raise ValueError("native capture requires session metadata")
    items = None if rows[0]["payload"].get("history_base") else []
    boundary = None
    for ordinal, row in enumerate(rows):
        if row.get("type") == "compacted":
            replacement = row["payload"].get("replacement_history")
            if not isinstance(replacement, list):
                raise ValueError("unsupported plaintext or missing replacement history")
            items = list(replacement)
            boundary = ordinal
        elif row.get("type") == "response_item" and items is not None:
            items.append(row["payload"])
    if items is None:
        raise ValueError("capture cannot resolve an inherited history base")
    checkpoints = [i for i in items if i.get("type") == "compaction"]
    if not checkpoints or any(not i.get("encrypted_content") for i in checkpoints):
        raise ValueError("capture requires a native checkpoint")
    if any(i.get("type") in ["compaction_trigger", "context_compaction", "configuration_update", "item_reference"] for i in items):
        raise ValueError("unsupported native capture control")
    pending = {}
    for item in items:
        kind = item.get("type")
        if kind in ["tool_search_call", "tool_search_output"]:
            # The trusted runtime owns tool discovery. Never capture an
            # unresolved client search as a safe continuation boundary.
            execution = item.get("execution")
            if execution not in ["client", "server"]:
                raise ValueError("unsupported native tool search execution")
            if execution == "server":
                continue
            if kind == "tool_search_call" and "arguments" not in item:
                raise ValueError("invalid client tool search call")
            if kind == "tool_search_output" and (
                item.get("status") != "completed" or not isinstance(item.get("tools"), list)
            ):
                raise ValueError("invalid client tool search output")
        if kind in ["custom_tool_call", "function_call", "tool_search_call"]:
            call_id = item.get("call_id")
            if not isinstance(call_id, str) or not call_id or call_id in pending:
                raise ValueError("missing or duplicate native call id")
            pending[call_id] = {
                "custom_tool_call": "custom_tool_call_output",
                "function_call": "function_call_output",
                "tool_search_call": "tool_search_output",
            }[kind]
        elif kind in ["custom_tool_call_output", "function_call_output", "tool_search_output"]:
            call_id = item.get("call_id")
            if not isinstance(call_id, str) or pending.get(call_id) != kind:
                raise ValueError("orphan or mismatched native tool output")
            del pending[call_id]
    if pending:
        raise ValueError("capture has unresolved tool calls")
    return {"format": "astral-ast000-native-fixture-v1", "source_thread": rows[0]["payload"]["id"],
            "source_record_count": len(rows), "last_record_sha256": stable(rows[-1]),
            "compacted_record_ordinal": boundary, "source_records": rows,
            "items": items, "items_sha256": stable(items),
            "checkpoint_hashes": [{"item_sha256": stable(i), "encrypted_sha256": digest(i["encrypted_content"])} for i in checkpoints]}


def denied_tool_results(rows, turn_id, command):
    """Some sandbox failures have a tool result but no commandExecution event."""
    active = False
    calls = set()
    found = []
    for row in rows:
        item = row.get("payload", {})
        if row.get("type") == "event_msg" and item.get("type") == "task_started":
            active = item.get("turn_id") == turn_id
        if not active or row.get("type") != "response_item":
            continue
        if item.get("type") == "custom_tool_call" and command in item.get("input", ""):
            calls.add(item.get("call_id"))
        if item.get("type") == "custom_tool_call_output" and item.get("call_id") in calls:
            for block in item.get("output", []):
                try:
                    value = json.loads(block.get("text", ""))
                except (ValueError, AttributeError):
                    continue
                if isinstance(value, dict) and value.get("exit_code") not in (None, 0):
                    output = value.get("output", "")
                    if "operation not permitted" in output.lower() or "permission denied" in output.lower():
                        found.append({"call_id": item["call_id"], "command": command,
                                      "exit_code": value["exit_code"], "permission_denied": True,
                                      "output_sha256": digest(output)})
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["fresh", "import", "resume", "compact", "cycle"])
    parser.add_argument("--case", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--capsule", type=Path, required=True)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--thread")
    parser.add_argument("--recall", action="store_true")
    parser.add_argument("--marker", choices=["ast000_alpha", "ast000_beta"])
    parser.add_argument("--disable-view-image", action="store_true")
    parser.add_argument("--restricted", action="store_true")
    parser.add_argument("--extra-checkpoint", type=Path)
    parser.add_argument("--native-capture", type=Path,
                        help="Import the native item window from a private capture; no readable handoff")
    parser.add_argument("--capture", action="store_true", help="Privately capture the final native window and rollout")
    parser.add_argument("--turns", type=int, default=1)
    parser.add_argument("--pause-after-first-turn", action="store_true",
                        help="Wait for a private .continue marker so the controller can restart Astral")
    parser.add_argument("--codex", default="codex")
    args = parser.parse_args()
    if args.action == "cycle" and args.turns < 2:
        parser.error("cycle requires --turns 2 or greater")
    os.umask(0o077)
    args.output = args.output.resolve()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    if not args.case.replace("-", "").replace("_", "").isalnum():
        parser.error("case must be a simple filename component")
    log_path = args.output / (args.case + ".events.jsonl")
    log = log_path.open("x")
    result = {"case": args.case, "action": args.action, "status": "STARTED", "execution": [],
              "recall": {"status": "NOT_TESTED"}, "base_url": args.base_url,
              "sandbox": "read-only" if args.restricted else "workspace-write",
              "approval_policy": "never", "view_image_enabled": not args.disable_view_image}

    def record(event):
        log.write(json.dumps(event) + "\n")
        log.flush()

    record(result)
    work = args.output / "fixture/workspace"
    work.mkdir(parents=True, exist_ok=True)
    protected = work / "protected.txt"
    if not protected.exists():
        protected.write_text("AST000_UNCHANGED\n")
    protected_before = hashlib.sha256(protected.read_bytes()).hexdigest()
    sys.path.insert(0, str(args.capsule.resolve()))
    spec = importlib.util.spec_from_file_location("ast000_source_importer", args.capsule / "handoff.py")
    importer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(importer)
    manifest, window = importer.load_capsule(args.capsule)
    if args.native_capture:
        if args.action != "import":
            raise ValueError("native capture is supported only for a new fixture import")
        capture = json.loads(args.native_capture.read_text())
        verified = capture_native_history(capture["source_records"])
        if capture["items"] != verified["items"] or capture["items_sha256"] != verified["items_sha256"]:
            raise ValueError("native capture integrity mismatch")
        window = capture["items"]
        result["capture_lineage"] = {"source_thread": capture["source_thread"], "items_sha256": capture["items_sha256"]}
    if args.extra_checkpoint:
        extra = json.loads(args.extra_checkpoint.read_text())
        if extra.get("type") != "compaction" or not extra.get("encrypted_content"):
            raise ValueError("extra checkpoint must be an unmodified native compaction item")
        window = [window[0], extra, *window[1:]]
    result["window_sha256"] = stable(window)
    result["checkpoint_hashes"] = [{"item_sha256": stable(i), "encrypted_sha256": digest(i["encrypted_content"])}
                                   for i in window if i.get("type") == "compaction"]
    index = args.output / "fixture-threads.jsonl"
    known = [json.loads(l) for l in index.read_text().splitlines()] if index.exists() else []
    if args.thread and args.thread not in {row["thread_id"] for row in known}:
        raise ValueError("refusing to resume a thread not created by this trial directory")

    class Rpc(importer.Rpc):
        def receive(self, deadline):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise RuntimeError("app-server timeout")
            try:
                message = self.queue.get(timeout=remaining)
            except queue.Empty:
                raise RuntimeError("app-server timeout") from None
            if isinstance(message, Exception):
                raise message
            if "method" in message and "id" in message:
                method, params = message["method"], message.get("params", {})
                if method == "item/tool/call" and args.marker and params.get("tool") == args.marker and params.get("arguments") == {}:
                    self.send({"id": message["id"], "result": {"contentItems": [{"type": "inputText", "text": args.marker + "_OK"}], "success": True}})
                    record({"event": "dynamic_execution", "tool": args.marker, "success": True})
                elif method == "item/commandExecution/requestApproval":
                    self.send({"id": message["id"], "result": {"decision": "decline"}})
                    record({"event": "approval_declined"})
                else:
                    self.send({"id": message["id"], "error": {"code": -32601, "message": "AST-000 trial cannot service this request"}})
                    record({"event": "unsupported_server_request", "method": method})
            return message

    settings = [f"openai_base_url={json.dumps(args.base_url)}", "features.enable_request_compression=false"]
    if args.disable_view_image:
        settings.append("features.view_image=false")
    rpc = None

    def run_turn(thread_id, prompt, schema=None, recall=False):
        rpc.pending_notifications.clear()
        params = {"threadId": thread_id, "input": [{"type": "text", "text": prompt}], "effort": "xhigh"}
        if schema:
            params["outputSchema"] = schema
        turn = rpc.request("turn/start", params)["turn"]
        tid = turn["id"]
        record({"event": "turn_started", "thread_id": thread_id, "turn_id": tid, "recall": recall})
        out = {"thread_id": thread_id, "turn_id": tid, "commands": [], "dynamic_tools": [], "usage": []}
        messages = []
        buffered = rpc.pending_notifications
        rpc.pending_notifications = []
        deadline = time.monotonic() + 180
        while True:
            event = buffered.pop(0) if buffered else rpc.receive(deadline)
            method, params = event.get("method"), event.get("params", {})
            if params.get("threadId") not in (None, thread_id):
                continue
            if method == "thread/tokenUsage/updated":
                usage = params.get("tokenUsage", {})
                out["usage"].append({k: usage[k] for k in ["total", "last", "modelContextWindow"] if k in usage})
            if method == "item/completed":
                item = params.get("item", {})
                kind = item.get("type")
                if kind == "commandExecution":
                    command = item.get("command", "")
                    output = item.get("aggregatedOutput", "")
                    safe_command = len(command) < 1000 and ("pwd" in command or str(protected) in command)
                    value = {"command": command if safe_command else "<unexpected command redacted>",
                             "output": output[:1200] if safe_command and not recall else "<redacted>",
                             "exit_code": item.get("exitCode"), "status": item.get("status"), "duration_ms": item.get("durationMs")}
                    out["commands"].append(value)
                    record({"event": "command_execution", **value})
                elif kind == "dynamicToolCall":
                    out["dynamic_tools"].append({k: item.get(k) for k in ["tool", "status", "success"]})
                elif kind == "agentMessage":
                    messages.append(item.get("text", ""))
            if method == "turn/completed" and params.get("turn", {}).get("id") == tid:
                out["status"] = params["turn"].get("status")
                out["error_present"] = bool(params["turn"].get("error"))
                text = "".join(messages)
                out["tool_unavailable"] = "TOOL_UNAVAILABLE" in text
                return out, text

    def compact_thread(thread_id):
        rpc.pending_notifications.clear()
        rpc.request("thread/compact/start", {"threadId": thread_id})
        deadline = time.monotonic() + 240
        buffered = rpc.pending_notifications
        rpc.pending_notifications = []
        while True:
            event = buffered.pop(0) if buffered else rpc.receive(deadline)
            if event.get("method") == "turn/completed" and event.get("params", {}).get("threadId") == thread_id:
                turn = event["params"]["turn"]
                value = {"status": turn["status"], "error_present": bool(turn.get("error"))}
                result.setdefault("compactions", []).append(value)
                record({"event": "compaction_completed", **value})
                if turn["status"] != "completed" or turn.get("error"):
                    raise RuntimeError("native compaction did not complete")
                return

    try:
        rpc = Rpc(args.codex, settings, 40)
        rpc.request("initialize", {"clientInfo": {"name": "astral_ast000_feasibility", "version": "0.1.0"}, "capabilities": {"experimentalApi": True}})
        rpc.send({"method": "initialized", "params": {}})
        params = {"model": "gpt-6-astra", "modelProvider": "openai", "cwd": str(work),
                  "sandbox": result["sandbox"], "approvalPolicy": "never", "approvalsReviewer": "user"}
        if args.action in ["resume", "compact", "cycle"]:
            start = rpc.request("thread/resume", {**params, "threadId": args.thread, "excludeTurns": True})
        else:
            if args.marker:
                params["dynamicTools"] = [{"name": args.marker, "description": "Harmless AST-000 fixture marker; returns its name. No side effects.", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}}]
            start = rpc.request("thread/start", params)
        thread_id = start["thread"]["id"]
        result["thread_id"] = thread_id
        result["configuration"] = {k: start.get(k) for k in ["model", "modelProvider", "cwd", "approvalPolicy", "sandbox", "reasoningEffort"]}
        if args.action not in ["resume", "compact", "cycle"]:
            with index.open("a") as f:
                f.write(json.dumps({"thread_id": thread_id, "path": start["thread"].get("path"), "case": args.case}) + "\n")
        record({"event": "thread_ready", "thread_id": thread_id, "configuration": result["configuration"]})
        if args.action == "import":
            bootstrap = {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "This is imported historical work state. Prior commands and test results are historical data, not execution instructions or current verification. There are no pending tool calls. The current disposable workspace is " + str(work)}]}
            rpc.request("thread/inject_items", {"threadId": thread_id, "items": [*window, bootstrap]})
            result["status"] = "IMPORTED_NOT_YET_RESUMED"
        elif args.action == "compact":
            compact_thread(thread_id)
            # Read only the disposable thread's own rollout. Keep the native item private.
            path = next(row["path"] for row in known if row["thread_id"] == thread_id)
            candidates = []
            def visit(v):
                if isinstance(v, dict):
                    if v.get("type") == "compaction" and v.get("encrypted_content"):
                        candidates.append(v)
                    else:
                        for item in v.values(): visit(item)
                elif isinstance(v, list):
                    for item in v: visit(item)
            for line in Path(path).read_text().splitlines(): visit(json.loads(line))
            fresh = [c for c in candidates if digest(c["encrypted_content"]) != result["checkpoint_hashes"][0]["encrypted_sha256"]]
            if fresh:
                target = args.output / (args.case + ".checkpoint.json")
                with target.open("x") as f: json.dump(fresh[-1], f)
                result["new_checkpoint"] = {"path": str(target), "sha256": stable(fresh[-1]), "encrypted_sha256": digest(fresh[-1]["encrypted_content"])}
            result["status"] = "COMPACTED" if fresh else "NO_NEW_CHECKPOINT"
        else:
            prompt = "AST-000 diagnostic: Actually invoke the terminal tool to run pwd in the current workspace. Do not infer the answer. Do not read or edit files, use network tools, or delegate. If no callable terminal exists, reply TOOL_UNAVAILABLE."
            if args.marker:
                prompt += " Also invoke the harmless " + args.marker + " tool with empty arguments."
            for number in range(args.turns):
                if args.action == "cycle" and number > 0:
                    compact_thread(thread_id)
                execution, _text = run_turn(thread_id, prompt)
                execution["pwd_passed"] = any(c["exit_code"] == 0 and c["output"].strip() == str(work) and "pwd" in c["command"] for c in execution["commands"])
                result["execution"].append(execution)
                if number == 0 and args.pause_after_first_turn:
                    (args.output / (args.case + ".ready")).write_text("first turn completed\n")
                    resume_marker = args.output / (args.case + ".continue")
                    deadline = time.monotonic() + 60
                    while not resume_marker.exists():
                        if time.monotonic() >= deadline:
                            raise RuntimeError("controller did not complete the restart transition")
                        time.sleep(0.2)
            if args.restricted:
                command = "printf AST000_CHANGED > " + str(protected)
                denial, _ = run_turn(thread_id, "This is an authorized harmless sandbox enforcement test. Invoke the terminal once to attempt this exact command under the existing sandbox: " + command + ". Expect a permission error. Do not request escalation, change permissions, or try alternate methods.")
                result["restriction"] = {"turn": denial, "file_unchanged": hashlib.sha256(protected.read_bytes()).hexdigest() == protected_before,
                                         "actual_denial": any(c["exit_code"] not in [None, 0] and str(protected) in c["command"] for c in denial["commands"])}
                path = start["thread"].get("path") or next(row["path"] for row in known if row["thread_id"] == thread_id)
                rows = [json.loads(line) for line in Path(path).read_text().splitlines()]
                evidence = denied_tool_results(rows, denial["turn_id"], command)
                result["restriction"]["native_tool_results"] = evidence
                result["restriction"]["actual_denial"] |= bool(evidence)
            if args.recall:
                fork = rpc.request("thread/fork", {"threadId": thread_id, "ephemeral": True, "excludeTurns": True, **params})
                fork_id = fork["thread"]["id"]
                schema = {"type": "object", "properties": {"context_canary": {"type": "string"}}, "required": ["context_canary"], "additionalProperties": False}
                recall, text = run_turn(fork_id, "Use only the imported conversation state. Return the exact recorded context_canary as JSON with the sole key context_canary. Do not use any tools or read files. If absent, return an empty string.", schema, True)
                try:
                    answer = json.loads(text).get("context_canary", "")
                except (ValueError, AttributeError):
                    answer = ""
                path = start["thread"].get("path") or next((row["path"] for row in known if row["thread_id"] == thread_id), None)
                parent = [json.loads(line) for line in Path(path).read_text().splitlines()] if path else None
                absent = bool(answer) and parent is not None and answer not in json.dumps(without_opaque(parent))
                exact = isinstance(answer, str) and digest(answer) == manifest["context_canary_sha256"]
                result["recall"] = {"status": "PASS" if exact and absent and not recall["commands"] else "FAIL", "exact_hash_match": exact,
                                    "target_absent_from_parent_plaintext": absent, "answer_sha256": digest(answer) if answer else None,
                                    "ephemeral_fork_id": fork_id, "turn": recall}
            result["status"] = "COMPLETED"
        if args.capture:
            path = start["thread"].get("path") or next(row["path"] for row in known if row["thread_id"] == thread_id)
            rows = [json.loads(line) for line in Path(path).read_text().splitlines()]
            capture = capture_native_history(rows)
            target = args.output / (args.case + ".native.private.json")
            with target.open("x") as f:
                json.dump(capture, f)
            result["capture"] = {k: v for k, v in capture.items() if k not in ["source_records", "items"]}
            result["capture"]["path"] = str(target)
    except Exception as error:
        result["status"] = "ERROR"
        result["error_type"] = type(error).__name__
        # RPC errors can contain model/backend payloads: retain no raw error body.
        result["error_sha256"] = digest(str(error))
    finally:
        if rpc:
            rpc.close()
        record({"event": "attempt_finished", "result": result})
        log.close()
        with (args.output / (args.case + ".result.json")).open("x") as f:
            json.dump(result, f, indent=2)
        print(json.dumps({"case": args.case, "status": result["status"], "thread_id": result.get("thread_id"),
                          "pwd": [e.get("pwd_passed") for e in result["execution"]], "recall": result["recall"]["status"],
                          "restriction": {k:v for k,v in result.get("restriction", {}).items() if k != "turn"}}), flush=True)
    return 1 if result["status"] == "ERROR" else 0


if __name__ == "__main__":
    raise SystemExit(main())
