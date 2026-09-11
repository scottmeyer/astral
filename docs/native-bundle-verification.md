# Native bundle validation verification

Recorded 2026-09-11 on `feat/native-bundle-contract`, based on
`d3e6eb5b1dd6fcdec18d7ef1b46f148b35ef71a8`. Checks were rerun on this change.
Historical native-recovery and fresh-launch results were not substituted for
current verification. The [bundle contract](native-bundles.md) defines the scope.

## Current checks

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --all-targets --locked -- -D warnings` | PASS |
| `cargo test --locked` | PASS, 186 Rust tests |
| `cargo build --release --locked --bins` | PASS |
| `python3 -m unittest discover -s tests -p 'test_*.py'` | PASS, 18 tests |
| Release `astral context validate` on this repository | PASS, 21 work records, 1 subsystem, 1 readable projection, 0 native artifacts |
| Release `astral project --inspect` | PASS, default readable context, empty native artifact list |
| `git diff --check` | PASS |

The worktree release binary SHA-256 was
`ce192f486daf96ac3112a1cd2ae150137dbc31743020d9f26c97bafed7047a96`.

The first broad Rust run stopped on an older CLI assertion expecting
`UNSUPPORTED_NATIVE_BINDING`. The new bundle contract reports
`INVALID_NATIVE_REFERENCE` for that malformed declaration. The assertion was
updated while preserving its check that Codex never starts. The complete rerun
passed all 186 tests.

## Demonstrated behavior

Eighteen native-item tests verify exact manifest, payload and item bytes;
encrypted strings and escaped characters; unknown fields and large numeric
spellings; required/strict metadata; compatibility declarations; integrity,
truncation and checkpoint counts; duplicate JSON keys; bounded collections;
typed tool pairing and every checkpoint boundary; and native agent messages.
They distinguish source instructions and metadata from executable capabilities.
Error messages, summaries and Debug output do not disclose payload markers.

Fifteen project integration tests verify confined native references, pinned
manifest and payload integrity, missing/corrupt artifacts, project identity,
dependency selection, metadata-only CLI output and source fingerprints. They
cover snapshots retained after disk changes, root-independent identity,
path-sensitive references, separate/aggregate resource caps, and refusal to
launch native state as fresh context. A subprocess sentinel establishes that the
native refusal occurs before Codex is started.

Review found and closed a role-alias gap: a fresh-linked subsystem could point
its README at an unrelated native bundle. Global validation now rejects any
declared readable document whose observed bytes equal a bundle manifest or
payload. Regressions cover both shared paths and exact copies. This protects
declared artifacts; it is not a general classifier for arbitrary document text.

## Supplemental scanner

UBS 5.4.2 completed a final staged scan with `ubs --staged --ci --no-cargo`.
Seven Rust files produced 7 critical, 652 warning and 253 informational heuristic
matches, exit 1. This is a reviewed scan, not a clean scanner pass. Cargo checks
ran separately in the complete checkout.

The critical matches are two intentional test-only panics, the existing
`MaybeUninit::assume_init` guarded by successful `fstatat`, and four test process
invocations whose executable is Cargo's compile-time `CARGO_BIN_EXE_astral` path.
Warnings predominantly concern test assertions/unwraps, validated-map indexing,
bounded cloning/allocation, and fixture path construction. Review found no
introduced production defect in these matches. No findings were suppressed.

Private check logs are retained in
`/private/tmp/astral-native-bundles-verification-20260911`. All native test payloads
are synthetic. No private checkpoint, transcript, credential or account data was
read or committed for this milestone.

## Not established

There were no live-provider native operations in this pass. Tests do not prove
decryptability, complete exporter history, destination account compatibility,
native import/continuity, semantic equivalence, cross-platform loading, or
resource savings. A self-consistent manifest cannot prove that its exporter did
not omit source history. Preserving bytes in the bundle reader does not prove
that Codex's importer preserves every host-owned metadata field.

Capture/export commands, new-thread native staging, destination binding, managed
proxy launch, Git worktree behavior and save/handoff remain later milestones.
