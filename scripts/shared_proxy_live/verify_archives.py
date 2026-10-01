"""Recover complete archived bodies over HTTP and verify hashes and fixtures."""

import hashlib
import json
import os
import pathlib
import urllib.request

r = pathlib.Path(os.environ["ASTRAL_TEST_ROOT"]).resolve()
result = []
url = os.environ.get("ASTRAL_TEST_URL", "http://127.0.0.1:18088").rstrip("/")
state = pathlib.Path(os.environ.get("ASTRAL_TEST_STATE", str(r / "shared-state")))


def texts(v):
    if isinstance(v, str):
        yield v
        try:
            yield from texts(json.loads(v))
        except (ValueError, RecursionError):
            pass
    elif isinstance(v, list):
        for x in v:
            yield from texts(x)
    elif isinstance(v, dict):
        for x in v.values():
            yield from texts(x)


expectations = {
    p.stem[:-9]: json.loads(p.read_text())
    for p in (r / "controller").glob("*.expected.json")
}
for p in (state / "artifacts").glob("*.json"):
    a = json.loads(p.read_text())
    content = list(texts(a["data"]))
    matches = [k for k, v in expectations.items() if v["text"] in content]
    page_offset = 0
    actual = bytearray()
    pages = 0
    while True:
        with urllib.request.urlopen(
            f"{url}/_astral/artifacts/{p.stem}?offset={page_offset}&limit=4093",
            timeout=15,
        ) as f:
            page = json.load(f)
        actual.extend(
            bytes.fromhex(page["data"])
            if page["encoding"] == "hex"
            else page["data"].encode()
        )
        pages += 1
        if page["eof"]:
            break
        assert page["next_offset"] > page_offset
        page_offset = page["next_offset"]
    row = {
        "handle": p.stem,
        "scope": a["scope"],
        "artifact_sha256": a["sha256"],
        "sha256_verified": hashlib.sha256(actual).hexdigest() == a["sha256"],
        "bytes": len(actual),
        "pages": pages,
        "exact_match": bytes(actual) == a["data"].encode(),
        "fixtures_matching_completely": matches,
    }
    result.append(row)
assert result, "No archives found; routing alone is not an archival pass"
assert all(
    x["sha256_verified"] and x["exact_match"] and x["fixtures_matching_completely"]
    for x in result
)
(r / "paged-recovery.json").write_text(json.dumps(result, indent=2))
print(
    json.dumps(
        [{k: v for k, v in x.items() if k not in ["handle", "scope"]} for x in result],
        indent=2,
    )
)
