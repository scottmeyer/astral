# Opt-in Git and Codex lifecycle integration

Git and Codex integrations are independent, opt-in advisory checks. This guide
defines the shared workflow and the implemented first-pass limits. Dated results
are recorded separately in [lifecycle verification](lifecycle-verification.md).

## Commands

```sh
astral lifecycle check
astral lifecycle check --scope index --work WORK_ID
astral hooks status
astral hooks install git
astral hooks install codex
astral commit -- -am 'Update code and context'
astral hooks uninstall git
astral hooks uninstall codex
```

In a terminal, install and uninstall show the repository, command, scope and
planned changes, then ask `Apply this hook setup? [y/N]`. Enter `y` or `yes` to
apply that plan immediately. Enter, `no`, EOF or an incomplete answer makes no
changes. Conflicting hooks are reported before any confirmation prompt. These
are setup commands; commits and sessions do not require repeating them.

For scripts or captured shell commands:

```sh
astral hooks install git --yes              # apply without prompting
astral hooks install codex --yes
astral hooks install git --dry-run          # human-readable preview only
astral hooks uninstall codex --dry-run --json # machine-readable preview only
astral hooks install git --apply PLAN_SHA256 # retain an explicitly reviewed plan
```

Without terminal stdin and stderr, the default command remains a read-only
preview and prints a hint to use `--yes`; piped input does not count as
confirmation. `--dry-run` always previews. Plans and results use concise human
summaries by default; add `--json` for indented machine-readable output. In JSON
mode, install and uninstall preview without prompting unless `--yes` or an
explicit `--apply` hash is supplied. `--dry-run`, `--yes` and `--apply` are mutually
exclusive. `--yes` skips the prompt, while all existing hook ownership, conflict
and stale-plan checks still run. Codex runtime trust remains separate.

`lifecycle check` and `hooks status` also use readable summaries by default;
request `--json` when consuming their structured reports. Hook callback output
retains its Git/Codex protocol and is unaffected by this display choice.

With optional subsystem [freshness declarations](context-freshness.md), checks
also compare code and knowledge against explicit review baselines in each view.
Unreviewed, changed or unavailable inputs produce advisory diagnostics. Review
acknowledgement remains explicit; hooks do not update its fingerprint.

The plan hash identifies its exact repository, target, executable path and
observed hook/configuration state. Astral retains it internally while you review
the prompt and checks it again before writing. If files change during review,
the operation fails with `HOOK_PLAN_CHANGED`; it does not silently apply a new
plan. You only need to handle `plan_sha256` yourself when deliberately separating
preview and apply across processes. Each Git/Codex opt-in has its own plan.

Use an installed, stable Astral path for Git shims. Build development versions
elsewhere, then promote a verified copy atomically between active worker sessions.
Moving the installed executable requires reviewing the retained shims and registration.
Stop other configuration editors during apply: Astral serializes its own writes,
rechecks file identity and bytes, and publishes atomically, but cannot reserve a
file against a noncooperating editor between that check and publication.

For an existing Git hook manager, add `astral hook git EVENT --manual` at the
appropriate event using that manager's normal configuration. Keep its existing
commands and exit policy. Astral's handler always returns success and writes only
an advisory to stderr. The automatic installer does not wrap foreign scripts or
edit `core.hooksPath`.

For Codex, inspect the resulting `.codex/hooks.json`, deliberately commit it if
it should travel with the branch, and review it through Codex's `/hooks` trust
workflow. The destination must resolve `astral` on its PATH. Astral does not enable
experimental features, change global configuration, or approve hook trust.
`hooks status` reports current configuration separately from private registration;
it cannot attest that Codex loaded or trusted it. Committed hook configuration can
be present on a clone with no private installation record.

## Ownership and defaults

Git owns its index, commits, refs, signing and existing hooks. Codex owns its
session, tools, permissions and hook trust. Astral owns its explicit worker/save
receipts and the shared context checks. Hook events are notifications to observe
current evidence, never acknowledgements of a completed Astral operation.

The initial hooks are advisory and offline. They do not invoke inference, run
tests, compact/export context, stage files, commit, merge, change branches, mark
work complete or repair a receipt. Explicit `save`, `recover` and `finish` retain
their existing ownership and review requirements. No hook blocks a commit or
causes another model turn. Strict policy enforcement is a separate future choice.

## Which surface handles which event

| Surface/event | Evidence and response |
| --- | --- |
| Git `pre-commit`, `pre-merge-commit` | Validate declared context from Git's exact candidate index, including a temporary/alternate index; report invalid or incomplete staged context |
| Git `post-commit` | Inspect the resulting commit and current checkout; a prior pre-commit observation is not proof of this commit |
| Git `post-checkout`, `post-merge`, `post-rewrite` | Resample current branch, HEAD and context; event flags or old/new SHA arguments do not establish success or completion |
| Codex `SessionStart` (`startup`, `resume`, `compact`) | Give the session a bounded current context/binding advisory |
| Codex `UserPromptSubmit` | Recheck relevant context drift before the next user turn, including Git steps done outside Codex |
| Codex `Stop`, `PostCompact` | UI-only observation; stopping a turn is not closing a session, and compaction is not an exported projection |
| `astral lifecycle check` | The same explicit checks, available even when hooks are missing, disabled or skipped |
| `astral commit` | Forward to ordinary `git commit` with literal arguments, stdio, configuration and exit behavior; installed Git hooks supply candidate checks |

Git supports separate pre-merge and pre-commit events. Squash merge may change
the index without creating a commit. Checkout can restore files without changing
branches; rewrite maps can be many-to-one. Hooks can be skipped with ordinary Git
options, absent during earlier work, or lost on interruption. These cases are
normal inputs to reconciliation, not reasons to replay commands.

## Both integrations, ordering and reconciliation

There is one checker with explicit `index`, `head` and `worktree` scopes. Index
checks read Git objects, never substitute working files, and honor Git's actual
index environment. A valid working document cannot hide a broken staged version.
The report identifies its scope and changed/unknown evidence; it never certifies
tests or semantic context freshness merely because hashes match.

Git notifications go to the terminal. Codex notifications go to that session's
supported context/UI channel. Duplicate delivery is harmless: advisory observation
state is separate from worker receipts, scoped to repository, worktree and output
audience. Any notification cache is a best-effort reduction in repeated output,
not a completion ledger or an exactly-once guarantee. Both integrations may run
concurrently. No event can move a worker receipt backwards or clear an uncertain
operation. Internal Astral Git probes disable hooks, and callbacks guard recursion.
Each selected `SessionStart` emits a fresh advisory after startup, resume or
compaction, even when the digest is unchanged. The following prompt can suppress
the repeated observation. Concurrent starts may repeat an advisory; delivery is
not treated as proof that the model retained it.

| User action or interruption | Reconciliation |
| --- | --- |
| Plain `git commit`, including `-a`, `--only`, amend or alternate index | Check the actual candidate when invoked; later inspect the resulting commit regardless of which command initiated it |
| Commit before saving | Code can be committed; no native handoff is inferred. A later explicit save produces a separate reviewed change |
| Save before commit, or save after merge | An export remains a working-tree change until committed; validate its present reference and bytes, retaining previous bundles |
| Manual merge/rebase/cherry-pick | Observe current refs and declarations; preserve native artifact source provenance; use explicit finish/context review where required |
| Branch changes while a Codex session remains open | Report binding/context mismatch; never silently associate the old thread with a new work ID or history |
| Git and Codex emit the same observation | Keep audience-scoped notices bounded; do not perform any additional mutation |
| Missing, duplicate or delayed event | Re-read current state. Event order and timestamps never manufacture completed stages |
| Pending save/staging or unavailable ownership | Report the retained state; a busy worker is expected during its own callbacks, and callbacks never acquire repair ownership |
| Partial hook installation or third-party hook edits | Report incomplete/conflicting registration; preserve all files and require a fresh reviewed plan |

## Installation, coexistence and removal

Installation is explicit and previewed before applying the exact retained plan. Git
and Codex are independent opt-ins. Installing support does not enable a proxy,
alter permissions, change Codex trust or replace another integration.

The Git automatic installer supports absent default hook files. It detects
effective `core.hooksPath` and existing foreign hooks, including non-executable
files and links, and provides manual composition guidance instead of replacing
them. Default hooks are shared across linked worktrees; the preview states that
repository-wide scope. Shims use the reviewed absolute Astral executable and a
private registration which becomes enabled only after installation is complete.
Disabling registration retains the shims and evidence. Changed owned files are
diagnostics, never silently overwritten.

Codex installation adds only Astral matcher groups to project `.codex/hooks.json`,
preserving other groups and retaining the prior bytes before replacement. It
does not edit user/global configuration, `notify`, feature overrides or trust.
Removal removes only unchanged owned groups, preserving all other configuration
and backups. Project hook configuration can be reviewed and committed deliberately;
the destination must have Astral installed and review the hooks in Codex.

Codex loads matching handlers from multiple sources concurrently. It does not
replace lower-precedence hooks with a project hook. The adapter therefore makes
no ordering assumptions about other integrations, and never wraps their commands.
Missing executables, timeouts or unavailable observations produce advisory failure,
not a changed Git result or a blocking Codex decision.

## Runtime and data boundary

Target the installed Codex 0.154.0 schemas. `SessionStart` and `UserPromptSubmit`
support additional context; `Stop` and `PostCompact` do not in that runtime and
must use UI-only output. Never emit `decision: block`, exit 2, or `continue: false`
from an advisory Codex hook. A turn stop must not create a continuation loop.

Read only bounded event metadata. Discard prompt/assistant text and tool bodies;
never follow `transcript_path`, read authentication or treat `permission_mode`
as a sandbox attestation. Session IDs alone do not establish a worker binding,
and subagent events are not automatic permission to adopt or launch a worker.
Checks and callbacks have explicit byte, entry and time budgets.

The command callbacks currently run on Unix. Codex stdin is capped at 64 KiB with
a 750 ms read deadline. The complete observation subprocess has a 3 second
deadline; timeout kills its process group, including its Git probes. Installed
Codex commands also declare a 5 second runtime timeout. Large or slow repositories
may receive an unavailable advisory and can use explicit `lifecycle check` for a
fuller bounded observation. No notification failure changes worker receipts.

Snapshot enumeration is limited to 16,384 `.astral` entries and 4 MiB of metadata.
Declared blobs load lazily through the existing project reader budgets, retaining
exact native bytes without decoding them. Index parsing supports the tested Git
`ls-files --debug` layout; unknown layouts are an unavailable observation. Unsafe
file types, unresolved conflicts and redirected repository identity are rejected.
Split indexes are explicitly unsupported because Git can refresh their shared
index during reads; the adapter rejects them before those Git probes. Index
format inspection is capped at 32 MiB and supports the tested versions 2–4.
Working context is observed per file, not as an atomic filesystem snapshot.

The notice cache keeps at most 64 hash-only entries in private Git metadata. It
does not store prompts, document contents, transcripts or event bodies. Cache
failures permit another advisory. Subagent events are ignored because their root
session ID alone cannot identify an Astral worker; delegated-worker integration
remains separate work.

Sources: [official Codex hooks](https://learn.chatgpt.com/docs/hooks), installed
Codex 0.154.0 hook schemas and dispatch code, and the local Git 2.51.0 `githooks`
manual. The current docs may describe features newer than the installed runtime.
Operational tests must distinguish matching schema/fixture checks from actual
runtime invocation and live-provider continuity.
