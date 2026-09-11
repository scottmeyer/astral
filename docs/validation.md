# Validation receipt

Updated: 2026-09-11. Environment: Linux x86_64, Rust/Cargo 1.98.1, Python 3.12, official stable Codex CLI 0.154.0. All Rust dependencies are public crates.io dependencies; resolution is recorded in `Cargo.lock`.

| Check | Result |
| --- | --- |
| `cargo test --locked` | 29 passed: 10 unit tests and 19 HTTP/persistence integration tests |
| Python trial protocol tests | 4 passed; native SSE output items and terminal completion handling |
| `cargo clippy --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all --check` | Passed |
| `cargo build --release --locked --bins` | Passed; proxy and stats binaries built |
| Release proxy startup, health and SIGTERM | Passed; ingress counts captured and all paired trial proxies exited 0 |
| Stable Codex install and normal login | 0.154.0 installed; ChatGPT login present; provider connectivity check succeeded |
| `codex exec` through `trial.base_url` | Failed: timeout after 45 seconds, no events, zero requests received by Astral |
| Configured gateway TLS | Fixed: additional environment CA loaded with certificate verification enabled |
| Live generation smoke through Astral | HTTP 200, completed response `ASTRAL_LIVE_OK`, 4.943 seconds |
| Matched live API trial | 16/16 generation calls completed; 4 actual function calls; 6/6 recall checks correct across two arms |
| Live native rollover | Failed: standard compact route returned 404; alternate route returned ordinary generations, all 5 rejected |
| Nonempty projection reused after live restart | Not demonstrated; no projection was accepted |
| Push to `scottmeyer/astral` | Blocked: GitHub connector write returned HTTP 403, `Resource not accessible by integration`; shell Git has no credentials |

## Routing and runtime findings

The configured model base is `https://chatgpt.com:18080/backend-api/codex`. Initially Astral could not connect because its Rust TLS client did not trust the environment's CA (`UnknownIssuer`). Astral now reads additional roots from `SSL_CERT_FILE` or `--upstream-ca-bundle`. An empty or invalid configured bundle fails startup. No certificate-verification bypass is used.

After that fix, live Responses inference succeeded through the configured gateway. The gateway supplies its existing authorized routing; the standalone client did not read Codex credentials or supply an authorization header. This does not imply that the public ChatGPT service supports unauthenticated API requests.

Native compaction remains incompatible with the observed gateway. `POST /responses/compact` returned 404. The alternate `POST /compact` returned HTTP 200 with `object: response`, an encrypted `reasoning` item, and an assistant message, but no encrypted `compaction` item. Astral correctly refused to replace history. A route name, smaller response, or encrypted reasoning is insufficient evidence of native compact semantics. The standard suffix remains the default, consistent with [Codex 0.154.0's compact client](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/codex-api/src/endpoint/compact.rs).

The stable nested Codex CLI stalled before HTTP ingress, independently of the working API route. Its timeout is not a successful session even though termination produced child exit code 0. The runner retained `timed_out: true`, `passed: false`, and zero ingress. Normal CLI authentication and runtime requirements were preserved.

## Matched live API result

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

The Rust suite covers frozen projection reuse, recursive rollovers with tool calls and encrypted reasoning, full canonical compact output, exact passthrough, completed/truncated SSE, failure rollback, downstream disconnect, observer overflow, same-lane serialization, cross-lane concurrency, restart, process locking, history edits, prompt-contract changes, and account/session isolation. Routing tests cover both autonomous and caller compaction with a custom path, preserved caller bytes, invalid CA bundles, and ingress counting before identity rejection. Ordinary encrypted reasoning is explicitly rejected as replacement state, with a safe rejection reason recorded.

The Python tests cover complete output-item reconstruction for gateways whose terminal response omits those items, canonical assistant phase, missing completion, and missing item indices. The restart gate now requires a nonempty projection; an unchanged empty snapshot cannot pass.

The passing Rust tests use local mock servers and establish state-machine/protocol behavior. macOS and Windows CI jobs are configured but were not run here. GitHub remains unmodified because the integration denied publication. Source was implemented independently of `os-tack/haystack`; no private dependency or private source was used.
