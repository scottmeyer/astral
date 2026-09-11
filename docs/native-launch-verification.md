# Native project launch verification — 2026-09-11

This receipt covers the native launcher and headless worker changes based on
`bf7d7122bb4e3ed9c845faaa2ac60f96bb08d2b0`. Earlier imported results were treated as
historical. Changes were implemented in a separate worktree and checked locally.

## Local checks

- `cargo fmt --check` and `git diff --check`: passed.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- `cargo test --locked`: **216 tests passed**.
- `cargo build --release --locked --bins`: passed.
- `python3 -m unittest discover -s tests -p 'test_*.py'`: **18 passed**, after the
  release helpers were built.

Coverage includes raw native staging, configuration/model/workspace rejection
before payload injection, typed-import fidelity gates, same-thread resume with
no reinjection, exact caller argv and null headless stdin, child exit propagation,
owned child cancellation, private receipt schema/identity/filesystem checks and
real subprocess lock exclusion. Managed-proxy tests exercise real local HTTP and
websocket transports, active downstream/upstream connection teardown, port and
store-lock release, startup failure and unrelated-listener survival.

The first full suite encountered one stale test expectation: a nonexistent root
with `--proxy` now reports `INVALID_ROOT`, replacing the prior unimplemented-proxy
error. That assertion was updated and the full suite rerun successfully.

## Live disposable controls

Installed Codex **0.154.0**, model **gpt-6-astra**, built-in **openai** provider,
effort **xhigh**, current account authentication, **read-only** sandbox and
**never** approval policy were used on macOS. All native launches used Astral's
owned loopback proxy, fixed HTTPS Codex upstream, native tool rebinding,
compression disabled and Astral rolling disabled. No original recovery session,
authentication file, installed client or pre-existing proxy was modified.

| Control | Actual result |
| --- | --- |
| Native bundle → new thread → headless execution | `pwd` invoked; exit 0; exact disposable workspace |
| Receipt-based cold resume | Same thread; new owned proxy; `pwd` invoked; exit 0 |
| Independent import for continuity | Exact opaque canary recovered; zero terminal calls |
| Read-only restriction on execution thread | Linked native call/result recorded exit 1 and permission denial; protected file unchanged |
| Release-binary cold resume | Same thread; `pwd` invoked; exit 0 |
| Release-binary new recall import | Exact opaque canary recovered; zero terminal calls |
| Source/destination audit | Both source items and encrypted checkpoint hashes preserved; source items appear once; current-context injection appears once across resumes |

The source fixture used a new disposable thread. It read a randomly generated
canary through a tool result and completed explicit native compaction. The full
resulting two-item window was captured from that fixture's own rollout, with a
successful completed-compaction boundary and no inherited history, rollback or
inter-agent events. This controlled diagnostic capture is not a production
exporter. No item was removed to make recall pass.

The canary was absent from the entire readable exported window. Recall used a
separate imported thread, and its answer was checked against a private digest.
The execution/cold-resume parent received no readable canary answer; its complete
readable rollout still lacked the target after testing. Recall is an independent
import control, not a claim that the execution parent emitted the answer.

The first source attempt placed the canary in a user message; Codex retained
that message during compaction. The harness rejected it as unsuitable for opaque
recall. That failed attempt was retained and not counted as a pass. The second
fixture initially lacked a required project description; manifest validation
rejected it, and its fixture metadata was corrected before launch.

The restriction turn had no `command_execution` notification. Its actual denial
was independently established from the linked native custom-tool result, rather
than inferred from the assistant's statement or the unchanged file alone.

## Astral dogfood

A real worker was launched through `astral project --non-interactive --work
AST-7esjfvymym3v`, with the selected project documents and exact work record,
read-only policy and an explicit review prompt. It used terminal tools, reviewed
the requested source, and correctly reported the work ID and worktree. It made no
edits. This was an Astral-spawned Codex process, separate from the collaboration
agents used to develop the implementation.

The worker reported no concrete issue in its limited inspection but did not
locate the matching Codex source/schema, and files changed during its review.
That result demonstrates the worker launch/context path; it is not complete
compatibility verification. Independent development reviews against the matching
local Codex source found cancellation and import-fidelity issues; those were
fixed and covered before the full checks above.

## Scanner and remaining limits

UBS **5.4.2** completed `ubs --staged --ci --no-cargo` over 18 Rust files with
**13 critical, 1,457 warning and 414 informational findings**, exiting 1. This
was not a clean scanner result. Critical findings were test failure assertions,
the existing syscall-success-guarded `assume_init`, a literal test comparison
misidentified as a secret, and trusted host/test executable selection. Warning
groups were primarily test assertions/unwraps, bounded indexing and allocation,
fixture path construction and a fixture socket lifetime heuristic. Reviewed
production indexing has input/bounds guards; receipts and project files use
confined readers. No actionable issue from those groups remained. UBS's Cargo
and dependency-audit phases were deliberately skipped; the separate Cargo checks
above are the build evidence, not a dependency security audit.

These controls do not qualify cross-account transfer, other providers/models or
Codex versions, arbitrary imported metadata, automatic worktrees, save/export,
semantic merging of opaque histories, or long-horizon multi-worker throughput.
Native bundles remain data; current runtime supplies executable authority.
There is no deterministic-output, token-saving, cache-hit or invoice-saving claim.

Private fixture events, receipts, source-attempt records, audit results, scanner
and build logs are retained outside tracked source under
`/private/tmp/astral-native-project-live-h3ouwxmp`, with the controller and private
state files at `/private/tmp/astral-native-launch-trial-20260911*`. Native payloads,
the plaintext canary and transcript bodies are not included in this receipt.
