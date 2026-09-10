# Initial validation receipt

Date: 2026-09-10. Environment: Linux x86_64, Rust/Cargo 1.98.1. All dependencies are public crates.io dependencies; resolution is recorded in `Cargo.lock`.

| Check | Result |
| --- | --- |
| `cargo test --locked` | 22 passed: 10 unit tests and 12 HTTP/persistence integration tests |
| `cargo clippy --all-targets --locked -- -D warnings` | Passed |
| `cargo fmt --all --check` | Passed |
| `cargo build --release --locked --bins` | Passed; proxy and stats binaries built |
| Release proxy startup and `/healthz` | Passed on a dynamically allocated loopback port |
| Release proxy SIGTERM | Clean exit, code 0 |
| Release stats smoke check | Correct separate generation/compaction groups and inclusive-input hit-rate calculation |

The integration suite covers frozen projection reuse, complete canonical compact output, main-upstream failure rollback, truncated and complete SSE, content-type precedence, exact passthrough, provider-managed context exclusions, missing identity, compactor failure fallback, credential/session isolation, same-lane serialization, cross-lane concurrency, persistence restart, process locking, duplicate identity-header rejection, and actual downstream disconnect.

Unit coverage includes recursive roll input, same-length history edits, prompt-contract changes, pending/orphan tool calls, active reasoning/tool-item preservation, legacy/modern cache controls, unknown model behavior, caller precedence, bounded fragmented SSE observation, and inclusive usage accounting.

No live OpenAI inference or compaction requests were made. Tests use local mock servers and do not demonstrate actual cache hits, semantic fidelity, dollar savings, or compatibility with a particular Codex OAuth backend. macOS and Windows CI jobs are configured but were not run here.

The source was implemented independently of `os-tack/haystack`. No private dependency or private source was used.
