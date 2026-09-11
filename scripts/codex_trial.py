#!/usr/bin/env python3
"""Run matched, isolated Codex sessions through passthrough and rolling proxies.

Uses Codex's normal authentication. Never reads or copies authentication files.
Python 3.11+; no third-party dependencies. This invokes live inference unless
--prepare-only is supplied. A timeout or absent rollover is a failed gate.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def stop(process):
    if process.poll() is None:
        # Kill only this invocation's process group, including its own helpers.
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGTERM)
        else:
            process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            if os.name == "posix":
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
            process.wait(timeout=5)
    return process.returncode


def spawn(command, **kwargs):
    return subprocess.Popen(command, stdin=subprocess.DEVNULL,
                            start_new_session=(os.name == "posix"), **kwargs)


def rows(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()]


def summarize_ledger(path):
    records = rows(path)
    groups = {}
    for kind in ("response", "compact"):
        events = [e for e in records if e.get("kind") == kind]
        usages = [e["usage"] for e in events if isinstance(e.get("usage"), dict)]
        groups[kind] = {
            "requests": len(events),
            "completed": sum(e.get("outcome") == "completed" for e in events),
            "accepted": sum(e.get("outcome") == "accepted" for e in events),
            "committed": sum(e.get("committed", False) for e in events),
            "usage_records": len(usages),
            "input_tokens": sum(u["input_tokens"] for u in usages),
            "cached_tokens": sum(u["cached_tokens"] for u in usages),
            "output_tokens": sum(u["output_tokens"] for u in usages),
            "known_cache_write_tokens": sum(u.get("cache_write_tokens") or 0 for u in usages),
            "missing_cache_write_records": sum(u.get("cache_write_tokens") is None for u in usages),
        }
    return groups


def snapshot(state):
    result = []
    for path in sorted((state / "lanes").glob("*.json")):
        value = json.loads(path.read_text(encoding="utf-8"))
        projection = json.dumps(value["projection"], sort_keys=True).encode()
        result.append({"lane": path.stem, "epoch": value["epoch"], "cut": value["cut"],
                       "projection_sha256": hashlib.sha256(projection).hexdigest()})
    return result


class Proxy:
    def __init__(self, args, arm, folder):
        self.args, self.arm, self.folder = args, arm, folder
        self.state = folder / "state"
        self.process = None
        self.port = 0
        self.generation = 0
        self.exit_codes = []
        self.request_counts = []

    def start(self):
        self.generation += 1
        log = self.folder / f"proxy-{self.generation}.stderr"
        command = [str(self.args.binary), "--listen", f"127.0.0.1:{self.port}",
                   "--upstream", self.args.upstream, "--mode", self.arm,
                   "--compact-path", getattr(self.args, "compact_path", "/responses/compact"),
                   "--compaction-backend", getattr(self.args, "compaction_backend", "standalone"),
                   "--inline-threshold-tokens", str(getattr(self.args, "threshold", 8192)),
                   "--state-dir", str(self.state), "--roll-bytes", str(self.args.roll_bytes),
                   "--keep-recent-turns", "1", "--min-compact-bytes", "4096",
                   "--min-roll-seconds", str(getattr(self.args, "min_roll_seconds", 0)),
                   "--request-timeout-seconds", str(self.args.timeout),
                   "--compact-timeout-seconds", str(self.args.timeout)]
        if self.args.allow_compatible_compaction:
            command.append("--allow-compatible-compaction")
        if getattr(self.args, "inline_tool_boundaries", False):
            command.append("--inline-tool-boundaries")
        if getattr(self.args, "economic_roll_policy", False):
            command.append("--economic-roll-policy")
        process_env = os.environ.copy()
        if getattr(self.args, "auth", None) == "gateway":
            # Gateway routing is already authorized; do not forward a separate
            # Platform key through the proxy's environment credential fallback.
            process_env.pop("OPENAI_API_KEY", None)
        with log.open("wb") as stderr:
            self.process = spawn(command, stdout=subprocess.DEVNULL, stderr=stderr, env=process_env)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError("proxy_startup_failed")
            match = re.search(r"listening on 127\.0\.0\.1:(\d+)", log.read_text())
            if match:
                self.port = int(match.group(1))
                try:
                    # Loopback health checks must not use an environment HTTP proxy.
                    client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                    with client.open(self.base_url + "/healthz", timeout=1) as response:
                        if json.load(response)["status"] == "ok":
                            return
                except (OSError, ValueError, KeyError):
                    pass
            time.sleep(0.05)
        raise RuntimeError("proxy_health_timeout")

    @property
    def base_url(self):
        return f"http://127.0.0.1:{self.port}"

    def close(self):
        if self.process:
            try:
                client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                with client.open(self.base_url + "/healthz", timeout=1) as response:
                    self.request_counts.append(json.load(response).get("requests_received"))
            except (OSError, ValueError):
                self.request_counts.append(None)
            self.exit_codes.append(stop(self.process))
            self.process = None


def fixtures(folder):
    # Facts appear only in tool results, so successful later recall must survive
    # compaction of old tool traffic, rather than re-reading a retained user prompt.
    filler = "".join(f"{i:04d} completed synthetic build check; no configuration changes.\n"
                     for i in range(480))
    (folder / "fixture.txt").write_text(
        'Deployment label: helios-731. Region: syd. Budget: 17.\n'
        'Recovery code: copper-otter. These four fields are authoritative.\n' + filler,
        encoding="utf-8")
    (folder / "checks.txt").write_text(
        "Audit code: violet-409. Keep all previously corrected deployment fields.\n" + filler,
        encoding="utf-8")
    prompts = [
        "This is an Astral context-retention trial. Read fixture.txt completely with a shell "
        "tool, allowing at least 14000 output tokens so it is not truncated. Retain its four "
        "authoritative fields for later. Do not change files or use network tools. Reply LOADED.",
        "Correction: the deployment budget is now 23, replacing 17. Keep all other fields. "
        "Use no tools. Reply UPDATED.",
        "Use no tools. From memory return only JSON with deployment_label, region, budget "
        "(an integer), and recovery_code. Apply the correction.",
        "Read checks.txt completely with a shell tool, allowing at least 14000 output tokens. "
        "Retain its audit code and all previously corrected fields. Reply AUDITED. "
        "Do not change files or use network tools.",
        "Use no tools. Return only JSON with deployment_label, region, budget (an integer), "
        "recovery_code, and audit_code. Preserve the corrected budget.",
        "The transport has restarted. Use no tools. Again return only JSON with "
        "deployment_label, region, budget (an integer), recovery_code, and audit_code.",
    ]
    write_json(folder / "prompts.json", prompts)
    return prompts


def command(args, proxy, lane, session, prompt):
    # A dedicated provider avoids mutating any built-in or user provider.
    provider = {
        "name": "Astral trial", "base_url": proxy.base_url, "wire_api": "responses",
        "supports_websockets": False, "request_max_retries": 0, "stream_max_retries": 0,
    }
    if args.auth == "chatgpt":
        provider["requires_openai_auth"] = True
    else:
        provider["env_key"] = "OPENAI_API_KEY"
    inline = ",".join(f"{k}={json.dumps(v)}" for k, v in provider.items())
    inline += ',http_headers={"x-ostk-session-id"=' + json.dumps(lane) + "}"
    settings = {
        "model_provider": '"trial"', "model_providers.trial": "{" + inline + "}",
        "sandbox_mode": json.dumps(args.sandbox), "model_reasoning_effort": '"low"',
        "features.enable_request_compression": "false", "features.apps": "false",
        "features.plugins": "false", "features.remote_plugin": "false",
        "features.shell_snapshot": "false", "features.skip_host_skill_discovery": "true",
        "features.multi_agent": "false", "features.context_management": "false",
        "web_search": '"disabled"',
    }
    if args.catalog:
        settings["model_catalog_json"] = json.dumps(str(args.catalog))
    result = [str(args.codex), "--ask-for-approval", "never", "exec"]
    if session:
        result.append("resume")
    result += ["--ignore-user-config", "--skip-git-repo-check", "--json", "--model", args.model]
    for key, value in settings.items():
        result.extend(["--config", key + "=" + value])
    if session:
        result.append(session)
    result.append(prompt)
    return result


def run_turn(args, proxy, lane, session, folder, number, prompt):
    output = folder / f"turn-{number}.jsonl"
    started = time.monotonic()
    timed_out = False
    with output.open("wb") as stdout, (folder / f"turn-{number}.stderr").open("wb") as stderr:
        process = spawn(command(args, proxy, lane, session, prompt), cwd=folder,
                        stdout=stdout, stderr=stderr)
        try:
            process.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
        finally:
            stop(process)
    events = rows(output)
    messages = [e["item"]["text"] for e in events if e.get("type") == "item.completed"
                and e.get("item", {}).get("type") == "agent_message"]
    thread = next((e.get("thread_id") for e in events if e.get("type") == "thread.started"), session)
    result = {"turn": number, "seconds": round(time.monotonic() - started, 3),
              "exit_code": process.returncode, "timed_out": timed_out,
              "response": messages[-1] if messages else None,
              "completed": any(e.get("type") == "turn.completed" for e in events),
              "tool_items": sum(e.get("type") == "item.completed" and
                                e.get("item", {}).get("type") not in ("agent_message", "reasoning")
                                for e in events)}
    return thread, result


def correct_answer(text, include_audit):
    expected = {"deployment_label": "helios-731", "region": "syd", "budget": 23,
                "recovery_code": "copper-otter"}
    if include_audit:
        expected["audit_code"] = "violet-409"
    try:
        answer = json.loads(text or "")
    except ValueError:
        return False
    return answer == expected and type(answer.get("budget")) is int


def run_arm(args, arm, folder, prompts):
    result = {"arm": arm, "status": "running", "turns": [], "passed": False}
    proxy = Proxy(args, arm, folder)
    started = time.monotonic()
    try:
        proxy.start()
        result["base_url"] = proxy.base_url
        lane, session = str(uuid.uuid4()), None
        before_restart = None
        for number, prompt in enumerate(prompts, 1):
            if number == 6:
                before_restart = snapshot(proxy.state)
                proxy.close()
                if proxy.exit_codes[-1] != 0:
                    raise RuntimeError("proxy_shutdown_failed")
                proxy.start()  # Same state directory, same port, same session.
            session, turn = run_turn(args, proxy, lane, session, folder, number, prompt)
            if number in (3, 5, 6):
                turn["answer_correct"] = correct_answer(turn["response"], number != 3)
                turn["recall_used_no_tools"] = turn["tool_items"] == 0
            result["turns"].append(turn)
            write_json(folder / "result.json", result)
            if turn["timed_out"]:
                raise RuntimeError("client_timeout")
            if turn["exit_code"] != 0 or not turn["completed"] or not session:
                raise RuntimeError("client_did_not_complete")
        summary = summarize_ledger(proxy.state / "ledger.jsonl")
        result["checks"] = {
            "recall": all(t["answer_correct"] and t["recall_used_no_tools"]
                          for t in result["turns"] if "answer_correct" in t),
            "fixtures_read_with_tools": all(result["turns"][i]["tool_items"] > 0 for i in (0, 3)),
            "proxy_observed_completions": summary["response"]["completed"] >= 6,
            "recursive_rollover": summary["compact"]["accepted"] >= 2 if arm == "rolling"
                                  else summary["compact"]["requests"] == 0,
            "projection_reused_after_restart": (len(before_restart or []) == 1 and before_restart[0]["cut"] > 0 and
                                                 before_restart == snapshot(proxy.state))
                                                if arm == "rolling" else True,
            "all_generations_committed": summary["response"]["committed"] == summary["response"]["requests"]
                                         if arm == "rolling" else True,
        }
        result["passed"] = all(result["checks"].values())
        result["status"] = "passed" if result["passed"] else "failed_gate"
    except (OSError, RuntimeError, ValueError) as exc:
        result["status"] = "blocked"
        result["error"] = str(exc)
    finally:
        proxy.close()
        result["seconds"] = round(time.monotonic() - started, 3)
        result["proxy_exit_codes"] = proxy.exit_codes
        result["proxy_requests_received_by_process"] = proxy.request_counts
        try:
            result["usage"] = summarize_ledger(proxy.state / "ledger.jsonl")
        except (OSError, ValueError, KeyError):
            result.update(status="blocked", passed=False, error="unreadable_proxy_ledger")
        if any(code != 0 for code in proxy.exit_codes):
            result.update(status="blocked", passed=False, error="proxy_shutdown_failed")
        write_json(folder / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, help="Use the same supported snapshot for both arms")
    parser.add_argument("--upstream", required=True, help="Explicit API base, without /responses")
    parser.add_argument("--auth", choices=("chatgpt", "api-key"), required=True)
    parser.add_argument("--allow-compatible-compaction", action="store_true")
    parser.add_argument("--compact-path", default="/responses/compact")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/ostk-gpt-cache")
    parser.add_argument("--codex", type=Path, default=Path(shutil.which("codex") or "/opt/codex/bin/codex"))
    parser.add_argument("--catalog", type=Path, help="Optional Codex model catalog JSON (offline discovery)")
    parser.add_argument("--sandbox", choices=("read-only", "workspace-write"), default="read-only")
    parser.add_argument("--output", type=Path, default=ROOT / ".astral-trials" / str(uuid.uuid4()))
    parser.add_argument("--timeout", type=int, default=180, help="Maximum seconds per Codex invocation")
    parser.add_argument("--roll-bytes", type=int, default=16_000)
    parser.add_argument("--seed", type=int, default=731, help="Reproducible random arm order")
    parser.add_argument("--prepare-only", action="store_true", help="Write fixtures and plan; invoke no models")
    args = parser.parse_args()
    if args.timeout < 1 or args.roll_bytes < 1:
        parser.error("timeout and roll-bytes must be positive")
    for name in ("binary", "codex", "output", "catalog"):
        if getattr(args, name) is not None:
            setattr(args, name, getattr(args, name).resolve())
    if not args.prepare_only:
        if not args.binary.is_file() or not args.codex.is_file():
            parser.error("build the proxy and install Codex first, or use --prepare-only")
        if args.auth == "api-key" and not os.environ.get("OPENAI_API_KEY"):
            parser.error("OPENAI_API_KEY must be set for API-key trials")
        from urllib.parse import urlsplit
        upstream = urlsplit(args.upstream)
        platform = upstream.scheme == "https" and upstream.hostname == "api.openai.com" and upstream.port in (None, 443)
        if not platform and not args.allow_compatible_compaction:
            parser.error("compatible trials require explicit --allow-compatible-compaction")
    args.output.mkdir(parents=True, exist_ok=False, mode=0o700)
    order = ["passthrough", "rolling"]
    random.Random(args.seed).shuffle(order)
    report = {"model": args.model, "auth": args.auth, "seed": args.seed, "order": order,
              "prepare_only": args.prepare_only, "arms": [], "passed": False,
              "interpretation": "A single synthetic correctness trial; no cost or quality generalization."}
    for arm in order:
        folder = args.output / arm
        folder.mkdir(mode=0o700)
        prompts = fixtures(folder)
        if not args.prepare_only:
            result = run_arm(args, arm, folder, prompts)
            report["arms"].append(result)
            write_json(args.output / "report.json", report)
            if result["status"] == "blocked":
                break  # Avoid repeating a setup failure or generating partial matched arms.
    report["passed"] = len(report["arms"]) == 2 and all(a["passed"] for a in report["arms"])
    write_json(args.output / "report.json", report)
    print(json.dumps({"report": str(args.output / "report.json"), "passed": report["passed"],
                      "prepare_only": args.prepare_only}))
    return 0 if args.prepare_only or report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
