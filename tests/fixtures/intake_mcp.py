"""Local MCP transport fixture. No credentials, network, or model calls."""

import json
import subprocess
import sys

mode = sys.argv[1] if len(sys.argv) > 1 else "normal"
waiting = None
server_request = None


def send(value):
    print(json.dumps(value), flush=True)


def reply(request, result):
    send({"jsonrpc": "2.0", "id": request["id"], "result": result})


for line in sys.stdin:
    q = json.loads(line)
    method = q.get("method")
    if method is None and server_request is not None:
        reply(server_request, {"content": [{"type": "text", "text": json.dumps(q)}]})
        server_request = None
        continue
    if "id" not in q:
        if method == "notifications/cancelled":
            send(
                {
                    "jsonrpc": "2.0",
                    "method": "notifications/message",
                    "params": q["params"],
                }
            )
        continue
    if method == "initialize":
        reply(
            q,
            {
                "protocolVersion": q["params"]["protocolVersion"],
                "capabilities": {"tools": {"listChanged": True}},
                "serverInfo": {"name": "intake-fixture", "version": "1"},
            },
        )
    elif method == "tools/list":
        names = (
            ["plain", "reverse", "roots", "spawn"]
            if not q.get("params", {}).get("cursor")
            else ["schema"]
        )
        if mode == "collision":
            names.append("astral_recall")
        result = {
            "tools": [
                {
                    "name": n,
                    "description": n,
                    "inputSchema": {"type": "object"},
                    **({"outputSchema": {"type": "object"}} if n == "schema" else {}),
                }
                for n in names
            ]
        }
        if not q.get("params", {}).get("cursor"):
            result["nextCursor"] = "second-page"
        reply(q, result)
    elif method == "tools/call":
        if mode == "crash":
            sys.exit(7)
        name = q["params"]["name"]
        result = (
            q["params"]
            .get("arguments", {})
            .get("result", {"content": [{"type": "text", "text": "small"}]})
        )
        if name == "reverse":
            if waiting is None:
                waiting = (q, result)
                send(
                    {
                        "jsonrpc": "2.0",
                        "method": "notifications/progress",
                        "params": {"progressToken": "working", "progress": 1},
                    }
                )
            else:
                reply(q, result)
                reply(*waiting)
                waiting = None
        elif name == "roots":
            server_request = q
            send(
                {"jsonrpc": "2.0", "id": q["id"], "method": "roots/list", "params": {}}
            )
        elif name == "spawn":
            p = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(60)"],
                stdout=subprocess.DEVNULL,
            )
            reply(q, {"content": [{"type": "text", "text": str(p.pid)}]})
        else:
            reply(q, result)
            if mode == "exit":
                break
    else:
        reply(q, {"echo": q})
