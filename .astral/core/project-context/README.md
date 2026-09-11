# Project context

This is the current document-based starting context for maintaining Astral.
Core knowledge stays small; manifests select subsystem documents, explicit
dependencies and named projections. JSONL work records carry status, dependencies
and acceptance criteria alongside code. Inspect the current checkout before
relying on a historical conversation or test receipt.

## Implemented workflow

- `init` asks Codex to build a best-effort index; `context` and `project --inspect`
  validate and inspect declared inputs without launching a worker.
- `project` launches selected documents or a supported native checkpoint.
  `--work ID` creates or reopens a branch, worktree and reusable thread.
- `work create`, `update` and `merge` implement random IDs, optimistic status
  updates and three-way record merging. External issue adapters are separate work.
- `save` explicitly compacts a stopped bound worker and publishes an immutable
  bundle. Named projections can advance while retaining older bundles.
- `status` and `doctor` inspect local state. `recover` applies reviewed repairs
  backed by recorded acknowledgements; uncertain outcomes stay visible.
- `finish` previews committed branch results and supports reviewed fast-forwards.
  Ordinary Git commits/merges remain valid workflow steps.
- Git and Codex hooks provide opt-in offline advice. Setup confirms a plan in a
  terminal; `--yes` and `--dry-run` support scripts. `astral commit` forwards to Git.

CLI output is readable and actionable by default. Use global `--json` for
machine-readable reports, help/version or errors; scripts must request it
explicitly. For `project` and `init`, only arguments before the first literal
`--` belong to Astral. A later `--json` is forwarded unchanged to Codex. Proxy
logs, helper JSONL and hook callbacks keep their protocol formats.

Model-catalog forwarding, module reorganization and the initial finish/resume
milestones are complete. Read [RUN](../RUN.md), [TEST](../TEST.md) and the
[workflow guide](../../../docs/finish-resume.md) for commands and limits.

## Runtime and continuation boundaries

Inspection uses Unix confined reads. All bound workers currently require Codex
0.154.0. Native import/save additionally require the supported same-account
OpenAI / `gpt-6-astra` route and explicit `--proxy`. Unbound fresh launch uses the
installed app-server API without that exact-version gate; broader compatibility
is not inferred from its absence.

Without `--work`, fresh launch starts a new thread in the current checkout.
Repeat the same selector and `--work ID` to continue a bound worker. After native
save, that worker needs `--proxy` even if its original selection was document-only.
`--resume LAUNCH_ID` is for unbound native launch receipts, not bound work IDs.

The destination supplies authentication, executors and permissions. Current
documents accompany native import; changed selected documents are appended on
bound resume. Neither operation erases old claims from a checkpoint or verifies
tests. Hooks do not save, commit, repair or start inference automatically.
Bound resume reads its own checkout: newer context on `main` must be brought
into that worker branch through Git before the worker can observe it.

## Names and storage

| Name/path | Current meaning |
| --- | --- |
| `astral` | Unified user-facing executable |
| `.astral/` | Git-tracked manifests, selected documents, work records and explicit exports |
| Git common directory: `astral/work-bindings/` | Private bound-worker receipts, ownership and proxy state |
| `~/.local/state/astral/launches/` | Default private unbound native launch receipts; override with `ASTRAL_LAUNCH_STATE_DIR` |
| Git common directory: `astral-hooks/` | Private hook registration and advisory metadata |
| `.codex/hooks.json` | Opt-in configuration for this checkout; Codex manages runtime trust |
| `.astral-runtime/` | Default standalone proxy state directory; not the portable context tree |
| `ASTRAL_UPSTREAM`, `x-astral-*` | Standalone proxy environment and wire identifiers |
| `astral` | Cargo package and Rust crate name as well as the executable |
| `astral:` | Generated prompt-cache key prefix |

Use `--state-dir /absolute/private/existing-state` to keep an existing standalone
proxy state directory; Astral does not move or delete previous private data.
Update callers to the current environment and header names; earlier names are
not aliases. Managed project proxies supply their own private state paths. See
the [proxy guide](../../../docs/responses-proxy.md) for the actual client contract.

## Current and historical projections

The subsystem links [project-workflow](../../projections/project-workflow/handoff.md),
a current readable projection with no native payload. The old
[bootstrap](../../projections/git-native-context-bootstrap/handoff.md) and
[native worktree review](../../projections/worktree-handoff-review/handoff.md)
remain available for explicit selection. Old names, completed tasks and dated
results inside native bundles are historical data; bundle bytes remain immutable.

The early [bootstrap](decisions/0001-bootstrap-scope.md) and
[read-only milestone](decisions/0002-read-only-resolution.md) decisions are retained
as history, outside the default decision selection. The current
[interface](decisions/0003-launcher-interface.md),
[work-ID policy](decisions/work-identifiers.md) and
[native portability contract](decisions/portable-native-launch.md) remain selected.

## Remaining work

The [work register](../../work/items.jsonl) tracks broader runtime/protocol and
cross-account qualification, long-horizon retrieval and freshness, artifact
retention, external issue synchronization, semantic reconciliation, delegated
worker orchestration and sustained quality/cost measurements. The initial
implementation does not establish these capabilities or general savings.
Use `astral work list` to observe current statuses; a completed record is not a
fresh verification result.
