#!/usr/bin/env python3
"""Full-history Responses client; Python stdlib only. Makes real API calls through the proxy."""
import argparse
import json
import os
import urllib.request
import uuid

parser = argparse.ArgumentParser()
parser.add_argument("--model", required=True)
parser.add_argument("--url", default="http://127.0.0.1:8088/v1/responses")
parser.add_argument("--session", default=str(uuid.uuid4()))
args = parser.parse_args()
history = []
print(f"Session: {args.session}. Ctrl-D to exit.")
while True:
    try:
        text = input("you> ")
    except (EOFError, KeyboardInterrupt):
        print()
        break
    if not text.strip():
        continue
    history.append({"role": "user", "content": text})
    payload = {"model": args.model, "store": False, "input": history}
    headers = {"Content-Type": "application/json", "x-astral-session-id": args.session}
    if os.environ.get("OPENAI_API_KEY"):
        headers["Authorization"] = "Bearer " + os.environ["OPENAI_API_KEY"]
    request = urllib.request.Request(args.url, json.dumps(payload).encode(), headers)
    with urllib.request.urlopen(request, timeout=900) as response:
        result = json.load(response)
    history.extend(result.get("output", []))  # Preserve every item, including encrypted reasoning and phase.
    for item in result.get("output", []):
        for block in item.get("content", []):
            if block.get("type") == "output_text":
                print("gpt>", block["text"])
