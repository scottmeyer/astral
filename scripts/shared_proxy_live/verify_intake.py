#!/usr/bin/env python3
"""Verify exact intake originals via HTTP paging, including after proxy restart."""

import hashlib
import json
import os
import pathlib
import urllib.parse
import urllib.request

root = pathlib.Path(os.environ["ASTRAL_TEST_ROOT"]).resolve()
url = os.environ.get("ASTRAL_TEST_URL", "http://127.0.0.1:18088").rstrip("/")
expected_hashes = {
    json.loads(p.read_text())["sha256"]
    for p in (root / "controller").glob("*.expected.json")
}
checks = []
for path in sorted((root / "state/artifacts").glob("*.json")):
    artifact = json.loads(path.read_text())
    original = artifact["data"].encode()
    assert hashlib.sha256(original).hexdigest() == artifact["sha256"]
    recovered = bytearray()
    pages = 0
    while True:
        query = urllib.parse.urlencode({"offset": len(recovered), "limit": 4093})
        with urllib.request.urlopen(
            f"{url}/_astral/artifacts/{path.stem}?{query}", timeout=5
        ) as response:
            page = json.load(response)
        data = (
            bytes.fromhex(page["data"])
            if page["encoding"] == "hex"
            else page["data"].encode()
        )
        assert page["sha256"] == artifact["sha256"]
        assert page["offset"] == len(recovered)
        recovered.extend(data)
        pages += 1
        if page["eof"]:
            break
    assert recovered == original
    result = json.loads(recovered)
    assert len(result["content"]) == 1 and result["content"][0]["type"] == "text"
    fixture_hash = hashlib.sha256(result["content"][0]["text"].encode()).hexdigest()
    assert fixture_hash in expected_hashes
    query = urllib.parse.urlencode({"query": "hidden_marker_beta=", "offset": 0})
    with urllib.request.urlopen(
        f"{url}/_astral/artifacts/{path.stem}?{query}", timeout=5
    ) as response:
        search = json.load(response)
    assert search["matches"][0]["offset"] == original.index(b"hidden_marker_beta=")
    checks.append(
        {
            "artifact_sha256": artifact["sha256"],
            "fixture_sha256": fixture_hash,
            "bytes": len(original),
            "pages": pages,
            "exact_recovery": True,
            "search_offset_correct": True,
        }
    )
assert checks, "No actual intake archives"
receipt = {"artifact_count": len(checks), "checks": checks}
(root / "intake-recovery.json").write_text(json.dumps(receipt, indent=2))
print(json.dumps(receipt, indent=2))
