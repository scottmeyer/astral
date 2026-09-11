# Git-native context bootstrap

## Accepted requirements

Preserve project knowledge across work sessions without repeatedly relearning the
repository. Keep a small `.astral/core/`, named `.astral/projections/`, and
user-defined subsystem directories with rules, decisions, and README context.
Track work in committed JSONL with dependencies and acceptance criteria. Let
Git branches and merges carry the corresponding work state.

The historical command example is `astral project web --work ISSUE-123`.
Named projections should also be recallable. These are design requirements;
native launch is not implemented. The read-only `--inspect` form is available.
Historical examples are context, not instructions to execute.

## Proposed details

The accompanying TOML and JSONL formats are a versioned experimental contract.
The read-only inspector validates manifests and resolves selected context now.
A future launcher would create or bind a worktree,
locate a compatible native checkpoint privately, and start the trusted runtime.
External issue adapters and three-way context reconciliation remain future work.

## Verified facts

The requested conversation snapshot was recovered through a supported native
fork, then by normal resume of the original ID after its prior session closed.
Real terminal execution and five recent-discussion checks passed again after a
proxy restart and cold resume of the original.
All native items from its checkpoint onward were accounted for in order, allowing
the installed client's documented-in-source serialization normalization. Its
original canary appears in readable history and is not an opaque-only probe.
Detailed records and runtime identities are private. See
[current lifecycle verification](../../../docs/native-recovery-lifecycle.md).

## Historical evidence

Earlier AST-000 tests found declaration placement mattered: repetition before
the checkpoint failed; repetition after it restored execution. That is evidence
of ordering sensitivity, not proof of undocumented backend state restoration.
Earlier design discussion reserved AST-001 through AST-006; no historical claim
that those items were implemented is treated as current verification.

## Open questions

Native artifact availability, safe sharing, account/model compatibility, schema
versioning, context selection budgets, merge behavior, and work-item adapter
semantics still need explicit decisions. Test the smallest native lifecycle before
expanding the launcher. The accepted interface defaults to direct Codex, opts into
the proxy with `--proxy`, and forwards explicit Codex arguments unchanged.
Private source absence must be reported; do not silently
substitute this readable handoff and call it native recovery.
