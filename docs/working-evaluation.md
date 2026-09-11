# Working-state coding evaluation

Date: 2026-09-11. Model: `gpt-6-astra`, medium reasoning, configured authorized
gateway. The [machine-readable receipt](receipts/working-2026-09-11.json) records
all six trials, individual generations, tools, acceptance checks and usage.

All six tasks passed. There were **103 completed model calls**, with complete
reported usage. Each task repaired real Python code, added a second public
function after a requirements change, passed 400 unseen acceptance cases,
preserved protected test code, recognized stale verification, survived host and
proxy restart, and recovered the exact token from an older successful test run
without rerunning it.

## Three conditions

All conditions used the same eight tools, prompts, check commands and property
tests. Each configured test run produced 600 detailed real case observations.
The raw artifact included a new audit token per execution, making retrieval of
the old token distinguishable from re-execution. Only `total.py` was writable.
Unseen acceptance tests also used much larger integers than the visible cases.

| Condition | Tool output | Context management |
| --- | --- | --- |
| Native | Full captured reports | Provider inline, threshold 8,192 |
| Native + adapter | Compact structured reports, exact retrieval | Same provider inline policy |
| Astral + adapter | Same compact adapter, plus state events and frozen snapshot | Astral inline scheduling, 64,000-byte trigger; provider threshold 8,192 |

Order was randomized independently for seeds 731 and 947. Every condition had
a separate stable session key and instruction nonce. All traffic passed through
Astral; native conditions used its byte-preserving passthrough mode. There were
no automatic retries or minimum checkpoint requirements in this benefit trial.

## Observations

Totals across two tasks per condition:

| Measurement | Native | Native + adapter | Astral + adapter |
| --- | ---: | ---: | ---: |
| Passed tasks | 2/2 | 2/2 | 2/2 |
| Model calls | 37 | 33 | 33 |
| Wall time | 298.901 s | 186.438 s | 137.406 s |
| Tool observation bytes | 545,759 | 35,130 | 48,640 |
| Native checkpoints | 6 | 1 | 0 |
| Reported input tokens | 96,301 | 161,599 | 228,042 |
| Reported cache-read tokens | 0 | 16,512 | 11,648 |
| Reported output tokens | 1,637 | 3,774 | 1,723 |

The adapter reduced model-visible tool bytes by **93.6%** against the raw native
condition. Its observed wall time was **37.6% lower**. That is the adapter's
result; it must not be attributed to Astral's projection planner.

Astral's observed wall time was **26.3% lower than native with the same adapter**,
and **54.0% lower than raw native**. It delivered more state information and
reported more input tokens. No checkpoint was needed under its byte policy in
these tasks, so this result does not demonstrate faster checkpoint creation or
replay. The incremental timing difference is heavily influenced by one native
adapted run crossing its compaction threshold. Per-task times expose that:

| Seed | Native | Native + adapter | Astral + adapter |
| --- | ---: | ---: | ---: |
| 731 | 148.845 s | 119.298 s | 71.554 s |
| 947 | 150.056 s | 67.140 s | 65.852 s |

This supports using compact observations and avoiding unnecessary checkpoints
on this workload. It does not establish a general latency advantage from state
projection, stronger reasoning, or a statistically reliable effect size.

## Why the token counts need care

The raw-native condition repeatedly sent approximately 85–90 KB check reports
and received native checkpoints in those generations. Reported input counts
for those calls were only about 1,300–2,100 tokens, while elapsed times were
about 27–34 seconds. The receipt therefore does not establish the complete cost
of reading and compacting those reports. Inline compaction work is not separately
itemized by this gateway. Treat the small counts as returned usage, not evidence
that the raw reports were free to process.

The gateway also does not provide controlled Platform cache retention in this
trial. Cache hits vary across calls and are not an independently warmed control.
There are no invoice-backed dollar savings, and all input counts above are
inclusive of reported cache reads. Reported cache writes were zero, not inferred
from misses.

## Revisions and separate rollover validation

The matched timing run used implementation commit `e86f276`. A subsequent
review added a durable unseen-event cursor: independent host observations can
no longer consume a file change before the session delivers it to the model.
The cursor has direct regression coverage. These six timings were not rerun
after that follow-up and should not be presented as a measurement of it.

The separate small-threshold rollover trial uses the cursor integration and
exercises native checkpoint adoption, fresh snapshots and restart. Its results
are recorded in the [validation receipt](validation.md). Its deliberately lower
threshold is a robustness test, not another benefit comparison.

That test passed 17 generations, with 12 native compaction items and 10 frozen
snapshots, including successful exact retrieval after restart. It took 515.937
seconds at a 4,096-token/4,096-byte trigger with no cooldown. This is a concrete
counterexample to indiscriminate rollover: a new state snapshot can immediately
cross an overly low threshold again. The standalone reference agent now exposes
the existing cooldown and defaults it to 60 seconds. The benchmark still uses
zero. See [the stress receipt](receipts/working-rollover-2026-09-11.json).

Reproduce with `scripts/working_trial.py`; see [the host guide](working-state.md)
for commands and limitations. Only the sanitized receipt is checked in; complete
native encrypted items, raw artifacts and session files remain private trial data.
