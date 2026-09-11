# Unified CLI verification

Date: 2026-09-11. Checks were rerun on the local checkout after the binary rename
and default-context changes; earlier receipts remain historical.

The build now declares `astral`, `astral-state` and `stats` binary targets, with
`astral` as `cargo run`'s default. No `ostk-gpt-cache` binary target remains; the
internal crate keeps its name. Proxy startup/shutdown moved to `src/server.rs`.
Use `astral proxy`; bare invocation and direct proxy options retain the former
server behavior. Project/context commands and help do not start the server.

`astral project` selects `project-context` when the name is omitted. Its inspection
output matches explicit `astral project project-context --inspect`. Explicit
context names, forwarded options/prompts and `--` boundaries retain their meaning.
Native staging and Codex spawning are still pending; without `--inspect`, the
project command reports `LAUNCH_NOT_IMPLEMENTED`.

## Current verification

- Formatting and all-target Clippy with warnings denied: PASS.
- Locked Rust suite: **108 passed**, zero failed or ignored.
- Locked release binary build: PASS.
- Python suite: **18 passed**.
- Actual CLI fixtures: both `astral proxy --listen 127.0.0.1:0` and direct
  `astral --listen 127.0.0.1:0` started, served a successful health response with
  the expected version, and made zero connections to their loopback upstream.
  Tests terminated only their own spawned processes.
- CLI help/version, default-context parsing, direct/proxy inspection, and literal
  argument forwarding: PASS.
- Release checks on the repository's own `.astral/`: validation, explicit/default
  inspection equivalence, forwarded argv and expected launch-unavailable error PASS.
- Cargo metadata confirms the three intended binary targets and default run target.

Rust test counts: 13 library, 2 CLI, 13 launcher, 24 native binding, 9 native
transport, 17 project, 25 proxy and 5 working-state. Execution was on macOS;
cross-platform CI has not been run in this pass. No live provider tests or restart
of the conversation's recovery proxy occurred. Existing compiled artifacts under
old names are not evidence of current Cargo targets.

UBS was repaired and scanned the staged changes, with heuristic findings rather
than a clean scan. See [development checks](development-checks.md) for the exact
scope, staged-snapshot workaround and triage. Source review found no blocker in
the unified command dispatch, extracted server or default-context implementation.
