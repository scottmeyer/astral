# Worktree binding and native handoff verification

Date: 2026-09-11. Scope: current implementation based on `c4d6a9a`, macOS,
installed Codex 0.154.0, `gpt-6-astra`, built-in OpenAI provider, Responses Lite,
and one account. Earlier recovery/launch receipts are historical evidence; their
passes are not counted as current checks here.

## Local checks

Formatting, strict Clippy, all **297 Rust tests**, locked release builds of all
binaries, and all **18 Python tests** pass. Production worktree, record, capture,
and publication paths are Rust. Existing Python tests continue to cover their
Python experiment/client helpers; Rust process tests use synthetic executables
to exercise the owned Codex adapter without a provider.

Coverage includes dirty-source preservation, one worker per work ID, concurrent
ownership, copied/stale receipts, retained failed creations, incomplete context
copy/staging diagnostics, exact caller arguments, fresh versus cold staging,
ignored/skip-worktree context, disabled Git hooks/fsmonitor, bounded Git processes,
record creation/update/three-way merge, immutable publication, compare-before-replace,
private staging, aggregate limits, and exact completed native capture boundaries.

The full suite initially exposed an inspection regression for valid `.astral`
indexes outside Git repositories. Inspection now retains the context result and
reports an unavailable worktree plan; actual bound launch still requires Git.
A new merge CLI test verifies exit status 1 and explicit conflicts without
modifying inputs. Failed intermediate builds/tests were resolved before the
complete pass above; no missing-module or isolated-worker pass substitutes for it.

The first real Astral-bound review worker found three integration defects before
merge: known pre-thread failures blocked retry, exporting under another name lost
the selected projection's anchor, and direct subsystem lists did not match their
dependency closures. All three were fixed. New process tests cover fresh/native
preflight failures followed by same-work retry, uncertain thread creation without
duplicate starts, and the complete launch/save-`p`/save-`q`/resume-`p` sequence.
The save test exercises the owned app-server, capture, publication and private
metadata together, including failed compaction retry and unrelated seed rejection.
An additional review caught and fixed an absent-export comparison in older receipts.
Synthetic fixture schema/UUID mistakes were corrected before the complete pass.

## Disposable live controls

| Control | Result | Evidence boundary |
| --- | --- | --- |
| Fresh automatic work binding | PASS | Actual terminal invocation, exit 0, exact assigned worktree; worker file edit stayed in that worktree, source file unchanged |
| Repeat fresh work launch | PASS | Same work ID, branch, worktree and Codex thread; explicit single-command `pwd` receipt |
| Explicit native save | PASS | New completed compaction through correction/rebinding mode; 4 native items, 10,624 payload bytes; validated hashes |
| Git transfer to separate clone | PASS | Committed bundle manifest and payload bytes identical after clone |
| Bound native import in destination | PASS | New worker/thread in destination repository, actual `pwd`, exit 0, exact assigned worktree |
| Independent native recall | PASS | Separate import recovered the exact 24-hex-character canary; zero tools; target absent from exported readable bytes and the execution parent's entire readable rollout |
| Cold resume after import | PASS | Same destination thread and worktree, new owned proxy, actual `pwd` result |
| Read-only prohibited fixture write | PASS | Linked native call/result reports exit 1 and permission denial; fixture hash unchanged, no escalation |
| Release-build save after imported work | PASS | Another completed native compaction; new projection preserves the imported bundle's parent digest and keeps the canary opaque |

The initial combined `pwd`/Python tool receipt reported only the Python output.
The verifier refused to infer the directory and ran a separate `pwd` control.
Likewise, the CLI event summary omitted the denied write's command item. A
read-only app-server lookup of the disposable worker's rollout established the
linked native invocation/result. Assistant prose and an unchanged file alone were
not treated as execution evidence. The first audit attempt closed app-server stdin
before a response arrived; sequential request/response handling resolved it.

Native save reads only its owned, stopped worker's pinned local rollout; it never
patches source session records. The capture preserves the exact new native
replacement array, validates both bundle and typed-import compatibility, and
does not reconstruct an old readable tail. Runtime declarations/executors and
permissions are rebound at the destination. No observation-mode switch, patched
client, source authentication read, or original recovery-session intervention was
used. Save may add a normal native compaction to its bound worker before a later
publication failure; the binding conservatively records the rebinding requirement.

Two implementation workers were themselves launched through Astral with the
selected project/work context and bounded assignments. They implemented the
record and worktree modules. Parent integration, independent reviews, current
checks and disposable live tests followed their isolated checks.

## Staged scanner review

UBS 5.4.2 completed its staged static Rust scan across 20 files with no failed
module: **5 critical, 1,565 warning, 720 informational** matches, exit 1. This
is not a clean scan. The five critical matches were two deliberate test panic
branches, two `assume_init` calls guarded by successful `fstatat`, and the Codex
executable selected from trusted current-process configuration. No project
document or native payload selects that executable. No detector suppression or
integrity bypass was added.

Other reviewed findings include bounded allocations/indices, preserved validated
record parsing, generated staging names, test assertions, and a test socket
consumed and dropped while checking connection failure. Independent Cargo checks
ran in the complete checkout; UBS's skipped Cargo/dependency-audit phases are
not reported as passes. Static scanner matches are not counts of confirmed bugs.

The subsequent staged scan of the review fixes covered nine Rust files: **0 critical,
1,045 warning, 417 informational** matches, exit 0, no failed module. Reviewed
matches include test assertions, bounded dependency traversal, JSON indexing into
constructed thread parameters, and the previously reviewed publication paths.
This separate scope does not erase the initial scan's findings or establish a
warning-free scan.

## Limits

These controls establish a working same-account Git handoff and local worktree
binding. They do not establish cross-account teammate portability, other runtime
versions/models/providers, Windows support, arbitrary inherited/rolled-back history,
concurrent non-Astral writers, hostile local-process isolation, power-loss durability,
or successful recovery from every publication/receipt failure.

Publication requires its private Git staging directory and target checkout to
share a filesystem. Failed/incomplete state and prior bundles are retained.
Native histories are not semantically merged, even when Git preserves both
artifacts. Explicit selection chooses one native projection plus current documents.
No deterministic-output, semantic-compaction-fidelity, cache-hit, token-saving or
invoice-saving conclusion follows from these tests.

Private event streams, fixture identities, canary target, captures, result audits,
and tool/build logs remain outside tracked source in the task's private temporary
evidence directory. No credential, transcript, canary plaintext or fixture native
payload is included in this report.
