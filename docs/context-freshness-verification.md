# Context freshness verification — 2026-09-11

The first bounded slice of `AST-4mhgqjrtxxdd` is tracked as
`AST-mbj20aw3ra6n`. It was developed from `f2a7c8ff6ea241a40a7f4d73a49aa226b3b37165`
on `feat/context-freshness`. These results record this implementation pass;
rerun relevant checks before treating a later checkout as verified.

Local macOS checks passed: the complete locked Rust suite (514 tests), strict
all-target Clippy, formatting, the locked release build of all binaries, the
Rust 1.85 all-target check, and 18 Python tests. The 11-test freshness suite was
rerun after the final review-publication/output changes and passed.

The focused fixtures cover:

- Explicit acknowledgement, comment preservation, stale code/document previews,
  changed worker context digests, and exclusion of code bodies from model input.
- Shared core and dependency changes; acknowledging a dependency does not itself
  invalidate its dependent's review baseline.
- Missing files, directories, symlinks, unsafe staged modes, hardlinked review
  manifests, cooperating writer locks, and byte/file/input budgets.
- Exact staged, committed and working code. Staging only a new review does not
  bless different staged code, and missing staged files never use working bytes.
- Literal bracket-containing Git paths, more than 2,000 unrelated fixture files,
  backward-compatible opt-out, human/JSON output, explicit apply, `--yes`,
  `--dry-run`, and conflicting command flags.

The shared terminal confirmation parser rejects EOF, partial and oversized
answers; existing hook-confirmation tests also passed in the full suite. Manual
terminal review/cancellation was exercised against this checkout. Repository
dogfood uses nine explicit resolver/lifecycle inputs; it does not claim complete
Astral source coverage.

Staged UBS was run with Cargo execution disabled and its findings reviewed.
It is not a zero-warning scan: the critical heuristic flags the existing
`MaybeUninit::assume_init` after a checked successful `fstatat`; warning categories
include fixture assertions/unwraps, validated indexing and bounded allocations.
Review also hardened the acknowledgement recheck against a subsystem manifest
being moved out of the loaded registry by a concurrent edit.

No live Codex/provider, native checkpoint, cross-account, long-horizon retrieval
or semantic stale-claim evaluation was run for this change. Equal fingerprints
establish unchanged observed bytes relative to an explicit review, not that the
knowledge is correct. There is no cost, token-saving, atomic-workspace or
large-repository latency guarantee. See the [contract and limits](context-freshness.md).
