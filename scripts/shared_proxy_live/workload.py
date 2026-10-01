#!/usr/bin/env python3
"""Run four real model turns against a separately started shared proxy.

Fixture output is random, one-shot, and deleted on consumption. Expected values,
commands, JSONL transcripts, and receipts stay in ASTRAL_TEST_ROOT. This is a
live qualification driver, not scripts/codex_trial.py's native rolling test.
"""

import argparse
import hashlib
import json
import os
import pathlib
import secrets
import subprocess
import sys

SCRIPTS = pathlib.Path(__file__).resolve().parent
REPO = SCRIPTS.parent.parent
ROOT = pathlib.Path(os.environ["ASTRAL_TEST_ROOT"]).resolve()
if ROOT.is_relative_to(REPO):
    raise SystemExit("ASTRAL_TEST_ROOT must be outside the repository")
os.umask(0o077)
ROOT.mkdir(parents=True, exist_ok=True)
(ROOT / "controller").mkdir(exist_ok=True)
(ROOT / "logs").mkdir(exist_ok=True)
BINARY = pathlib.Path(
    os.environ.get("ASTRAL_TEST_BINARY", str(REPO / "target/release/astral"))
).resolve()
URL = os.environ.get("ASTRAL_TEST_URL", "http://127.0.0.1:18088").rstrip("/")
PROMPTS = [
    "This is a synthetic context-proxy test. Use only the fixture and astral MCP tools throughout this test; do not use shell, files, other tools, or saved transcripts. Call fixture_once exactly once. Its full output must enter your tool history. Do not repeat or summarize any hidden_marker values in your reply; reply only LOADED.",
    "Call pulse six times, sequentially with n=1 through n=6, waiting for each result before the next call. Do not call fixture_once again. Do not mention any fixture markers. Reply only AGED.",
    "Use no tools in this turn. Reply only READY. Keep the prior synthetic tool history.",
    "Recover hidden_marker_beta from the archived fixture. Use astral_search to find its byte offset and actually call astral_recall for the original page, then reply only with its exact value. Do not read any files, saved transcripts, or rerun fixture_once. If no Astral handle exists in your context, reply NO_ARCHIVE_HANDLE.",
]


def rows(path):
    return (
        [json.loads(l) for l in path.read_text().splitlines() if l.strip()]
        if path.exists()
        else []
    )


def make_fixture(label):
    folder = ROOT / label
    folder.mkdir()
    source = folder / "fixture-once.txt"
    expected = ROOT / "controller" / f"{label}.expected.json"
    # Paired arms can reuse the same controller fixture; all values stay out of prompts.
    base = ROOT / "controller" / "matched-fixture.json"
    if label.startswith("matched") and base.exists():
        data = json.loads(base.read_text())
    else:
        markers = {
            f"hidden_marker_{s}": secrets.token_hex(20)
            for s in ("alpha", "beta", "gamma")
        }
        blocks = ["BEGIN synthetic fixture\n"]
        for i in range(540):
            blocks.append(
                f"{i:04d} ordinary synthetic padding; all routine checks complete.\n"
            )
            if i in (150, 270, 420):
                k = list(markers)[(150, 270, 420).index(i)]
                blocks.append(k + "=" + markers[k] + "\n")
        blocks.append("END synthetic fixture\n")
        text = "".join(blocks)
        data = {
            "text": text,
            "markers": markers,
            "sha256": hashlib.sha256(text.encode()).hexdigest(),
            "bytes": len(text.encode()),
        }
        if label.startswith("matched"):
            base.write_text(json.dumps(data))
    source.write_text(data["text"])
    expected.write_text(json.dumps(data))
    return folder, source, data


def codex_config(label, source, diagnostic):
    result = []
    settings = {
        "features.enable_request_compression": "false",
        "mcp_servers.astral.command": json.dumps(str(BINARY)),
        "mcp_servers.astral.args": json.dumps(["mcp", "--proxy-url", URL]),
        "mcp_servers.fixture.command": json.dumps(sys.executable),
        "mcp_servers.fixture.args": json.dumps(
            [str(SCRIPTS / "fixture_mcp.py"), str(source)]
        ),
    }
    if diagnostic:
        result += ["--ignore-user-config", "-m", "gpt-6-astra"]
        settings.update(
            {
                "mcp_servers.fixture.default_tools_approval_mode": '"approve"',
                "mcp_servers.astral.default_tools_approval_mode": '"approve"',
                "mcp_servers.fixture.tools.fixture_once.output_token_limit": "20000",
            }
        )
        settings.update(
            {
                "model_provider": '"astral_trial"',
                "model_reasoning_effort": '"low"',
                "model_providers.astral_trial": '{name="Astral HTTP test",base_url="'
                + URL
                + '/providers/codex/backend-api/codex",wire_api="responses",requires_openai_auth=true,supports_websockets=false,http_headers={"x-astral-workspace-id"="'
                + label
                + '","x-astral-session-id"="'
                + label
                + '","x-astral-harness-id"="codex-live-test"}}',
            }
        )
    else:
        settings["openai_base_url"] = json.dumps(
            URL + "/providers/codex/backend-api/codex"
        )
    for k, v in settings.items():
        result += ["-c", k + "=" + v]
    return result


def run(label, diagnostic=False, claude=False, full_history=False):
    folder, source, data = make_fixture(label)
    config = codex_config(label, source, diagnostic)
    sid = None
    all_results = []
    for step, prompt in enumerate(PROMPTS):
        if claude:
            mcp = {
                "mcpServers": {
                    "astral": {
                        "command": str(BINARY),
                        "args": ["mcp", "--proxy-url", URL],
                    },
                    "fixture": {
                        "command": sys.executable,
                        "args": [str(SCRIPTS / "fixture_mcp.py"), str(source)],
                    },
                }
            }
            mcpfile = folder / "mcp.json"
            mcpfile.write_text(json.dumps(mcp))
            cmd = [
                "claude",
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--strict-mcp-config",
                "--mcp-config",
                str(mcpfile),
                "--tools",
                "",
                "--allowedTools",
                "mcp__fixture__fixture_once,mcp__fixture__pulse,mcp__astral__astral_search,mcp__astral__astral_recall",
            ]
            if sid:
                cmd += ["--resume", sid]
            cmd += ["--", prompt]
        else:
            cmd = ["codex", "-C", str(folder), "exec"]
            if sid:
                cmd += ["resume"]
            cmd += ["--skip-git-repo-check", "--json"] + config
            if sid:
                cmd.append(sid)
            cmd.append(prompt)
        cmdfile = folder / f"command-{step}.json"
        cmdfile.write_text(json.dumps(cmd, indent=2))
        env = None
        if claude:
            env = dict(os.environ, ANTHROPIC_BASE_URL=URL)
            if full_history:
                env["CLAUDE_CODE_EXTRA_BODY"] = '{"context_management":null}'
        subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "run_exec.py"),
                f"{label}-{step}",
                str(cmdfile),
            ],
            cwd=folder,
            env=env,
            check=True,
        )
        events = rows(ROOT / "logs" / f"{label}-{step}.jsonl")
        all_results.extend(events)
        if claude:
            for e in events:
                if e.get("session_id"):
                    sid = e["session_id"]
            done = [e for e in events if e.get("type") == "result"]
            status = bool(done and not done[-1].get("is_error")) and not any(
                block.get("is_error")
                for e in events
                if e.get("type") == "user"
                for block in e.get("message", {}).get("content", [])
                if isinstance(block, dict) and block.get("type") == "tool_result"
            )
            answer = done[-1].get("result") if done else None
        else:
            for e in events:
                if e.get("type") == "thread.started":
                    sid = e["thread_id"]
            status = any(e.get("type") == "turn.completed" for e in events) and not any(
                e.get("type") == "item.completed"
                and (
                    e.get("item", {}).get("status") == "failed"
                    or (e.get("item", {}).get("result") or {}).get("isError")
                )
                for e in events
            )
            messages = [
                e["item"]["text"]
                for e in events
                if e.get("type") == "item.completed"
                and e.get("item", {}).get("type") == "agent_message"
            ]
            answer = messages[-1] if messages else None
        receipt = json.loads(
            (ROOT / "logs" / f"{label}-{step}.status.json").read_text()
        )
        status = status and receipt["exit_code"] == 0 and not receipt["timeout"]
        print(
            json.dumps(
                {
                    "label": label,
                    "step": step,
                    "completed": status,
                    "answer": answer
                    if step != 3
                    else (
                        "MATCH"
                        if answer
                        and answer.strip() == data["markers"]["hidden_marker_beta"]
                        else "NO_MATCH"
                    ),
                    "session": sid,
                }
            ),
            flush=True,
        )
        (folder / "session.json").write_text(
            json.dumps({"session": sid, "step": step, "completed": status})
        )
        if not status or not sid:
            break
    (folder / "summary.json").write_text(
        json.dumps(
            {
                "label": label,
                "session": sid,
                "fixture_bytes": data["bytes"],
                "fixture_sha256": data["sha256"],
                "fixture_consumed": not source.exists(),
                "completed_steps": step + 1,
                "last_completed": status,
                "recall_answer_correct": bool(
                    step == 3
                    and answer
                    and answer.strip() == data["markers"]["hidden_marker_beta"]
                ),
            },
            indent=2,
        )
    )
    calls = []
    unexpected = []
    for event in all_results:
        if claude and event.get("type") == "assistant":
            calls.extend(
                block["name"].split("__")[-1]
                for block in event["message"]["content"]
                if block.get("type") == "tool_use"
            )
        elif event.get("type") == "item.completed":
            item = event["item"]
            if item.get("type") == "mcp_tool_call":
                calls.append(item["tool"])
            elif item.get("type") in ["command_execution", "file_change", "web_search"]:
                unexpected.append(item["type"])
    allowed = {"fixture_once", "pulse", "astral_search", "astral_recall"}
    unexpected.extend(name for name in calls if name not in allowed)
    summary_path = folder / "summary.json"
    summary = json.loads(summary_path.read_text())
    summary.update(
        {
            "tool_calls": {name: calls.count(name) for name in sorted(set(calls))},
            "unexpected_tools": unexpected,
            "workload_pass": status
            and step == 3
            and not source.exists()
            and calls.count("fixture_once") == 1
            and calls.count("pulse") == 6
            and not unexpected,
        }
    )
    summary["recall_pass"] = (
        summary["workload_pass"]
        and summary["recall_answer_correct"]
        and calls.count("astral_recall") > 0
    )
    summary_path.write_text(json.dumps(summary, indent=2))
    if not summary["workload_pass"]:
        raise SystemExit("Workload failed; inspect the private logs and summary.json")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="Live shared tool-history archival trial. Uses normal harness authentication; never reads credential files. Keep ASTRAL_TEST_ROOT outside the repo. Start one standalone astral proxy separately."
    )
    parser.add_argument(
        "label",
        help="Fresh session folder; labels starting matched reuse controller/matched-fixture.json for identical workloads",
    )
    parser.add_argument(
        "--http",
        action="store_true",
        help="Explicit Codex Responses provider, ChatGPT auth, WebSockets disabled, test MCP tools approved",
    )
    parser.add_argument(
        "--claude", action="store_true", help="Use Claude Code and ANTHROPIC_BASE_URL"
    )
    parser.add_argument(
        "--full-history",
        action="store_true",
        help="Claude diagnostic only: set context_management=null through CLAUDE_CODE_EXTRA_BODY",
    )
    args = parser.parse_args()
    if not args.label or any(
        c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_"
        for c in args.label
    ):
        parser.error("Use an alphanumeric label with dashes or underscores")
    if args.full_history and not args.claude:
        parser.error("--full-history requires --claude")
    run(
        args.label,
        diagnostic=args.http,
        claude=args.claude,
        full_history=args.full_history,
    )
