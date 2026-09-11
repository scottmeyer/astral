"""Real executable property checks with detailed per-case observations.

The Astral adapter retains group failures and warnings. Every original case
and the per-execution audit token remain in the immutable raw report.
"""
import json
from pathlib import Path
import random
import sys
import uuid
import total as implementation

requirements = json.loads(Path("requirements.json").read_text())
rng = random.Random(requirements["seed"])
cases = []
failures = []
groups = 0


def group(name, observations):
    global groups
    groups += 1
    bad = [r for r in observations if not r["ok"]]
    if bad:
        failures.append({"test": name, "failed_cases": len(bad), "examples": bad[:5]})
    cases.extend(observations)


def observe(fn, args, expected, label):
    try:
        actual = fn(*args)
        ok = type(actual) is int and actual == expected
    except Exception as error:
        actual, ok = type(error).__name__ + ": " + str(error), False
    return {"case": label, "args": args, "expected": expected, "actual": actual, "ok": ok}


group("empty", [observe(implementation.total, ([], 0), 0, "empty")])
group("half_up", [observe(implementation.total, ([(1, 1)], 5000), 1, "half")])
group("no_discount", [observe(implementation.total, ([(2, 123), (3, 456)], 0), 1614, "ordinary")])
property_cases = []
for index in range(600):
    items = [(rng.randrange(0, 100), rng.randrange(0, 100000)) for _ in range(rng.randrange(1, 8))]
    bps = rng.randrange(0, 10001)
    expected = (sum(q * p for q, p in items) * (10000 - bps) + 5000) // 10000
    property_cases.append(observe(implementation.total, (items, bps), expected, f"property-{index}"))
group("integer_property", property_cases)
invalid = []
for value in (-1, 10001):
    try:
        implementation.total([(1, 100)], value)
        ok = False
    except ValueError:
        ok = True
    except Exception:
        ok = False
    invalid.append({"case": f"invalid-discount-{value}", "ok": ok})
group("discount_bounds", invalid)
if requirements["phase"] >= 2:
    fn = getattr(implementation, "subtotal", lambda _: None)
    group("subtotal", [observe(fn, ([(2, 123), (3, 456)],), 1614, "subtotal"), observe(fn, ([],), 0, "empty-subtotal")])
report = {"schema": "astral.check.v1", "passed": groups - len(failures), "failed": len(failures),
          "failures": failures, "warnings": [], "cases": cases, "audit_token": "run-" + uuid.uuid4().hex}
print(json.dumps(report, separators=(",", ":")))
sys.exit(1 if failures else 0)
