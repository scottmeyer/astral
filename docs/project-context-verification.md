# Project context integration verification

Date: 2026-09-11. This records checks rerun on the integrated local checkout,
after the native recovery review, semantic rename, context resolver and argument
plumbing were merged. Imported historical results are separate evidence.

## Implemented boundary

- `native_binding` and `native_transport` replace work-item-bound module and test
  names. `NativeToolBindingMode` and `--native-tool-binding` are the primary API;
  the old flag remains a hidden compatibility alias. Diagnostic names and the
  private trial helper use matching semantic names.
- The `astral` binary validates and lists the committed context catalog, then
  resolves a named subsystem/projection and optional JSONL work item. Reads and
  output are bounded; Unix descriptor-relative access rejects escaping paths,
  symlinks and nonregular files. The native binding remains explicitly UNBOUND.
- Project argument inspection defaults to direct routing; `--proxy` opts in.
  Raw user arguments remain unchanged, including explicit permission flags.
  The reusable process builder uses OS argument strings without a shell, and
  adds no permission flags. The CLI does not yet invoke that process builder.

Independent reviews checked source confinement, native naming parity and argument
boundaries. The argv review caught a moved Rust path and a missing root value
that could consume `--`; both were fixed before final verification. A regression
also checks the complete JSON output budget after adding the argument preview.

## Current local checks

Environment: macOS, Rust/Cargo 1.95.0, Python 3.14.7.

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --all-targets --locked -- -D warnings` | PASS, no lint allowances |
| `cargo test --locked` | 104 passed, 0 failed, 0 ignored |
| `cargo build --release --locked --bins` | PASS |
| `python3 -m unittest discover -s tests -p 'test_*.py'` | 18 passed |

The Rust count comprises 13 library, 24 native binding, 9 native transport,
17 project resolver, 11 launcher argument, 25 existing proxy and 5 working-state
tests. Transport tests use real local sockets with synthetic upstreams. Argument
tests compile and execute a harmless fixture that records exact OS argument bytes
and its working directory. They cover quotes, metacharacters, empty/newline values,
explicit permission flags, separator handling, and Unix non-UTF-8 bytes.

Release-CLI checks against this repository also passed: validation of 13 work
items, listing both named contexts, subsystem/work and projection inspection,
exact direct/proxy argv previews, explicit UNBOUND native state, and the expected
`LAUNCH_NOT_IMPLEMENTED` error without `--inspect`.

During the rename worktree check, the first Python invocation had five missing
`astral-state` release-binary errors. Building the release binaries and rerunning
the suite resolved those errors. The final integrated sequence builds before
running Python. Local socket tests required authorized execution outside the
sandbox's loopback restriction. The UBS scanner remains unavailable because its
downloaded module failed integrity validation, as recorded in the
[native integration receipt](native-recovery-integration.md); it is not a pass.

## Evidence limits and remaining work

No live provider tests or recovery-proxy restart were performed in this integration
pass. The running conversation still uses the earlier recovery runtime. Historical
execution/recall evidence retains the version and route scope recorded in the
[native lifecycle receipt](native-recovery-lifecycle.md).

Native artifact availability, staging/import/resume, workspace binding, proxy
health/lifetime management and real `astral project` launch remain pending.
Omitting `--inspect` returns `LAUNCH_NOT_IMPLEMENTED`. The argv fixture proves
process argument transport, not successful Codex startup or native context
restoration. Explicit Codex configuration overrides can change the effective
route; the inspection preview reports only the requested route.

No opaque checkpoint payload, private transcript, credential or machine-specific
runtime binding was added to portable project context. Local commits and merges
have not been pushed.
