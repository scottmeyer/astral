#!/usr/bin/env python3
"""Run or resume an agent with an explicit workspace profile and local Astral proxy."""
import argparse
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "python"), str(ROOT / "scripts")]
from astral import Host, Session
from codex_trial import Proxy


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--workspace", type=Path, required=True)
    p.add_argument("--profile", type=Path, required=True)
    p.add_argument("--state-dir", type=Path, required=True)
    p.add_argument("--model", required=True)
    p.add_argument("--upstream", required=True)
    p.add_argument("--auth", choices=("gateway", "api-key"), required=True)
    p.add_argument("--mode", choices=("native", "native-adapted", "astral"), default="astral")
    p.add_argument("--prompt")
    p.add_argument("--max-calls", type=int, default=30)
    p.add_argument("--threshold", type=int, default=8192)
    p.add_argument("--roll-bytes", type=int, default=64000)
    p.add_argument("--min-roll-seconds", type=int, default=60)
    p.add_argument("--timeout", type=int, default=90)
    p.add_argument("--cache-mode", choices=("provider-default", "implicit"), default="provider-default")
    args = p.parse_args()
    if min(args.max_calls, args.threshold, args.roll_bytes, args.timeout) < 1:
        p.error("limits must be positive")
    if args.min_roll_seconds < 0:
        p.error("min-roll-seconds must be nonnegative")
    if args.auth == "api-key" and not os.environ.get("OPENAI_API_KEY"):
        p.error("OPENAI_API_KEY is required")
    args.state_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    args.binary = ROOT / "target/release/ostk-gpt-cache"
    args.compaction_backend = "inline"
    args.inline_tool_boundaries = True
    args.allow_compatible_compaction = True
    proxy = Proxy(args, "rolling" if args.mode == "astral" else "passthrough", args.state_dir)
    host = Host(args.workspace, args.state_dir / "working", args.profile, raw=args.mode == "native")
    try:
        proxy.start()
        session = Session(host, args.state_dir / "session.json", proxy.base_url, args.model, args.mode,
                          args.auth, args.threshold, args.timeout, args.cache_mode, provider_scope=args.upstream)
        print(session.turn(args.prompt, args.max_calls))
    finally:
        proxy.close()
        host.close()


if __name__ == "__main__":
    main()
