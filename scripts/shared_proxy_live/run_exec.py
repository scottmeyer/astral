"""Bound one harness invocation and retain evidence outside the repository."""

import json
import os
import pathlib
import signal
import subprocess
import sys
import time

root = pathlib.Path(os.environ["ASTRAL_TEST_ROOT"]).resolve()
label = sys.argv[1]
cmd = json.loads(pathlib.Path(sys.argv[2]).read_text())
with (
    (root / "logs" / f"{label}.jsonl").open("w") as out,
    (root / "logs" / f"{label}.stderr").open("w") as err,
):
    p = subprocess.Popen(
        cmd, stdin=subprocess.DEVNULL, stdout=out, stderr=err, start_new_session=True
    )
    start = time.time()
    timeout = False
    try:
        code = p.wait(timeout=240)
    except subprocess.TimeoutExpired:
        timeout = True
        os.killpg(p.pid, signal.SIGTERM)
        try:
            code = p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGKILL)
            code = p.wait()
    result = {
        "command": cmd,
        "exit_code": code,
        "timeout": timeout,
        "seconds": round(time.time() - start, 2),
        "start_ts_ms": round(start * 1000),
        "end_ts_ms": round(time.time() * 1000),
    }
    (root / "logs" / f"{label}.status.json").write_text(json.dumps(result))
    print(
        label,
        json.dumps({k: v for k, v in result.items() if k != "command"}),
        flush=True,
    )
