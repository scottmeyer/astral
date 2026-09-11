#!/usr/bin/env python3
"""Validate provider-owned inline compaction and opaque replay through Astral.

This tests context_management on /responses, not Astral's standalone compactor.
Raw streams and the encrypted client window stay in the private trial directory.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import random
import sys
import time
import uuid

import api_trial as api
import codex_trial as common


def opaque_items(items):
    return [{"type": item["type"], "sha256": hashlib.sha256(item["encrypted_content"].encode()).hexdigest(),
             "bytes": len(item["encrypted_content"].encode())}
            for item in items if isinstance(item.get("encrypted_content"), str) and item["encrypted_content"]]


def prune_inline_history(history):
    """Only for inline output appended to history; never trim standalone compact output."""
    positions = [index for index, item in enumerate(history) if item.get("type") == "compaction"]
    if not positions:
        return history
    start = positions[-1]
    payload = history[start].get("encrypted_content")
    if not isinstance(payload, str) or not payload:
        raise RuntimeError("inline_compaction_missing_encrypted_content")
    return history[start:]


def run(args):
    rng = random.Random(args.seed)
    expected = {"deployment_label": f"deploy-{rng.getrandbits(96):024x}",
                "region": f"zone-{rng.getrandbits(64):016x}", "budget": 23,
                "recovery_code": f"recover-{rng.getrandbits(96):024x}",
                "audit_code": f"audit-{rng.getrandbits(96):024x}"}
    initial = {k: v for k, v in expected.items() if k != "audit_code"}
    initial["budget"] = 17
    filler = "".join(f"{i:04d} synthetic build check passed; no configuration changes.\n" for i in range(1800))
    fixtures = {"config": json.dumps(initial) + "\n" + filler,
                "audit": json.dumps({"audit_code": expected["audit_code"]}) + "\n" + filler}
    prompts = [
        "Read the config fixture. Remember every configuration field, but reply only READY. Do not repeat field values.",
        "Correction: budget is now 23, replacing 17. Preserve all other configuration fields. Reply only READY. Use no tools.",
        "Read the audit fixture. Preserve its audit_code, all earlier configuration fields, and the corrected budget. Reply only READY without repeating any field values.",
        "Return only JSON with deployment_label, region, budget (integer), recovery_code, and audit_code from the conversation. Use no tools.",
    ]
    lane = "astral-inline-" + str(uuid.uuid4())
    body = {"model": args.model, "instructions": f"Trial label: {lane}. Do not repeat the label. Preserve fixture facts and later corrections. Use read_fixture only when requested. Before the final JSON request, reply only READY and never repeat field values.",
            "tools": [api.TOOL], "store": False, "stream": True,
            "reasoning": {"effort": "medium", "context": "all_turns"},
            "include": ["reasoning.encrypted_content"], "prompt_cache_key": lane,
            "context_management": [{"type": "compaction", "compact_threshold": args.threshold}]}
    report = {"date": datetime.now(timezone.utc).date().isoformat(), "model": args.model, "auth": args.auth,
              "upstream": args.upstream, "seed": args.seed, "threshold": args.threshold,
              "owner": "provider inline context_management", "astral_autonomous_rollover_verified": False,
              "expected": expected, "fixture_bytes": {k: len(v.encode()) for k, v in fixtures.items()},
              "calls": [], "turns": [], "passed": False,
              "interpretation": "One synthetic canary test. Does not establish general semantic fidelity, cache retention, or cost savings."}
    history = []
    proxy = common.Proxy(args, "rolling", args.output)
    compactions = []
    loaded = []
    started = time.monotonic()
    try:
        proxy.start()
        before_restart = None
        for number, prompt in enumerate(prompts, 1):
            if number == 4:
                before_restart = opaque_items(history)
                common.write_json(args.output / "client-window.json", history)
                proxy.close()
                proxy.start()
                history = json.loads((args.output / "client-window.json").read_text())
                report["opaque_identical_after_reload"] = before_restart == opaque_items(history)
            history.append({"role": "user", "content": [{"type": "input_text", "text": prompt}]})
            if number == 4:
                wire = json.dumps({**body, "input": history})
                report["canaries_absent_from_final_plaintext_input"] = all(
                    value not in wire for value in expected.values() if isinstance(value, str))
            for attempt in range(3):
                body["input"] = history
                input_opaque = opaque_items(history)
                response = api.call(args, proxy, body, lane, args.output / f"turn-{number}-{attempt + 1}.sse")
                output = response["output"]
                new_compactions = [item for item in opaque_items(output) if item["type"] == "compaction"]
                if new_compactions:
                    compactions.append({"items": new_compactions, "input_opaque": input_opaque})
                report["calls"].append({"turn": number, "input_bytes": len(json.dumps(history).encode()),
                                        "input_opaque": input_opaque, "output_opaque": opaque_items(output),
                                        "output_types": [item.get("type") for item in output],
                                        "reasoning": response.get("reasoning")})
                history.extend(output)
                history = prune_inline_history(history)
                calls = [item for item in output if item.get("type") == "function_call"]
                if not calls:
                    text = "".join(c.get("text", "") for item in output if item.get("type") == "message"
                                   for c in item.get("content", []) if c.get("type") == "output_text")
                    if number != 4 and text.strip() != "READY":
                        raise RuntimeError("fixture_values_must_not_be_echoed_before_final_recall")
                    turn = {"turn": number, "text": text}
                    if number == 4:
                        turn["correct"] = json.loads(text) == expected
                    report["turns"].append(turn)
                    print(json.dumps({"turn": number, "compaction_events": len(compactions),
                                      "retained_input_bytes": len(json.dumps(history).encode()), **turn}), flush=True)
                    break
                for item in calls:
                    name = "config" if number == 1 else "audit" if number == 3 else None
                    if name is None or item["name"] != "read_fixture" or json.loads(item["arguments"]) != {"name": name}:
                        raise RuntimeError("unexpected_tool_call")
                    loaded.append(name)
                    history.append({"type": "function_call_output", "call_id": item["call_id"], "output": fixtures[name]})
            else:
                raise RuntimeError("tool_loop_limit")
            common.write_json(args.output / "report.json", report)
        report["checks"] = {
            "both_fixtures_loaded": set(loaded) == {"config", "audit"},
            "at_least_two_compactions": len(compactions) >= 2,
            "previous_compaction_replayed_into_next": len(compactions) >= 2 and all(
                item in compactions[1]["input_opaque"] for item in compactions[0]["items"]),
            "final_recall": report["turns"][-1].get("correct", False),
            "canaries_absent_from_final_plaintext_input": report.get("canaries_absent_from_final_plaintext_input", False),
            "opaque_identical_after_reload": bool(before_restart) and report.get("opaque_identical_after_reload", False),
        }
        report["passed"] = all(report["checks"].values())
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        report["error"] = str(error)
    finally:
        proxy.close()
        report["seconds"] = round(time.monotonic() - started, 3)
        report["proxy_ingress"] = proxy.request_counts
        report["proxy_exit_codes"] = proxy.exit_codes
        report["usage"] = common.summarize_ledger(proxy.state / "ledger.jsonl")
        if any(code != 0 for code in proxy.exit_codes):
            report.update(passed=False, error="proxy_shutdown_failed")
        common.write_json(args.output / "report.json", report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True)
    parser.add_argument("--upstream", required=True)
    parser.add_argument("--auth", choices=("gateway", "api-key"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=common.ROOT / "target/release/astral")
    parser.add_argument("--threshold", type=int, default=8192)
    parser.add_argument("--timeout", type=int, default=90)
    parser.add_argument("--seed", type=int, default=731)
    parser.set_defaults(compact_path="/responses/compact", roll_bytes=1_000_000, allow_compatible_compaction=False)
    args = parser.parse_args()
    if args.threshold < 1 or args.timeout < 1:
        parser.error("threshold and timeout must be positive")
    if args.auth == "api-key" and not os.environ.get("OPENAI_API_KEY"):
        parser.error("OPENAI_API_KEY is required for API-key mode")
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.output.mkdir(parents=True, mode=0o700, exist_ok=False)
    report = run(args)
    print(json.dumps({"report": str(args.output / "report.json"), "passed": report["passed"], "checks": report.get("checks")}), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
