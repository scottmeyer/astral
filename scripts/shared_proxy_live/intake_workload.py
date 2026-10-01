#!/usr/bin/env python3
"""Qualify first-delivery MCP intake with normal Codex or Claude history management.

Start one separate Astral proxy in passthrough mode. Set ASTRAL_TEST_ROOT to a
private directory outside the repository; credentials stay with the harness.
Labels beginning with 'matched' share a controller fixture for paired trials.
"""

import argparse
import json
import os
import subprocess
import sys
import urllib.request

from workload import BINARY, ROOT, SCRIPTS, URL, make_fixture, rows

PROMPTS = [
    "This is a synthetic tool-output test. Throughout it, use only the fixture MCP tools, including its Astral retrieval tools if useful. Do not use shell, files, web, saved transcripts, or any other tools. Discover the fixture tools in ALL_TOOLS if needed. Call fixture_once exactly once. When using code mode, forward all returned text and use max_output_tokens=120000. Do not echo, summarize, or memorize hidden_marker values in your reply. Reply only LOADED.",
    "Call pulse sequentially with n=1, n=2, and n=3, waiting for each result. Do not reread or regenerate the fixture. Reply only PULSED.",
    "Use no tools. Reply only READY.",
    "Call pulse sequentially with n=4, n=5, and n=6, waiting for each result. Do not reread or regenerate the fixture. Reply only PULSED.",
    "Use no tools. Reply only READY.",
    "What was hidden_marker_beta in the original fixture? Use the retained evidence or the available Astral search and recall tools as needed. If using retrieval, search for the field name and read the relevant page; do not reread the whole fixture. Do not read files, saved transcripts, or rerun fixture_once. Reply only with the exact value.",
]


def run(label, harness, passthrough, policy):
    folder, source, expected = make_fixture(label)
    args = ["mcp-wrap", "--proxy-url", URL, "--session-id", label, "--policy", policy]
    if passthrough:
        args.append("--passthrough")
    args += ["--", sys.executable, str(SCRIPTS / "fixture_mcp.py"), str(source)]
    mcpfile = folder / "mcp.json"
    mcpfile.write_text(
        json.dumps({"mcpServers": {"fixture": {"command": str(BINARY), "args": args}}})
    )
    session = None
    receipts = []
    for step, prompt in enumerate(PROMPTS):
        if harness == "codex":
            cmd = ["codex", "-C", str(folder), "exec"] + (["resume"] if session else [])
            cmd += [
                "--json",
                "--skip-git-repo-check",
                "--ignore-user-config",
                "-m",
                "gpt-6-astra",
            ]
            settings = {
                "openai_base_url": json.dumps(
                    URL + "/providers/codex/backend-api/codex"
                ),
                "features.enable_request_compression": "false",
                "features.shell_tool": "false",
                "model_reasoning_effort": '"low"',
                "mcp_servers.fixture.command": json.dumps(str(BINARY)),
                "mcp_servers.fixture.args": json.dumps(args),
                "mcp_servers.fixture.default_tools_approval_mode": '"approve"',
                "mcp_servers.fixture.tools.fixture_once.output_token_limit": "200000",
            }
            for key, value in settings.items():
                cmd += ["-c", key + "=" + value]
            if session:
                cmd.append(session)
            cmd.append(prompt)
            env = None
        else:
            cmd = [
                "claude",
                "-p",
                "--model",
                "claude-opus-5-5",
                "--output-format",
                "stream-json",
                "--verbose",
                "--strict-mcp-config",
                "--mcp-config",
                str(mcpfile),
                "--tools",
                "",
                "--allowedTools",
                "mcp__fixture__fixture_once,mcp__fixture__pulse,mcp__fixture__astral_search,mcp__fixture__astral_recall",
            ]
            if session:
                cmd += ["--resume", session]
            cmd += ["--", prompt]
            env = dict(os.environ, ANTHROPIC_BASE_URL=URL)
        command_path = folder / f"command-{step}.json"
        command_path.write_text(json.dumps(cmd, indent=2))
        subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "run_exec.py"),
                f"{label}-{step}",
                str(command_path),
            ],
            cwd=folder,
            env=env,
            check=True,
        )
        events = rows(ROOT / "logs" / f"{label}-{step}.jsonl")
        status = json.loads((ROOT / "logs" / f"{label}-{step}.status.json").read_text())
        if harness == "codex":
            session = next(
                (e["thread_id"] for e in events if e["type"] == "thread.started"),
                session,
            )
            terminal = [e for e in events if e["type"] == "turn.completed"]
            calls = [
                e["item"]
                for e in events
                if e["type"] == "item.completed"
                and e.get("item", {}).get("type") == "mcp_tool_call"
            ]
            failed = sum(c.get("status") != "completed" for c in calls)
            answers = [
                e["item"]["text"]
                for e in events
                if e["type"] == "item.completed"
                and e.get("item", {}).get("type") == "agent_message"
            ]
            answer = answers[-1] if answers else ""
        else:
            session = next(
                (e["session_id"] for e in events if e.get("session_id")), session
            )
            terminal = [
                e for e in events if e["type"] == "result" and not e.get("is_error")
            ]
            failed = sum(
                bool(b.get("is_error"))
                for e in events
                if e["type"] == "user"
                for b in e.get("message", {}).get("content", [])
                if isinstance(b, dict) and b.get("type") == "tool_result"
            )
            answer = terminal[-1].get("result", "") if terminal else ""
        receipt = {
            "step": step,
            "session": session,
            "complete": bool(terminal),
            "failed_tools": failed,
            "seconds": status["seconds"],
            "exit_code": status["exit_code"],
            "timeout": status["timeout"],
            "usage": terminal[-1].get("usage") if terminal else None,
        }
        receipts.append(receipt)
        (folder / f"answer-{step}.txt").write_text(answer)
        (folder / "receipts.json").write_text(json.dumps(receipts, indent=2))
        if status["timeout"] or status["exit_code"] or not terminal or failed:
            raise RuntimeError(f"{label} failed at step {step}; evidence retained")
        if step == 0 and source.exists():
            raise RuntimeError("Fixture was not consumed")
    result = {
        "label": label,
        "harness": harness,
        "passthrough": passthrough,
        "policy": policy,
        "session": session,
        "correct_marker": answer.strip() == expected["markers"]["hidden_marker_beta"],
        "fixture_sha256": expected["sha256"],
        "fixture_bytes": expected["bytes"],
        "seconds": round(sum(r["seconds"] for r in receipts), 3),
        "receipts": receipts,
    }
    with urllib.request.urlopen(URL + "/healthz", timeout=5) as response:
        result["health"] = json.load(response)
    (folder / "summary.json").write_text(json.dumps(result, indent=2))
    print(json.dumps({k: v for k, v in result.items() if k != "receipts"}), flush=True)
    if not result["correct_marker"]:
        raise RuntimeError("Incorrect final marker; evidence retained")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("label")
    parser.add_argument("--harness", choices=["codex", "claude"], default="codex")
    parser.add_argument("--passthrough", action="store_true")
    parser.add_argument(
        "--policy", choices=["repetitions", "preview"], default="repetitions"
    )
    options = parser.parse_args()
    run(options.label, options.harness, options.passthrough, options.policy)
