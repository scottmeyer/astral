# Lifecycle integration verification — 2026-09-11

Implemented on `feat/lifecycle-integration`, based on
`5bc7f2d32dc9144e925b3ff6b9be840a2d708362`. These are dated results from the
implementation worktree, not a claim about future edits or another checkout.
The [workflow guide](lifecycle-integration.md) records event ownership and limits.

## Current local checks

- Full locked Rust suite: **424 passed**, zero failures. Three subsequently added
  Git signing/environment tests and one ordinary checkout/merge test also passed,
  bringing the distinct tested cases to **428**. After the final review fixes,
  all **39** lifecycle, installer, forwarding, Codex-adapter and deadline tests
  passed again. Repeated fixture runs are not counted as additional tests.
- All **18 Python tests passed** against the newly built release helper.
- `cargo fmt --all --check`, Clippy for all targets with warnings denied, release
  builds of all binaries, and the Rust **1.85.0** all-target check passed.
- The checked-in project context validates. No dependency or lockfile change was
  needed. The original checkout's preexisting lockfile edit was preserved.

The tests use real local Git/process/filesystem fixtures and synthetic native
items. They cover candidate versus working context, `commit --only` temporary
indexes, `commit -a`, alternate indexes, SHA-256 repositories, unborn and detached
HEAD, index v4, exact native bytes, unsafe entries and stale snapshot rejection.
Split-index rejection preserves shared-index bytes and modification timestamps.

Installed fixture shims run through ordinary commit, checkout and fast-forward
merge. Foreign hooks, configured hook managers, signing failures, explicit skip
flags, literal argument/environment forwarding and Git exit behavior are covered.
Other tests exercise partial installation, unchanged-plan checks, retained edited
hooks, raw JSON preservation, backups, linked-worktree scope and private cache
bounds. Callbacks do not write worker receipts, stage context, or invoke Codex.

Busy-worker context changes, mismatched session IDs, changed branches and project
identity, delayed/duplicate events and pending-operation evidence remain visible
without adoption or repair. Session starts force a fresh advisory; a following
prompt may suppress the duplicate. Failed output sinks remain advisory successes.
The timeout fixture killed the checker, its Git process and a descendant in about
**3.03 seconds**. A current-project debug check observed 19 `.astral` files in
**1.484 seconds**, exit 0. This is one timing observation, not a scalability claim.

## Installed Codex dispatch

Actual installed **Codex 0.154.0** ran against a loopback canned Responses provider
using a disposable home, repository and configuration. Account authentication was
not supplied. No Authorization header or unexpected provider route was observed.
These controls test the installed client and real hook executables; they do not
perform live inference or native checkpoint recovery.

The final release Astral binary had SHA-256
`1aa3f09d3ecc4315a62e71cf08622273e05b7ccfeda058e8f015b1154fe5982a`.

| Control | Observed result |
| --- | --- |
| Untrusted disposable hook configuration | Exit 0; no Astral advisory delivered |
| Trusted startup with real Astral callback | Exit 0; one advisory; immediate prompt duplicate suppressed |
| Cold resume of the same fixture session | Exit 0; a fresh advisory despite the unchanged context digest |
| Stop callback in the synthetic dispatch fixture | No extra inference request or continuation loop |
| Project files | Declared file hashes unchanged |

Each of the three final-release cases made exactly **one mocked response request**.
The resumed request contained the prior advisory and one new advisory. Trusted
controls explicitly bypassed hook trust only for their reviewed disposable
fixture. The separate untrusted control was skipped normally. User trust, global
configuration, installed Git hooks and original sessions were unchanged.

A separate synthetic command-hook fixture recorded actual `SessionStart`,
`UserPromptSubmit` and `Stop` invocations, including `source=resume` and the same
session ID. The direct Astral fixture verifies model-context delivery, while
`codex exec --json` omits individual hook notifications. **Stop UI visibility was
not observed in that frontend. PostCompact remains operationally NOT_TESTED.**
Those output schemas are covered structurally and checked against the installed
source/schema and [official hook guide](https://learn.chatgpt.com/docs/hooks).
Manual interactive trust approval, imported native contexts, other Codex versions,
live providers and Linux execution were not qualified by these controls.

## Review and static scan

The staged UBS static scan completed for **27 Rust files**, reporting **2 critical,
886 warning and 418 informational findings**, exiting 1. This is not a clean scanner result.
The two critical flags were reviewed in source: an existing `assume_init` follows
a successful `fstatat` check, and the new index parser compares the public four-byte
extension tag `link`, not a secret or cryptographic signature. Neither represents
the condition suggested by the scanner's label.

The warning categories include fixture assertions/unwraps, checked slices and
integer conversions, bounded collection allocation/cloning, and path construction.
Production conversions follow exact-size slices; process reads are bounded by
buffers/deadlines; private paths use descriptor-relative checks and Git snapshot
identity verification. Static reports scan whole staged files, including unchanged
code and tests. Cargo checks ran separately because UBS used `--no-cargo`.

Review found and fixed two additional cases: an invalid existing binding could be
hidden after a project-ID/branch change, and a failed stderr write could panic in
a Git callback. Regression coverage now preserves the diagnostic and successful
advisory exit respectively. Earlier development failures included a Darwin
`mode_t` mismatch, a fixture missing `update-index --add`, and split-index timestamp
refreshes; each was corrected or made explicitly unsupported before final checks.

Raw runtime fixtures and scan/build logs remain in the private lifecycle evidence
directory outside tracked source. No transcript, credential or checkpoint payload
is included in this receipt. Concurrent noncooperating configuration edits during
installation, strict policy enforcement and broader runtime/platform support
remain outside this first integration pass.

## Terminal setup confirmation — 2026-09-11

The follow-up on `feat/hook-install-confirmation`, based on
`579089a31c3e7a55a6377901df8c551d99fbe9f0`, adds a same-command plan and confirmation
for hook installation/removal. The installer and its exact-plan checks are shared
with `--yes` and the retained `--apply HASH` interface; `--dry-run` always previews.

The complete locked Rust suite passed **440 tests**, including **12 new tests**
using real Unix pseudo-terminals on macOS. They cover Git/Codex install and removal,
affirmative input, negative/blank/unknown input, EOF and unterminated answers,
no writes before approval, changes after the prompt, foreign-hook blockers,
nonterminal previews, redirected stderr, flag conflicts and legacy hash application.
Changed files during review produced `HOOK_PLAN_CHANGED` and preserved their bytes.

Formatting, strict all-target Clippy, the Rust **1.85.0** all-target check, locked
release builds of all binaries and all **18 Python tests** passed. The first Python
attempt had five missing-helper errors because the shared Cargo target directory
was outside the worktree. Copying the freshly built helper into the harness's
expected `target/release` location resolved that setup failure; the complete suite
then passed. The release CLI also validated the checked-in project context.
No dependency or lockfile changes were needed. No live provider or additional
Codex trust/dispatch qualification was performed in this follow-up.

The staged UBS static scan completed all modules across **3 Rust files** and
exited 1 with **4 critical, 164 warning and 33 informational matches**. This is not
a clean scanner result. Source review identified the critical matches as two
intentional test failure `panic!` calls and two pseudo-terminal `finish()` calls
misidentified as noncryptographic token generation. The latter wait for a child
and collect its output; they generate no token. Other matches concern fixture
assertions, checked descriptor ownership, bounded reads, literal fixture paths,
and existing CLI output/JSON operations. No detector suppressions were added.
Cargo checks ran separately; UBS used `--no-cargo`. Private logs remain outside Git.

The verified release was archived and copied to the stable installed path using
an atomic replacement, with the previous release retained. The installed binary's
SHA-256 is `d2e2e77f1b9f90d99180445785bd69661984e99b9ed4f81c3f3e5c92fc34016e`.
The installed command validated the project and returned blocker-free Git/Codex
`--dry-run` plans. Existing Git registration and Codex hook configuration bytes
were unchanged; status still reported Git enabled and Codex configured.
