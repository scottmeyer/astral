# Git-native context bootstrap

This is the historical design/recovery record from the first bootstrap milestone.
For present-day work use [project-workflow](../project-workflow/handoff.md) and
the [current subsystem overview](../../core/project-context/README.md). The
default subsystem no longer links this historical projection as its starting point.

## Accepted requirements

Preserve project knowledge across work sessions without repeatedly relearning the
repository. Keep a small `.astral/core/`, named `.astral/projections/`, and
user-defined subsystem directories with rules, decisions, and README context.
Track work in committed JSONL with dependencies and acceptance criteria. Let
Git branches and merges carry the corresponding work state.

The historical command example is `astral project web --work ISSUE-123`.
Named projections are recallable through `astral project NAME --proxy` when native.
Fresh/native launch, bound workers, explicit save, and read-only inspection are
implemented within the [documented limits](../../../docs/worktree-handoff.md).
Historical examples are context, not instructions to execute.

## Implemented contract and remaining extensions

The TOML and JSONL formats are an implemented, experimental version-one contract.
The read-only inspector validates manifests and resolves selected context.
The launcher creates or reuses a bound worktree and trusted destination runtime.
Explicit native exports are Git-trackable; runtime bindings remain private.
External issue adapters and semantic context reconciliation remain future work.

## Recovery evidence at the bootstrap milestone

The requested conversation snapshot was recovered through a supported native
fork, then by normal resume of the original ID after its prior session closed.
Real terminal execution and five recent-discussion checks passed again after a
proxy restart and cold resume of the original.
All native items from its checkpoint onward were accounted for in order, allowing
the installed client's documented-in-source serialization normalization. Its
original canary appears in readable history and is not an opaque-only probe.
Detailed records and runtime identities are private. See
[dated lifecycle verification](../../../docs/native-recovery-lifecycle.md).

## Historical evidence

Earlier AST-000 tests found declaration placement mattered: repetition before
the checkpoint failed; repetition after it restored execution. That is evidence
of ordering sensitivity, not proof of undocumented backend state restoration.
Earlier design discussion reserved AST-001 through AST-006; no historical claim
that those items were implemented is treated as current verification.

## Open questions

Cross-account sharing, broader account/model compatibility, schema evolution,
long-horizon context selection budgets and external work-item adapters remain open.
The accepted interface defaults to direct Codex, opts into
the proxy with `--proxy`, and forwards explicit Codex arguments unchanged.
Private source absence must be reported; do not silently
substitute this readable handoff and call it native recovery.
