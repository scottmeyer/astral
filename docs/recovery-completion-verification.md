# Recovery and branch completion verification

Date: 2026-09-11. Local macOS implementation based on
`04a83d816c7f745bdf0df063e55c52e00d1a088d`, developed on
`feat/recovery-completion`. Earlier native lifecycle receipts are historical;
this pass did not contact a provider or recover a live conversation.

## Current checks

All **373 Rust tests** and **18 Python tests** passed, with no ignored Rust tests.
Formatting, strict all-target Clippy, locked release builds of all binaries, and
the Rust **1.85.0** all-target check passed. The Python bridge used the freshly
built release helper; its copied binary was compared byte-for-byte first.

New coverage includes 9 recovery tests, 13 private-binding recovery tests,
8 launch-inventory tests, 16 completion tests, one snapshot-race unit test, and
two process-level staging/save interruption regressions. Real local Git fixtures
exercise commits, fast-forwards and ordinary merges; synthetic app-server
processes exercise acknowledgement loss and shutdown failure.

- Recovery checks exact receipt hashes, ownership, pinned worktree identities,
  context changes, acknowledged staging and exact published bundle evidence.
  Unknown creation/injection outcomes remain blocked without another runtime call.
  Interrupted archive writes retain their partial evidence and permit a retry.
- Inventory checks explicit roots, foreign scope exclusion, malformed and unsafe
  files, ownership, pagination and entry limits. The release CLI observed three
  retained bindings in this repository without contacting Codex.
- Completion checks stale plans, changed refs, clean and stopped workers,
  record conflicts, explicit native choice, artifact retention, ignored-file
  collisions in both file/directory directions, and idempotent completed retries.
  All declared inputs must match committed bytes, including unselected bundles
  and payloads larger than the ordinary 1 MiB Git output limit.
- Valid post-integration work-status updates and unrelated ignored build output
  remain usable. Reverted worker changes require review. Global signature and
  checkout-conversion policies are detected before mutation; hooks and filters
  are not invoked by the integration path.

The release CLI validated the project graph: 36 work records, two projections,
one subsystem and one native artifact. This establishes declared local structure,
not checkpoint decryptability, historical test validity or runtime availability.

Intermediate failures were resolved before the full pass: incomplete module
wiring and compile errors, a Clippy comparison simplification, two fixtures that
tried to create filenames rejected by the macOS filesystem, and a stat-cache
fixture whose timestamp setup was nondeterministic. Review also found lost
context-injection acknowledgements, partial archive publication, ignored-file
overwrite risks and incomplete committed-input checks; regression tests cover
the resulting fixes.

## Scanner review

The required staged UBS static scan covered **26 Rust files** and reported
**1 critical, 1,560 warning and 542 informational** matches, exit 1, with no failed
modules. This is not a clean scan. The critical match is an intentional test-only
panic when an opaque-filename fixture fails for an unexpected reason. The test
accepts only the filesystem's explicit illegal-byte-sequence rejection.

Other reviewed categories include test assertions/parsing, guarded indices,
bounded allocations, fixture path joins, a test connection released by its owner,
and descriptor-based filesystem operations. No introduced production defect was
confirmed from these findings; no suppressions were added. Static scans used
`--no-cargo`; separate Cargo checks ran in the complete checkout. Dependency
audits and remote CI were not run.

## Supported scope and remaining work

[Recovery](worker-recovery.md) repairs local bookkeeping only from retained
evidence. Observation supports Unix; atomic repair evidence publication requires
Linux or macOS. This pass tested macOS. Legacy unproven creation, missing save
publications and lost runtime acknowledgements are reported without inferred
success or automatic duplication.

[Completion](branch-completion.md) supports reviewed fast-forwards and observes
ordinary Git integration. Save, checks, commits and semantic merge decisions stay
explicit. Divergence, tracked deletions, unsupported Git policies and nonexact
post-merge code/context changes require ordinary Git and review. Native histories
are preserved and selected explicitly, never concatenated.

The broader finish/resume milestone remains open for opt-in Git/Codex lifecycle
integration. Semantic reconciliation, live-provider interrupted recovery,
cross-account portability and other operating systems remain unverified here.
Private logs and local binding identifiers remain outside Git; no transcript,
credential or newly captured native payload is included in this receipt.
