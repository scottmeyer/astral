#!/usr/bin/env python3
"""Matched coding trial: native, native + adapter, Astral + same adapter.

All arms get identical tools, prompts and executable tests. Private session
files contain complete native items. report.json contains only measurements,
synthetic answers, hashes and acceptance results; no encrypted payloads.
"""
from __future__ import annotations
import argparse
from copy import copy
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import time
from urllib.parse import urlsplit

import codex_trial as common
sys.path.insert(0, str(common.ROOT / "python"))
from astral import Host, Session


PROMPTS = [
    "Fix total.py. total(items, discount_bps=0) takes a list of (quantity, unit_price_cents) nonnegative integer pairs. Discount is an integer number of basis points, 0..10000 inclusive; reject out-of-range values with ValueError. Return an integer number of cents, rounding half upward after discount, using exact integer arithmetic. Preserve that public signature. Inspect the implementation and run the configured tests before editing, then fix it and run tests again. Keep all changes in total.py. Finish only with a concise status; do not echo audit tokens from reports.",
    "The host has advanced requirements.json to phase 2. Add subtotal(items), returning the undiscounted integer sum of quantity times unit price. Preserve total's existing signature, discount behavior and bounds. Inspect working_state first: determine whether the earlier passing check is still current after the requirements change. Implement the addition and rerun the configured tests. Keep all changes in total.py. Finish with a concise status and do not echo any audit tokens.",
    "Retrieve audit_token from the FIRST SUCCESSFUL tests execution in phase 1, before the requirements changed to phase 2. Call recall to verify the token against the saved original bytes even if you remember it; check_history and search can locate the handle and offset. A rerun cannot recover that execution's token. Do not run any new checks or change files on this turn. Return only JSON with audit_token (that exact string), sample_total (total([(2,123),(3,456)],2500)), and sample_subtotal (subtotal of those same items)).",
]

# Acceptance is outside the model-writable workspace and uses unseen inputs.
ACCEPT = r'''
import importlib.util,json,random,sys
spec=importlib.util.spec_from_file_location("candidate",sys.argv[1])
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
rng=random.Random(int(sys.argv[2])+1000003)
for i in range(400):
    items=[(rng.randrange(1000),rng.randrange(10**14)) for _ in range(rng.randrange(10))]
    bps=rng.randrange(10001); raw=sum(q*p for q,p in items)
    value=m.total(items,bps)
    assert type(value) is int and value==(raw*(10000-bps)+5000)//10000,(i,value)
    assert type(m.subtotal(items)) is int and m.subtotal(items)==raw
for bps in (-1,10001):
    try:m.total([(1,100)],bps)
    except ValueError:pass
    else:raise AssertionError("bounds")
assert m.total([(1,1)],5000)==1
print(json.dumps({"passed":True,"unseen_cases":400,"large_integers":True,"bounds":True,"half_up":True}))
'''


def raw_json(host, handle):
    data = (host.state_dir / "blobs" / handle).read_bytes()
    if hashlib.sha256(data).hexdigest() != handle:
        raise RuntimeError("raw artifact hash mismatch")
    return json.loads(data)


def receipts(host):
    return host.call("check_history", name="tests", before_sequence=0)["result"]["receipts"]


def run_arm(args, arm, seed, folder):
    workspace = folder / "workspace"
    shutil.copytree(common.ROOT / "examples/working-task", workspace, ignore=shutil.ignore_patterns("__pycache__"))
    (workspace / "requirements.json").write_text(json.dumps({"phase": 1, "seed": seed}))
    profile = json.loads((workspace / "profile.json").read_text())
    profile["checks"]["tests"]["argv"][0] = sys.executable
    profile_path = folder / "profile.json"
    common.write_json(profile_path, profile)
    protected_hash = hashlib.sha256((workspace / "check_total.py").read_bytes()).hexdigest()
    opts = copy(args)
    opts.compaction_backend, opts.inline_tool_boundaries, opts.allow_compatible_compaction = "inline", True, True
    proxy = common.Proxy(opts, "rolling" if arm == "astral" else "passthrough", folder)
    host = Host(workspace, folder / "working", profile_path, raw=arm == "native")
    report = {"arm": arm, "seed": seed, "passed": False, "turns": [], "checks": {}}
    session = None
    started = time.monotonic()
    try:
        proxy.start()
        session = Session(host, folder / "session.json", proxy.base_url, args.model, arm, args.auth,
                          args.threshold, args.timeout, args.cache_mode, provider_scope=args.upstream)
        for phase in (1, 2):
            if phase == 2:
                (workspace / "requirements.json").write_text(json.dumps({"phase": 2, "seed": seed}))
                report["checks"]["changed_requirements_invalidate_check"] = not host.snapshot()["checks"]["tests"]["current"]
            text = session.turn(PROMPTS[phase - 1], args.max_calls)
            state = host.snapshot()
            check = state["checks"].get("tests", {})
            passed = check.get("passed") is True and check.get("current") is True
            report["checks"][f"phase_{phase}_current_pass"] = passed
            report["turns"].append({"phase": phase, "text": text, "passed": passed})
            if not passed:
                raise RuntimeError(f"phase {phase} lacks current passing verification")
            if phase == 1:
                first = min((r for r in receipts(host) if r["passed"]), key=lambda r: r["sequence"])
                expected_token = raw_json(host, first["stdout_handle"])["audit_token"]
                report["original_successful_stdout_sha256"] = first["stdout_handle"]
                report["checks"]["initial_failing_check_observed"] = any(not r["passed"] for r in receipts(host))
            common.write_json(folder / "result.json", report)
            print(json.dumps({"arm": arm, "seed": seed, "phase": phase, "passed": passed, "model_calls": len(session.data["calls"])}), flush=True)

        accepted = subprocess.run([sys.executable, "-B", "-c", ACCEPT, str(workspace / "total.py"), str(seed)],
                                  capture_output=True, text=True, timeout=20)
        report["checks"]["independent_code_acceptance"] = accepted.returncode == 0
        report["acceptance"] = json.loads(accepted.stdout) if accepted.returncode == 0 else {"error": accepted.stderr[-2000:]}
        report["checks"]["protected_test_file_unchanged"] = hashlib.sha256((workspace / "check_total.py").read_bytes()).hexdigest() == protected_hash
        before_host = host.snapshot()
        before_proxy = common.snapshot(proxy.state)
        before_snapshots = session.data["snapshots"].copy()
        prior_checks = len(receipts(host))
        prior_tools = len(session.data["tools"])
        host.close()
        proxy.close()
        host = Host(workspace, folder / "working", profile_path, raw=arm == "native")
        proxy.start()
        session = Session(host, folder / "session.json", proxy.base_url, args.model, arm, args.auth,
                          args.threshold, args.timeout, args.cache_mode, provider_scope=args.upstream)
        report["checks"]["working_state_survived_restart"] = before_host == host.snapshot()
        report["checks"]["projection_survived_restart"] = before_proxy == common.snapshot(proxy.state)
        report["checks"]["frozen_snapshots_survived_restart"] = before_snapshots == session.data["snapshots"]
        text = session.turn(PROMPTS[2], args.max_calls)
        answer = json.loads(text)
        report["checks"]["late_exact_retrieval_correct"] = answer == {"audit_token": expected_token, "sample_total": 1211, "sample_subtotal": 1614}
        report["checks"]["no_check_rerun_for_old_output"] = len(receipts(host)) == prior_checks
        report["checks"]["exact_artifact_tool_used"] = any(t["name"] == "recall" and t["ok"] for t in session.data["tools"][prior_tools:])
        report["turns"].append({"phase": 3, "text": text, "passed": report["checks"]["late_exact_retrieval_correct"]})
        report["checks"]["final_verification_current"] = host.snapshot()["checks"]["tests"]["current"] is True
        response_rows = [r for r in common.rows(proxy.state / "ledger.jsonl") if r["kind"] == "response"]
        report["checks"]["all_generations_completed"] = len(response_rows) == len(session.data["calls"]) and all(r["outcome"] == "completed" for r in response_rows)
        report["checks"]["all_usage_reported"] = all(isinstance(r.get("usage"), dict) for r in response_rows)
        if arm == "astral":
            report["checks"]["all_generations_committed"] = all(r.get("committed") for r in response_rows)
            report["checks"]["no_history_resets"] = all(r["reason"] not in ("history_reset", "contract_reset") for r in response_rows)
        report["projection_epochs"] = [{k: r.get(k) for k in ("epoch", "cut", "projection_sha256", "inline_scheduled", "inline_checkpoints_adopted", "bytes_in", "bytes_out", "reason")} for r in response_rows]
        report["astral_adopted_checkpoints"] = sum(r.get("inline_checkpoints_adopted", 0) for r in response_rows)
        report["passed"] = all(report["checks"].values())
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.SubprocessError) as error:
        report["error"] = str(error)
    finally:
        host.close()
        proxy.close()
        report["seconds"] = round(time.monotonic() - started, 3)
        report["proxy_ingress"] = proxy.request_counts
        report["proxy_exit_codes"] = proxy.exit_codes
        report["usage"] = common.summarize_ledger(proxy.state / "ledger.jsonl")
        if session:
            report["calls"], report["tools"], report["snapshots"] = session.data["calls"], session.data["tools"], session.data["snapshots"]
            report["native_compactions"] = sum(c["native_compactions"] for c in report["calls"])
            report["tool_observation_bytes"] = sum(t["bytes"] for t in report["tools"])
        if any(code != 0 for code in proxy.exit_codes):
            report.update(passed=False, error="proxy shutdown failed")
        common.write_json(folder / "result.json", report)
    return report


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--model", required=True)
    p.add_argument("--upstream", required=True)
    p.add_argument("--auth", choices=("gateway", "api-key"), required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--binary", type=Path, default=common.ROOT / "target/release/ostk-gpt-cache")
    p.add_argument("--seeds", default="731,947")
    p.add_argument("--arms", default="native,native-adapted,astral")
    p.add_argument("--threshold", type=int, default=8192)
    p.add_argument("--roll-bytes", type=int, default=64000)
    p.add_argument("--timeout", type=int, default=90)
    p.add_argument("--max-calls", type=int, default=20)
    p.add_argument("--cache-mode", choices=("provider-default", "implicit"), default="provider-default")
    args = p.parse_args()
    args.seeds, args.arms = [int(v) for v in args.seeds.split(",")], args.arms.split(",")
    if not args.arms or any(a not in ("native", "native-adapted", "astral") for a in args.arms):
        p.error("invalid arms")
    if min(args.threshold, args.roll_bytes, args.timeout, args.max_calls) < 1:
        p.error("limits must be positive")
    modern = any(args.model == f or args.model.startswith(f + "-") for f in ("gpt-5.6", "gpt-6-astra"))
    if urlsplit(args.upstream).hostname == "api.openai.com" and modern and args.cache_mode != "implicit":
        p.error("use --cache-mode implicit for equal modern Platform cache controls")
    if args.auth == "api-key" and not os.environ.get("OPENAI_API_KEY"):
        p.error("OPENAI_API_KEY is required")
    args.binary, args.output = args.binary.resolve(), args.output.resolve()
    args.output.mkdir(parents=True, mode=0o700, exist_ok=False)
    report = {"date": datetime.now(timezone.utc).date().isoformat(), "model": args.model, "upstream": args.upstream,
              "auth": args.auth, "cache_mode": args.cache_mode, "threshold": args.threshold, "roll_bytes": args.roll_bytes,
              "seeds": args.seeds, "ordering": [], "trials": [], "passed": False,
              "limits": "Small executable coding task, 600 detailed check observations per run, 400 unseen acceptance cases per arm. Identical prompts/tools. Adapter effects must be separated from projection effects. Reported tokens and wall time are not verified charges; provider-default cache behavior is uncontrolled on gateway. No automatic retries or required minimum rollover count."}
    for seed in args.seeds:
        order = args.arms.copy()
        random.Random(seed).shuffle(order)
        report["ordering"].append({"seed": seed, "arms": order})
        for arm in order:
            folder = args.output / f"{seed}-{arm}"
            folder.mkdir(mode=0o700)
            result = run_arm(args, arm, seed, folder)
            report["trials"].append(result)
            common.write_json(args.output / "report.json", report)
            print(json.dumps({"arm": arm, "seed": seed, "passed": result["passed"], "error": result.get("error"), "input_tokens": result["usage"]["response"]["input_tokens"]}), flush=True)
            if "error" in result:
                return 1
    report["passed"] = all(t["passed"] for t in report["trials"])
    common.write_json(args.output / "report.json", report)
    print(json.dumps({"report": str(args.output / "report.json"), "passed": report["passed"]}), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
