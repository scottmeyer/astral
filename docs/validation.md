# Validation receipt

Updated: 2026-09-11. Environment: Linux x86_64, Rust/Cargo 1.98.1, Python 3.12, official stable Codex CLI 0.154.0. All Rust dependencies are public crates.io dependencies; resolution is recorded in `Cargo.lock`.

| Check | Result |
| --- | --- |
| `cargo test --locked` | 43 passed: 13 unit tests, 25 HTTP/persistence tests, 5 working-state tests |
| Python protocol and host tests | 12 passed; native item replay, real executable checks, exact retrieval, stale verification, frozen epochs, provider binding and paged state delivery |
| `cargo clippy --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all --check` | Passed |
| `cargo build --release --locked --bins` | Passed; proxy, stats and `astral-state` binaries built |
| Release proxy startup, health and SIGTERM | Passed; ingress counts captured and all paired trial proxies exited 0 |
| Stable Codex install and normal login | 0.154.0 installed; ChatGPT login present; provider connectivity check succeeded |
| `codex exec` through `trial.base_url` | Failed: timeout after 45 seconds, no events, zero requests received by Astral |
| Configured gateway TLS | Fixed: additional environment CA loaded with certificate verification enabled |
| Live generation smoke through Astral | HTTP 200, completed response `ASTRAL_LIVE_OK`, 4.943 seconds |
| Matched live API trial | 16/16 generation calls completed; 4 actual function calls; 6/6 recall checks correct across two arms |
| Live standalone rollover | Failed: standard compact route returned 404; alternate route returned ordinary generations, all 5 rejected |
| Live inline native rollover | Passed: two compactions, recursive checkpoint replay, four random strings and corrected budget recalled after restart with no plaintext strings in final input |
| Live encrypted reasoning replay | Passed in a separate three-call probe, including restart; hashes unchanged |
| Astral-owned projection reused after live restart | Passed in both new inline-backend trials; nonempty projection, epoch and cut unchanged, actual outbound input hash verified |
| Three-arm live benefit comparison | 66/66 calls completed; all six tasks passed; Astral input down 69.4%, wall time up 22.1% versus full history |
| New three-arm live coding comparison | 103/103 calls completed; six tasks passed, each including 400 unseen cases and exact old-artifact retrieval after restart |
| Working-state rollover stress | 17/17 calls completed; 12 native checkpoints, 10 frozen snapshots, restart and exact recovery passed; aggressive settings took 515.937 seconds |
| Push to `scottmeyer/astral` | Blocked: GitHub connector write returned HTTP 403, `Resource not accessible by integration`; shell Git has no credentials |

## Routing and runtime findings

The configured model base is `https://chatgpt.com:18080/backend-api/codex`. Initially Astral could not connect because its Rust TLS client did not trust the environment's CA (`UnknownIssuer`). Astral now reads additional roots from `SSL_CERT_FILE` or `--upstream-ca-bundle`. An empty or invalid configured bundle fails startup. No certificate-verification bypass is used.

After that fix, live Responses inference succeeded through the configured gateway. The gateway supplies its existing authorized routing; the standalone client did not read Codex credentials or supply an authorization header. This does not imply that the public ChatGPT service supports unauthenticated API requests.

The standalone compact routes remain incompatible with the observed gateway. `POST /responses/compact` returned 404. The alternate `POST /compact` returned HTTP 200 with `object: response`, an encrypted `reasoning` item, and an assistant message, but no encrypted `compaction` item. Astral correctly refused to replace history. A route name, smaller response, or encrypted reasoning is insufficient evidence of native compact semantics. The standard suffix remains the default, consistent with [Codex 0.154.0's compact client](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/codex-api/src/endpoint/compact.rs). A later inline test succeeded through `/responses`, as described below.

The stable nested Codex CLI stalled before HTTP ingress, independently of the working API route. Its timeout is not a successful session even though termination produced child exit code 0. The runner retained `timed_out: true`, `passed: false`, and zero ingress. Normal CLI authentication and runtime requirements were preserved.

## Working-state host and executable coding task

The [new host](working-state.md) stores exact artifacts by SHA-256, records
versioned files and checks in a durable journal, detects stale verification and
replays completed action IDs after restart. A persisted session cursor delivers
unseen events even if another host observer discovered the change first. User
constraint records, agent declarations and executable check receipts remain
distinct. Frozen snapshots supplement native continuation state; they do not
decode or replace encrypted reasoning.

In the [new coding comparison](working-evaluation.md), all six tasks passed the
actual code repair, second-phase requirements, 400 unseen acceptance cases,
stale-check detection, process restart and retrieval of an original execution's
token without a rerun. Model-visible tool bytes fell 93.6% using the deterministic
adapter. Astral's observed wall time was 26.3% lower than native with the same
adapter, but it reported more input tokens. Zero checkpoints were needed in
the two Astral benefit runs. Two seeds and uncontrolled gateway cache behavior
do not establish general savings. The measured revision and subsequent cursor
follow-up are explicitly identified in the receipt.

The separate [cursor-enabled rollover stress receipt](receipts/working-rollover-2026-09-11.json)
passed all 17 generations and code/retrieval gates with a 4,096-token threshold,
4,096-byte trigger and zero cooldown. It observed and adopted 12 native
compaction items, persisted 10 frozen state snapshots, and preserved workspace,
projection and snapshot state across restart. Multiple checkpoints can arrive
in one response; a fresh snapshot is appended before the following generation.
The final response may leave that refresh pending for the next turn.

This deliberately aggressive policy took **515.937 seconds**, reported 92,238
input tokens and 9,357 output tokens, and demonstrates repeated compaction
overhead. It is evidence of rollover correctness, not performance benefit. The
user-facing reference agent exposes a cooldown and defaults it to 60 seconds;
the benchmark retains zero. The economic hook still requires caller estimates.

## Astral-owned inline rollover and benefit

The opt-in [inline backend](inline-backend.md) now lets Astral schedule native compaction on the working Responses route and map the checkpoint back to the client's full original history. In the [matched three-arm evaluation](benefit-evaluation.md), both Astral trials committed two native checkpoints, reused the second after restart, and recovered all random strings and the corrected budget with those strings absent from the actual final upstream input. Both full-history controls and both provider-managed inline controls also passed.

Across two tasks per arm, Astral used 171,921 input tokens and 110.608 seconds, versus full history's 562,268 tokens and 90.595 seconds. Provider inline used 92,490 tokens and 114.235 seconds. Astral's safe-boundary schedule sends each large tool result through an additional generation before rolling. The observed reduction is useful, but no latency or input-token advantage over provider compaction is established. Cache warming and gateway billing remain unresolved; output and rollover work are included in the receipt.

## Successful inline compaction and opaque replay

The gateway accepts native `context_management` compaction on the normal `/responses` route. A three-call probe emitted genuine encrypted compaction and reasoning items and replayed both unchanged after a proxy restart. A stronger six-call trial performed two successive compactions with actual tool results, never echoed its random canaries before the final question, and recovered all four random strings and the corrected budget after restarting the proxy and reloading the saved client window. The final input contained none of those strings in plaintext. Both trials passed.

See [opaque replay](opaque-replay.md) for the reproducible command, distinct inline/standalone handling rules, and [canary receipt](receipts/inline-canaries-2026-09-11.json). This validates provider-owned recursive compaction through Astral's passthrough path. It does not validate the autonomous standalone planner, general semantic fidelity, or cache economics. The earlier failed trials below remain evidence of their specific routes and configurations.

## Earlier matched live API result

Both arms used `gpt-6-astra`, low reasoning effort, the same function schema and synthetic fixtures, stable per-arm keys, and distinct instruction prefixes to reduce shared-prefix cache priming. Each performed six user turns, two actual fixture reads, three tool-free recall checks, and a proxy restart. Each fixture contained approximately 108 KB. Arm order was randomized with seed 731.

| Observation | Rolling arm | Passthrough arm |
| --- | ---: | ---: |
| Completed generations | 8 | 8 |
| Correct recall checks | 3/3 | 3/3 |
| Accepted native projections | 0 of 5 attempts | No attempts |
| Generation input tokens, inclusive | 182,419 | 182,427 |
| Generation cache-read tokens | 0 | 0 |
| Generation output tokens | 165 | 165 |
| Attempted-compaction input tokens, inclusive | 127,356 | 0 |
| Attempted-compaction cache-read tokens | 90,879 | 0 |
| Attempted-compaction reported cache-write tokens | 36,363 | 0 |
| Attempted-compaction output tokens | 208 | 0 |
| Wall time, including proxy lifecycle | 58.099 s | 35.675 s |
| Requests received before/after restart | 7 / 1 | 7 / 1 |

The overall gate is **failed**. The rolling arm retained full history because its compact outputs were unusable. Its extra calls consumed reported tokens without yielding a projection. Those usage records describe requests made to the configured compact route, which behaved as ordinary generation here. They are not evidence of effective native compaction or verified charges. No semantic fidelity, cache-retention duration, cost improvement, or production-quality claim follows from this fixture. API Platform cache controls were not injected into this compatible backend.

Safe machine-readable receipts: [matched API trial](receipts/api-gateway-2026-09-11.json), [stable Codex timeout](receipts/codex-stable-2026-09-11.json), and [routing diagnostics](receipts/routing-2026-09-11.json). The earlier alpha Codex failure remains in [its original receipt](receipts/codex-2026-09-11.json). Raw response streams, opaque items, and private proxy state are excluded from the repository.

## Regression coverage and limits

The Rust suite covers frozen projection reuse, recursive rollovers with tool calls and encrypted reasoning, full canonical compact output, exact passthrough, completed/truncated SSE, failure rollback, downstream disconnect, observer overflow, same-lane serialization, cross-lane concurrency, restart, process locking, history edits, prompt-contract changes, and account/session isolation. Routing tests cover both autonomous and caller compaction with a custom path, preserved caller bytes, invalid CA bundles, and ingress counting before identity rejection. Ordinary encrypted reasoning is explicitly rejected as replacement state, with a safe rejection reason recorded. New inline cases cover original-history mapping across two checkpoints, an earlier output item before the checkpoint, preserved cache controls, active tool continuation, replay edits, missing indices, and disconnect after checkpoint arrival. Observer tests check aggregate capture bounds, conflicting items, and JSON/SSE equivalence.

The Python tests cover complete output-item reconstruction for gateways whose terminal response omits those items, canonical assistant phase, missing completion, and missing item indices. They also check preservation of the complete suffix after inline compaction, rejection of malformed checkpoints, and refusal to prune on ordinary reasoning alone. The standalone restart gate requires a nonempty projection; an unchanged empty snapshot cannot pass.

New host tests execute real property checks, bind passing results to tracked
revisions, reject stale file/state compare-and-swap writes, protect configured
read-only paths, detect tampered artifacts, replay completed actions exactly and
reject interrupted actions with uncertain outcomes. They cover torn journal
recovery, constraint source history, provider-scope changes, external edits,
frozen snapshot bytes and paged delivery of more than 16 unseen state events.
New HTTP tests exercise complete parallel tool batches and positive, negative
and malformed economic estimates without changing upstream native items.

The passing Rust tests use local mock servers and establish state-machine/protocol behavior. macOS and Windows CI jobs are configured but were not run here. GitHub remains unmodified because the integration denied publication. Source was implemented independently of `os-tack/haystack`; no private dependency or private source was used.
