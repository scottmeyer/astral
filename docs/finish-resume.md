# Finish and resume work confidently

The launcher, bound worktrees and explicit native save are implemented. This
milestone connects them into an everyday workflow with visible state, evidence
for recovery, and reviewed branch completion. It does not change what ordinary
Git commands mean or make native histories automatically mergeable.

## Sequence and acceptance

| Work | ID | Outcome |
| --- | --- | --- |
| Finish/resume milestone | `AST-g3krn1a1918m` | A coherent inspect, resume, recover and complete workflow |
| Status and diagnosis | `AST-2c0byax9z620` | Bounded observations of recorded workers, ownership, current context and actionable blockers; no runtime launch or repair |
| Explicit recovery | `AST-shfva8kccrev` | Previewed, identity-checked repairs for incomplete operations; retain uncertain outcomes and artifacts |
| Branch completion | `AST-675y0gng8eb5` | Review code, work records and decisions; preserve divergent bundles; explicitly select or reconcile context before integration |
| Git/Codex integration | `AST-cfqs5ngqcyg2` | Opt-in lifecycle checks shared by hooks and any Astral wrappers, with ordinary Git behavior preserved |

Status, diagnosis and [explicit worker recovery](worker-recovery.md) are implemented.
[Branch completion](branch-completion.md) previews the results and supports an
explicit, checked fast-forward, while recognizing ordinary Git integration.
Hook installation and a potential `astral commit` command remain planned.
The [recovery/completion verification receipt](recovery-completion-verification.md)
records this implementation's current local checks and remaining limits.
A status observation cannot establish that a remote model is
available, a recorded thread still exists, a checkpoint is decryptable, or a
runtime command will execute. Launch retains its current preflights and locks.
The [status verification receipt](status-verification.md) retains its dated local
tests and live owned-worker observation; it does not qualify later workflow stages.

## Current status and doctor commands

```sh
astral status
astral status --work AST-EXAMPLE --json
astral doctor --work AST-EXAMPLE
astral status --offset 32 --limit 32
```

Replace `AST-EXAMPLE` with a record from `astral work list`. Output is human-readable
by default; `--json` emits indented schema-version-one JSON. Status lists the
current checkout's work records, their stored selector, branch/worktree, recorded
thread, current ownership observation, recorded export digest, and selected
document changes since staging. Current context is read from the bound worktree,
which may differ from the invoking checkout. A changed digest means selected
inputs changed, not that earlier reasoning or test results remain correct.
If no staging digest was recorded, JSON reports `changed_since_staging: null`
and the human view labels that comparison unknown.

An available, locally consistent binding includes a suggested continuation argv
array. It is not executed, carries no permission flags and retains `--proxy` when
the recorded native state requires it. The launcher checks the binding and
runtime again when the suggested command is actually invoked. A recorded thread
is labeled `recorded`, not verified live. A held ownership lock is `owned`; wait
for that operation. An abandoned initial staging attempt becomes an attention
item, since automatically starting another thread could duplicate work.

`doctor` uses the same observations and guidance. Exit 0 means no blockers were
found in the requested page, exit 1 means local attention is needed, and exit 2
means invalid arguments or a command-level failure. Status exits 0 for a rendered
report even when it contains diagnostics. Invalid project inputs are included
as diagnostics when the root is accessible. Neither command repairs state.

The default page size is 32 and the maximum is 64. `next_offset` indicates more
records; `--work` selects one and requires offset zero. An empty page does not
mean the repository has no workers. The inventory is scoped to the current work
register: orphan private bindings and launches without `--work` are not listed.
Files and Git plumbing outputs are bounded; each Git subprocess has the existing
30-second deadline. Ownership probes are nonblocking, brief, and released before
return. There is no reservation or atomic snapshot across the whole report.

Observation invokes no Codex process, inference, hook, repository script or
repair. Git plumbing disables optional locks, hooks and fsmonitor. It does not
run tests, inspect authentication or conversation contents, or claim provider
compatibility. Remote thread existence, general code freshness and historical
test validity remain unchecked. Save still requires an explicit stopped worker.

## Git integration direction

Ordinary `git commit`, checkout and merge must remain useful. Build one set of
context checks and invoke it from explicit Astral commands or opt-in lifecycle
hooks. A future wrapper should add a convenient reviewed sequence, while
preserving literal Git arguments, signing, existing hooks and exit behavior.
Installing an integration must not replace another tool's hooks or silently
rewrite `core.hooksPath`. Detect existing configuration and show the proposed
integration first. Guard against invoking hooks recursively from Astral's own
Git operations.

Commit-time checks should examine the staged snapshot, since working-tree files
can differ from what Git will commit. Checkout/merge checks should report how
the selected documents and binding differ after Git's operation. Hooks should
be bounded, offline and advisory by default. They must not unexpectedly start
inference, compact a conversation, stage files, change branches, commit or push.
Any enforcement policy needs explicit configuration and clear recovery behavior.

Native export is an explicit operation against a stopped, owned worker today.
A commit hook must not infer permission to compact an active conversation.
Codex integration must use a supported runtime lifecycle surface when one is
available; the host's subagent tool is not automatically intercepted by Astral.
Exact hook names and event contracts are still design work, not claimed APIs.

Branch completion should produce a reviewable plan before integration. Git can
preserve both branches' immutable native artifacts, but a user or reconciliation
worker must choose the next starting context and reconcile readable decisions.
An interrupted completion must be inspectable and resumable without repeating
commits, overwriting code, or manufacturing combined opaque memory.

The implemented completion command leaves save, editing, checks and commits
explicit. It observes their current Git results on every preview; it does not
replay a list of historical commands after an interruption. Divergent integration
uses ordinary reviewed Git merging, then another completion preview. A reviewed
plan hash is required before Astral applies any fast-forward.

## Broader work register

The JSONL register also tracks long-horizon context freshness/retrieval
(`AST-4mhgqjrtxxdd`), projection retention (`AST-w5z6g1mgm8cn`), cross-account
teammate qualification (`AST-64rmpqdvq263`), delegated-worker coordination
(`AST-9zdzsmzdvxpy`), external issue adapters (`AST-vm5rkdk0w42c`) and sustained
quality/cost evaluation (`AST-b1q2xsxh3qcb`). Runtime/protocol qualification remains
`AST-010`; semantic context reconciliation remains `AST-005`. These are open
work, not benefits inferred from the completed native handoff demonstration.

## Work-register reconciliation

`AST-009` is complete within the documented Unix/Codex native scope. Its obsolete
claim that automatic worktree binding was missing has been removed. `AST-003`
and `AST-004` are also closed against their implemented worktree and native
handoff milestones, with their original acceptance criteria and scope retained.
`AST-002` remains open for its external adapter boundary; its JSONL operations
are implemented. Dependencies now point at implemented work where an unrelated
external integration would otherwise keep the bounded launcher misleadingly open.

Dated verification receipts retain the results and limitations of their runs.
Current usage guides point to implemented behavior and distinguish the planned
workflow above. A status field is not a new test result.
