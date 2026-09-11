# Validation receipt

Updated: 2026-09-11. Environment: Linux x86_64, Rust/Cargo 1.98.1, Codex CLI 0.154.0-alpha.3. All dependencies are public crates.io dependencies; resolution is recorded in `Cargo.lock`.

| Check | Result |
| --- | --- |
| `cargo test --locked` | 27 passed: 10 unit tests and 17 HTTP/persistence integration tests |
| `cargo clippy --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all --check` | Passed |
| `cargo build --release --locked --bins` | Passed; proxy and stats binaries built |
| Release proxy startup and `/healthz` | Passed on a dynamically allocated loopback port |
| Release proxy SIGTERM | Clean exit, code 0 |
| Release stats smoke check | Correct separate generation/compaction groups and inclusive-input hit-rate calculation |
| Fresh/resumed trial configuration | Both sets of provider overrides parse as TOML; fixture preparation succeeds without model calls |
| Bounded live trial through `trial.base_url` | Blocked: nested Codex timed out before a completed request; runner exits 1 and reports `passed:false` |
| Trial process cleanup | Proxy shut down cleanly with exit code 0 after the client timeout |
| Push to `scottmeyer/astral` | Blocked: GitHub connector write returned HTTP 403, `Resource not accessible by integration`; shell Git has no credentials |

The integration suite covers frozen projection reuse, complete canonical compact output, main-upstream failure rollback, truncated and complete SSE, content-type precedence, exact passthrough, provider-managed context exclusions, missing identity, compactor failure fallback, credential/session isolation, same-lane serialization, cross-lane concurrency, persistence restart, process locking, duplicate identity-header rejection, and actual downstream disconnect. Added tests exercise two successive projections containing parallel tool calls and encrypted reasoning, HTTP reuse after proxy restart, a same-length history edit after a roll, unusable compact outputs, observer overflow with unchanged forwarded bytes, and OAuth account/API-key isolation.

Unit coverage includes recursive roll input, same-length history edits, prompt-contract changes, pending/orphan tool calls, active reasoning/tool-item preservation, legacy/modern cache controls, unknown model behavior, caller precedence, bounded fragmented SSE observation, and inclusive usage accounting.

Validation found and fixed a lane-isolation gap: `chatgpt-account-id`, `api-key`, and `x-api-key` now participate in the identity hash and duplicate-header checks. Explicit API-key headers also suppress the environment credential fallback.

The bounded live attempt created an isolated rolling proxy and a fresh Codex instance with its custom provider's `base_url` set to that proxy. The client timed out after 20 seconds during nested startup. The ledger contains zero generation or compaction records; no live inference result, cache hit, or semantic-compaction receipt is available. The runner correctly failed instead of treating absent traffic as success. See [codex-trials.md](codex-trials.md) for the reproducible six-turn, two-arm protocol.

The passing Rust tests use local mock servers and do not demonstrate actual cache hits, semantic fidelity, dollar savings, or compatibility with a particular Codex OAuth backend. macOS and Windows CI jobs are configured but were not run here. GitHub remains unmodified because publication was denied by the integration.

The source was implemented independently of `os-tack/haystack`. No private dependency or private source was used.
