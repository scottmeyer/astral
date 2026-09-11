# Measured benefit of rolling projection

Executed 2026-09-11 with `gpt-6-astra` through the configured Codex gateway. Astral-scheduled inline compaction reduced reported input tokens by **69.4%** against full history, while increasing total wall time by **22.1%**. Provider-managed inline compaction reduced input by **83.6%**. All six tasks passed. This demonstrates useful context reduction and working persistent rollover; it does not demonstrate that Astral's current scheduler improves on native provider compaction or lowers verified charges.

## Matched protocol

`scripts/benefit_trial.py` ran two task seeds, 731 and 947, across three arms. Each task had nine user turns and eleven generation calls: two actual tool reads, a correction from budget 17 to 23, intervening acknowledgments, then final structured recall after restarting the proxy. Each tool result contained approximately 112 KB of synthetic data. Four random string fields appeared only in those results; all intermediate visible replies had to be exactly `READY`.

The arms used the same fixture and prompts per seed, tool schema, medium reasoning effort, `reasoning.context: all_turns`, and `store:false`. Every arm had a stable distinct cache key and a distinct instruction nonce to reduce cross-arm cache priming. There was no server-side conversation or previous-response reference. The gateway can override routing keys; these settings do not guarantee isolated provider caches.

| Arm | Client history | Compaction policy |
| --- | --- | --- |
| Full history | Complete original history | None; proxy passthrough |
| Provider inline | Checkpoint and complete suffix after each native roll | Provider threshold 8,192 tokens on every call; proxy passthrough |
| Astral inline | Complete original history, including returned checkpoints | Astral byte trigger 64,000; safe new-user boundary; provider threshold 8,192 tokens on scheduled calls |

Astral used one retained-turn eligibility setting, minimum compactable prefix 4,096 bytes, zero cooldown, and its default 15% wire-size acceptance gate. In inline mode the provider controls checkpoint contents and does not guarantee a fixed raw tail. No standalone compactor calls were made. Inline work is included in observed response latency and reported generation usage; its billing is not separately itemized here.

Arm order was randomized: provider/full/Astral for seed 731, then Astral/provider/full for seed 947. Calls ran sequentially. The requested model name was an alias, not a pinned model snapshot. There were no recovery calls or retries in these successful runs.

## Observed results

Totals across **two tasks and 22 completed calls per arm**, including both rollover responses and proxy lifecycle time:

| Metric | Full history | Provider inline | Astral inline |
| --- | ---: | ---: | ---: |
| Correct final tasks | 2/2 | 2/2 | 2/2 |
| Reported input tokens, inclusive | 562,268 | 92,490 | 171,921 |
| Reported cache-read tokens | 39,936 | 0 | 0 |
| Reported cache-write tokens | 0 | 0 | 0 |
| Reported output tokens | 322 | 2,360 | 2,566 |
| Native checkpoints emitted | 0 | 4 | 4 |
| Checkpoints adopted by Astral | 0 | 0 | 4 |
| Total wall time | 90.595 s | 114.235 s | 110.608 s |
| Mean wall time per task | 45.298 s | 57.118 s | 55.304 s |
| Upstream request-body bytes | 3,192,094 | 551,710 | 989,760 |
| Mean final input-array bytes | 227,519 | 5,544 | 5,328 |

Input totals include cache reads and writes; those counters must not be added again. All 66 generation calls had usage records. The recorded run predates the runner's additional explicit usage-presence gate; that gate was checked against the saved receipts afterward and passed for every arm. The raw trial receipt is preserved unchanged.

| Seed | Arm | Input tokens | Output tokens | Wall time |
| --- | --- | ---: | ---: | ---: |
| 731 | Full history | 281,128 | 160 | 45.534 s |
| 731 | Provider inline | 46,316 | 1,208 | 61.196 s |
| 731 | Astral inline | 85,223 | 1,002 | 52.816 s |
| 947 | Full history | 281,140 | 162 | 45.061 s |
| 947 | Provider inline | 46,174 | 1,152 | 53.039 s |
| 947 | Astral inline | 86,698 | 1,564 | 57.792 s |

Astral used 85.9% more input than provider inline. The observed request sequence explains an important part of the difference: provider inline checkpointed while processing each large tool result; Astral deferred until the next user turn and sent that large result through one additional generation. Astral was 3.2% faster than provider inline in aggregate, but the ordering reversed between seeds. Two trials do not establish a latency advantage.

The four Astral rollover responses consumed 53.665 seconds, or 48.5% of its total wall time. At the six matched call positions per task after rollover that did not themselves roll in either arm, Astral totaled 36.693 seconds versus full history's 49.442 seconds. This is a descriptive slice of the same small sample, not an independent performance test. The complete task remained slower after paying for rollover.

## What passed

Both Astral tasks adopted two genuine native encrypted checkpoints, reused the first while producing the second, and loaded the second after a process restart. Each had eleven committed generation records with no history or prompt-contract reset. Projection hash, cut, and epoch matched across restart. The final projected-input hash reconstructed from persisted state matched the proxy's hash of the actual outbound input.

All four random strings were absent from each compacted final request's plaintext input, yet every string and the corrected budget was recovered exactly. The budget correction itself is not a hidden-string control. These results support retention through opaque checkpoints on this synthetic task; they do not establish general semantic fidelity or the independent benefit of replaying encrypted reasoning. The separate [opaque replay probe](opaque-replay.md) covers live reasoning-item preservation.

Local HTTP tests also passed for two successive checkpoints with full original client history, output before the checkpoint, unchanged cache controls, active tool continuation, replay edits, truncated responses, missing output-item indices, and client disconnect after a checkpoint has already arrived. In failure cases, the old durable mapping remains in force and the lane can be retried. Bounded-observer tests cover conflicting items and canonical JSON/SSE equivalence. These failure controls use mock upstreams rather than billed live failures.

## Cache and cost interpretation

Full history reported a 7.1% cache-read fraction; the compacted arms reported no reads. A separate calibration repeated the same 5,053-input-token request twice and reported zero reads and writes both times. The response showed legacy `24h` retention and an overridden cache key. Sending modern `prompt_cache_options` returned HTTP 400, `Unsupported parameter: prompt_cache_options`. These observations do not establish controllable cache warming, actual retention duration, or trustworthy production billing on this gateway.

Astral preserves caller cache controls and freezes the checkpoint between rolls. Neither action pins the provider's cache. No API Platform cache controls were injected into these compatible-backend trials.

A rate sensitivity illustrates why token reduction alone is insufficient. Let `I` be inclusive input, `C` cache reads, `W` cache writes, `O` output, and `r` the output/input price ratio. Under the documented modern API Platform read multiplier 0.1 and write multiplier 1.25, hypothetical input-price units are:

```text
(I - C - W) + 0.1*C + 1.25*W + r*O
```

Applying those multipliers to the observed counters gives Astral 354,404.6 fewer input-price units but 2,244 extra output tokens. Its break-even output/input ratio would be approximately 157.9. This is arithmetic under an assumed billing model, not a gateway charge estimate. If a production full-history baseline instead cached roughly 77.1% of its input, the measured Astral input advantage would disappear even before charging for its extra output, assuming Astral still received no cache hits and writes were zero. At an illustrative output/input ratio of 6, that baseline threshold falls to 74.5%. [API Platform cache semantics and multipliers](https://developers.openai.com/api/docs/guides/prompt-caching).

The next decision is whether safe scheduling closer to tool-result ingestion and a reliable warmed-cache baseline improve the complete-task outcome. This experiment supports keeping the inline backend as an opt-in implementation; it does not justify making it the default on performance grounds. Useful follow-up tasks should include substantive reasoning and code acceptance checks, less repetitive context, more rollovers, longer post-roll reuse, and provider billing reconciliation. No statistical confidence or general quality claim follows from two seeds.

## Reproduce and inspect

```bash
cargo build --release --locked --bins
python3 scripts/benefit_trial.py \
  --model gpt-6-astra \
  --auth gateway \
  --upstream https://chatgpt.com:18080/backend-api/codex \
  --seeds 731,947 \
  --cache-mode provider-default \
  --output .astral-trials/benefit-matched
```

Gateway authentication mode is only for existing authorized routing. For API Platform use your configured API key/upstream and, for a supported modern model, `--cache-mode implicit` so all three arms receive identical explicit cache controls. Provider-default mode on modern Platform models would otherwise let Astral's cache policy add a field absent from the passthrough arms. Compatibility and model availability must be checked for the actual backend.

Safe receipts: [complete matched run](receipts/benefit-2026-09-11.json), [derived metrics and usage checks](receipts/benefit-analysis-2026-09-11.json), and [cache calibration](receipts/cache-calibration-2026-09-11.json). Raw response streams, encrypted checkpoint bodies, and private proxy state are excluded. See [inline backend](inline-backend.md) for the original-history mapping and commit contract.
