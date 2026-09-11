# Human CLI output and Astral naming

Date: 2026-09-11. Implemented from `76938f5` in an isolated worktree.
These checks were rerun for this change; earlier receipts remain historical.

## Behavior

The Cargo package and Rust crate are `astral`, matching the executable. Active
source, tests, Python helpers, examples and scripts use the current runtime
names: `ASTRAL_UPSTREAM`, `.astral-runtime`, `x-astral-*`, `astral:` generated
cache keys, and `astral_proxy_error`. The lockfile's only semantic change is
the package name; dependency versions, checksums and features are unchanged.

The main CLI prints human summaries, explanations and next commands by default.
`--json` selects indented JSON, including help/version envelopes and errors.
Machine result fields and exit statuses remain unchanged. `project` and `init`
consume Astral's `--json` before the first literal `--`; subsequent arguments
remain literal Codex arguments. Git/Codex child streams, hook callbacks,
`hook-check`, and helper protocols retain their own output formats.

Human hook setup still confirms a captured plan in a terminal. JSON mode
previews without prompting unless `--yes` or an explicit `--apply` hash is
supplied. Piped `yes` does not authorize a change. Lifecycle diagnostics name
the difference and provide review commands for the selected repository.
Suggested follow-up commands preserve an explicit `--root`, including when
invoked from another checkout. Paths are quoted without shell interpretation;
non-UTF-8 paths are never converted into misleading lossy commands.

## Current local checks

- All **474 Rust tests** passed, none failed or ignored, with the locked graph
  and isolated Git signing settings. The **46 presentation/CLI/terminal tests**
  passed again after final follow-up wording changes.
- All **18 Python tests** passed using the newly built release helper.
- Formatting, whitespace checks, strict all-target Clippy, and the locked
  Rust **1.85.0** all-target check passed.
- All release binaries built successfully. Ten direct release CLI controls
  covered help, version, context list/validation, project inspection, lifecycle,
  hook status, explicit JSON and the misspelled `lifecyle` suggestion.
- Active source/configuration searches found no previous namespace. Historical
  receipts and immutable native exports were retained. The checked-in native
  bundle manifest and payload still match their recorded SHA-256 digests.

The tests cover real local HTTP/WebSocket forwarding, native binding fixtures,
literal child argv, worker/save/recovery behavior, and terminal hook confirmation.
No live provider inference or remote CI jobs were run for this presentation and
naming change.

## Scanner review

The required staged UBS JSON and verbose scans both completed with no failed
scanner modules and exited 1 for heuristic findings. Across 51 Rust files and
4 Python files they reported **26 critical, 6,178 warning and 1,174 informational**
matches. This is not a clean scanner result.

All critical locations were reviewed: 13 test panic branches, 4 fixed test-binary
invocations, 2 terminal fixture `finish()` calls mistaken for token generation,
1 literal configuration comparison mistaken for a secret comparison, and 6
outgoing HTTP header constructions mistaken for response-header injection.
The latter use typed/fixed Rust headers or operator-supplied Python credentials
through the HTTP client. None establish the reported vulnerability.

New CLI warnings were also reviewed: serialization of known values, a work-ID
branch guarded by outer dispatch, bounded indexing/text, and intentional terminal
output. No introduced defect was confirmed; no suppression was added. UBS's
Cargo/audit phases were skipped with `--no-cargo`; the independent Cargo checks
above ran against the full checkout.

Private logs and release smoke output are retained outside tracked source.

## Installed release

Source commit: `0ae93ce50ac6055f2a0dbba5641759d1f431b438`.
The verified binary was archived and atomically promoted to
`~/.local/bin/astral` as an independent regular file, separate from build output.
Its SHA-256 is
`6854a6010d9e715db87fe4563fa35817d412907486556eb479160687b1b47d52`.

The previous installed release remains archived. Git hook scripts, registration
and `.codex/hooks.json` matched their pre-install hashes. PATH-based version,
context-list, lifecycle and explicit-JSON hook-status controls passed in the
main checkout. Codex runtime trust remains unobserved by this check.
