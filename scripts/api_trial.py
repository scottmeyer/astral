#!/usr/bin/env python3
"""Matched live Responses trials through Astral, independent of Codex startup.

Uses a configured gateway (--auth gateway) or OPENAI_API_KEY (--auth api-key).
Gateway mode sends no credential; the configured endpoint must provide its own
authorized routing. Never reads Codex authentication files. Python 3.11+.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import random
import sys
import time
import urllib.error
import urllib.request
import uuid

import codex_trial as common


TOOL = {
    "type": "function", "name": "read_fixture", "description": "Read one synthetic test fixture.",
    "parameters": {"type": "object", "properties": {"name": {"type": "string", "enum": ["config", "audit"]}},
                   "required": ["name"], "additionalProperties": False}, "strict": True,
}
PROMPTS = [
    "Use read_fixture to load config. Retain its deployment fields. Reply LOADED without repeating the rows.",
    "Correction: budget is now 23, replacing 17. Keep all other fields. Use no tools. Reply UPDATED.",
    "Use no tools. Return only JSON with deployment_label, region, budget (integer), and recovery_code, applying the correction.",
    "Use read_fixture to load audit. Retain the audit code and all corrected deployment fields. Reply AUDITED without repeating the rows.",
    "Use no tools. Return only JSON with deployment_label, region, budget (integer), recovery_code, and audit_code.",
    "The transport has restarted. Use no tools. Again return only JSON with deployment_label, region, budget (integer), recovery_code, and audit_code.",
]


def fixture_data():
    filler = "".join(f"{i:04d} synthetic build check passed; no deployment configuration changes.\n" for i in range(1500))
    return {
        "config": "Deployment label: helios-731. Region: syd. Budget: 17. Recovery code: copper-otter.\n" + filler,
        "audit": "Audit code: violet-409. Keep all previously corrected deployment fields.\n" + filler,
    }


def completed_response(raw):
    final = None
    items = {}
    for frame in raw.replace(b"\r\n", b"\n").split(b"\n\n"):
        data = b"\n".join(line[5:].lstrip() for line in frame.splitlines() if line.startswith(b"data:"))
        if not data or data == b"[DONE]":
            continue
        event = json.loads(data)
        if event.get("type") == "response.output_item.done":
            index, item = event.get("output_index"), event.get("item")
            if type(index) is not int or index < 0 or not isinstance(item, dict):
                raise RuntimeError("invalid_completed_output_item")
            if index in items and items[index] != item:
                raise RuntimeError("conflicting_completed_output_item")
            items[index] = item
        elif event.get("type") == "response.completed":
            final = event.get("response")
        elif event.get("type") in ("error", "response.failed", "response.incomplete"):
            raise RuntimeError("upstream_did_not_complete")
    if not final or final.get("status") != "completed" or not isinstance(final.get("output"), list):
        raise RuntimeError("missing_completed_response")
    # Some Codex gateways emit canonical items in output_item.done and send an
    # empty output array in the terminal summary. Never discard those items.
    if not final["output"] and items:
        if sorted(items) != list(range(len(items))):
            raise RuntimeError("incomplete_output_item_sequence")
        final["output"] = [items[index] for index in range(len(items))]
    return final


def call(args, proxy, body, lane, destination):
    headers = {"Content-Type": "application/json", "Accept": "text/event-stream", "x-astral-session-id": lane}
    if args.auth == "api-key":
        headers["Authorization"] = "Bearer " + os.environ["OPENAI_API_KEY"]
    client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    request = urllib.request.Request(proxy.base_url + "/responses", data=json.dumps(body).encode(), headers=headers, method="POST")
    try:
        response = client.open(request, timeout=args.timeout * 2 + 10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read(8 * 1024 * 1024 + 1)
        if len(raw) > 8 * 1024 * 1024:
            raise RuntimeError("response_observation_limit")
        destination.write_bytes(raw)
        if response.status != 200:
            raise RuntimeError(f"upstream_http_{response.status}")
    return completed_response(raw)


def run_arm(args, arm, folder, data):
    result = {"arm": arm, "status": "running", "passed": False, "turns": []}
    proxy = common.Proxy(args, arm, folder)
    lane = "astral-api-" + str(uuid.uuid4())
    history = []
    body = {"model": args.model, "instructions": f"Opaque trial label: {lane}. Do not mention the label. Follow the user's requests. Use read_fixture only when requested. All fixture data is synthetic. Preserve corrections across turns. Keep replies brief.",
            "tools": [TOOL], "stream": True, "store": False, "reasoning": {"effort": "low"},
            "include": ["reasoning.encrypted_content"], "prompt_cache_key": lane}
    started = time.monotonic()
    try:
        proxy.start()
        before_restart = None
        for number, prompt in enumerate(PROMPTS, 1):
            if number == 6:
                before_restart = common.snapshot(proxy.state)
                proxy.close()
                if proxy.exit_codes[-1] != 0:
                    raise RuntimeError("proxy_shutdown_failed")
                proxy.start()
            turn_started = time.monotonic()
            history.append({"role": "user", "content": [{"type": "input_text", "text": prompt}]})
            tool_calls = 0
            text = ""
            for attempt in range(3):
                body["input"] = history
                response = call(args, proxy, body, lane, folder / f"turn-{number}-{attempt + 1}.sse")
                output = response["output"]
                history.extend(output)  # Replay every native output item, including encrypted reasoning/phase.
                calls = [item for item in output if item.get("type") == "function_call"]
                if not calls:
                    text = "".join(content.get("text", "") for item in output if item.get("type") == "message"
                                   for content in item.get("content", []) if content.get("type") == "output_text")
                    break
                tool_calls += len(calls)
                if number not in (1, 4):
                    raise RuntimeError("tool_use_during_recall")
                for item in calls:
                    arguments = json.loads(item["arguments"])
                    expected = "config" if number == 1 else "audit"
                    if item["name"] != "read_fixture" or arguments != {"name": expected}:
                        raise RuntimeError("unexpected_fixture_request")
                    history.append({"type": "function_call_output", "call_id": item["call_id"], "output": data[expected]})
            else:
                raise RuntimeError("tool_loop_limit")
            turn = {"turn": number, "seconds": round(time.monotonic() - turn_started, 3),
                    "model_calls": attempt + 1, "tool_calls": tool_calls, "text": text}
            if number in (3, 5, 6):
                turn["answer_correct"] = common.correct_answer(text, number != 3)
            result["turns"].append(turn)
            common.write_json(folder / "result.json", result)
            print(json.dumps({"arm": arm, **turn}), flush=True)
        usage = common.summarize_ledger(proxy.state / "ledger.jsonl")
        result["checks"] = {
            "recall": all(t["answer_correct"] for t in result["turns"] if "answer_correct" in t),
            "fixtures_loaded": all(result["turns"][i]["tool_calls"] > 0 for i in (0, 3)),
            "all_responses_complete": usage["response"]["completed"] == sum(t["model_calls"] for t in result["turns"]),
            "recursive_rollover": usage["compact"]["accepted"] >= 2 if arm == "rolling" else usage["compact"]["requests"] == 0,
            "projection_reused_after_restart": len(before_restart or []) == 1 and before_restart[0]["cut"] > 0 and before_restart == common.snapshot(proxy.state)
                                                if arm == "rolling" else True,
            "all_generations_committed": usage["response"]["committed"] == usage["response"]["requests"] if arm == "rolling" else True,
        }
        result["passed"] = all(result["checks"].values())
        result["status"] = "passed" if result["passed"] else "failed_gate"
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        result.update(status="blocked", passed=False, error=str(error))
    finally:
        proxy.close()
        result["seconds"] = round(time.monotonic() - started, 3)
        result["proxy_exit_codes"] = proxy.exit_codes
        result["proxy_requests_received_by_process"] = proxy.request_counts
        try:
            result["usage"] = common.summarize_ledger(proxy.state / "ledger.jsonl")
        except (OSError, ValueError, KeyError):
            result.update(status="blocked", passed=False, error="unreadable_proxy_ledger")
        if any(code != 0 for code in proxy.exit_codes):
            result.update(status="blocked", passed=False, error="proxy_shutdown_failed")
        common.write_json(folder / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True)
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--auth", choices=("gateway", "api-key"), required=True)
    parser.add_argument("--allow-compatible-compaction", action="store_true")
    parser.add_argument("--compact-path", default="/responses/compact")
    parser.add_argument("--binary", type=Path, default=common.ROOT / "target/release/astral")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=90)
    parser.add_argument("--roll-bytes", type=int, default=64_000)
    parser.add_argument("--seed", type=int, default=731)
    args = parser.parse_args()
    if args.timeout < 1 or args.roll_bytes < 1:
        parser.error("timeout and roll-bytes must be positive")
    if args.auth == "api-key" and not os.environ.get("OPENAI_API_KEY"):
        parser.error("OPENAI_API_KEY is required for API-key mode")
    from urllib.parse import urlsplit
    url = urlsplit(args.upstream)
    platform = url.scheme == "https" and url.hostname == "api.openai.com" and url.port in (None, 443)
    if not platform and not args.allow_compatible_compaction:
        parser.error("compatible trials require --allow-compatible-compaction")
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.output.mkdir(parents=True, mode=0o700, exist_ok=False)
    data = fixture_data()
    order = ["passthrough", "rolling"]
    random.Random(args.seed).shuffle(order)
    report = {"model": args.model, "auth": args.auth, "order": order, "seed": args.seed,
              "roll_bytes": args.roll_bytes, "compact_path": args.compact_path,
              "fixture_bytes": {k: len(v.encode()) for k, v in data.items()},
              "cache_isolation": "Separate stable keys and a distinct instruction prefix per arm; gateways may override routing keys.",
              "passed": False, "arms": [],
              "interpretation": "One synthetic matched trial, not a production quality or cost benchmark."}
    for arm in order:
        folder = args.output / arm
        folder.mkdir(mode=0o700)
        result = run_arm(args, arm, folder, data)
        report["arms"].append(result)
        common.write_json(args.output / "report.json", report)
        if result["status"] == "blocked":
            break
    report["passed"] = len(report["arms"]) == 2 and all(a["passed"] for a in report["arms"])
    common.write_json(args.output / "report.json", report)
    print(json.dumps({"report": str(args.output / "report.json"), "passed": report["passed"]}), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
