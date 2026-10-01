#!/usr/bin/env python3
import json
import pathlib
import sys

source = pathlib.Path(sys.argv[1])
for line in sys.stdin:
    q = {}
    try:
        q = json.loads(line)
        method = q.get("method")
        result = {}
        if "id" not in q:
            continue
        if method == "initialize":
            result = {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "synthetic-one-shot", "version": "1"},
            }
        elif method == "tools/list":
            result = {
                "tools": [
                    {
                        "name": "fixture_once",
                        "description": "Return the complete synthetic test fixture exactly once. Consumes and deletes the source; subsequent calls fail.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {},
                            "additionalProperties": False,
                        },
                    },
                    {
                        "name": "pulse",
                        "description": "Return one small synthetic tool result to age history.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"n": {"type": "integer"}},
                            "required": ["n"],
                            "additionalProperties": False,
                        },
                    },
                ]
            }
        elif method == "tools/call":
            name = q["params"]["name"]
            if name == "fixture_once":
                if source.exists():
                    content = source.read_text()
                    source.unlink()
                    result = {
                        "content": [{"type": "text", "text": content}],
                        "isError": False,
                    }
                else:
                    result = {
                        "content": [
                            {
                                "type": "text",
                                "text": "Fixture consumed; cannot regenerate.",
                            }
                        ],
                        "isError": True,
                    }
            elif name == "pulse":
                result = {
                    "content": [
                        {
                            "type": "text",
                            "text": f"pulse {q['params']['arguments']['n']} complete",
                        }
                    ],
                    "isError": False,
                }
            else:
                raise ValueError("unknown tool")
        print(
            json.dumps({"jsonrpc": "2.0", "id": q["id"], "result": result}), flush=True
        )
    except (ValueError, KeyError, TypeError, OSError) as e:
        print(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": q.get("id"),
                    "error": {"code": -32603, "message": type(e).__name__},
                }
            ),
            flush=True,
        )
