# Astral

![Astral — luminous branching paths across a celestial horizon](docs/assets/astral-banner.png)

**Agent context that follows your code.**

Astral keeps project knowledge and saved agent context alongside your code in
Git. Define the subsystems that matter, launch Codex with the right context, and
return to the same work later. A work item can bind its own branch, worktree and
conversation; an explicit save creates a portable starting point for the next
session.

[Get started](#get-started) · [Project layout](#context-lives-in-your-repository) ·
[Current scope](#current-scope) · [Save and resume](#save-a-starting-point) · [Guides](#guides)

## What you can do

- **Start with relevant context.** Load shared architecture, subsystem rules,
  decisions and the selected work item.
- **Keep work together.** Reopen a bound branch, worktree and Codex thread with
  the same command.
- **Save a continuation.** Commit an exported native checkpoint and its readable
  handoff, then launch it from a compatible checkout.
- **Use your normal Git workflow.** Optional Git and Codex hooks report context
  drift. Ordinary commits and merges remain supported.

Astral also includes a standalone Responses proxy for rolling native context
projections and an optional host for compact tool observations, exact artifact
retrieval and verification tied to file versions.

## Current scope

The project workflow is implemented, with an experimental version-one context
format and a deliberately narrow runtime contract. These are separate paths:

| Path | What works today | Boundary |
| --- | --- | --- |
| Inspect or launch documents | Validate named contexts; initialize an index; start a fresh Codex thread | Inspection is offline. Fresh unbound launch uses the installed app-server API; tested on macOS, without a general runtime compatibility guarantee |
| Bound work: `--work ID` | Dedicated branch/worktree, reusable thread, status and diagnostics | **Codex 0.154.0 is required even for document-only workers** |
| Native save/import | Explicit export, Git transfer, new-thread import and cold resume | Codex 0.154.0, `gpt-6-astra`, built-in OpenAI provider, same account, explicit `--proxy` |
| Recovery and completion | Evidence-based repairs and reviewed fast-forward integration; recognize ordinary Git work | Save, tests, commits and divergent merges remain explicit |
| Git/Codex hooks | Per-repository Git setup and per-checkout Codex configuration; offline context advice | Opt-in and advisory; Codex trust is separate; no automatic save or global Codex hook installer |

Project filesystem operations use Unix confinement. The standalone Responses
proxy is a separate component; its rolling and cache policies do not qualify
additional Codex versions or native checkpoint formats.

Remaining work includes cross-account handoff, broader runtime qualification,
external issue adapters, long-horizon retrieval/freshness, retention cleanup,
semantic context reconciliation and automatic delegated-worker orchestration.
See the [current project context](.astral/core/project-context/README.md) and
[work register](.astral/work/items.jsonl). Historical test receipts and completed
work records describe their recorded scope; they do not verify today's checkout.

## Get started

Build from source with Rust **1.85 or later**, and have Git and Codex installed.
Use Codex **0.154.0** for the bound-worker and native examples below. An unbound
fresh launch does not enforce that exact-version gate; its app-server API must
still support the documented staging operations.

```sh
git clone https://github.com/scottmeyer/astral.git
cd astral
cargo install --path . --locked --bin astral

# In the repository you want to work on:
cd /path/to/your/project
astral init
astral context list
astral project --inspect
astral project
```

`astral init` asks Codex to scan the repository and build a best-effort
`.astral/` index. Review the generated documents and commit the context you want
workers to inherit. If an index already exists, start with `context list`.
Omitting the context name selects `project-context`.

Fresh document contexts launch directly through Codex. The proxy is opt-in.
Codex supplies authentication, executable tools and permission enforcement.

For Astral development, keep the installed executable separate from
`target/release`. Git hooks should reference a stable installed path; promote
verified builds between worker sessions. See [hook setup](docs/lifecycle-integration.md#commands).

## Context lives in your repository

```text
.astral/
├── project.toml
├── core/
│   ├── ARCHITECTURE.md
│   ├── RUN.md
│   ├── TEST.md
│   ├── project-context/
│   │   ├── subsystem.toml
│   │   └── README.md
│   └── web/
│       ├── subsystem.toml
│       ├── README.md
│       ├── rules/
│       └── decisions/
├── projections/
│   └── authentication-review/
│       ├── projection.toml
│       ├── handoff.md
│       └── bundles/
└── work/
    └── items.jsonl
```

Subsystems are user-defined and registered in `project.toml`. Their manifests
select documents and explicit dependencies. Selecting `web` loads its declared
context plus shared core; it does not inject the entire repository or every
saved conversation.

This layout is illustrative. Astral's own index currently defines the
`project-context` subsystem and three projections: the current readable
`project-workflow`, the historical `git-native-context-bootstrap`, and the saved
native `worktree-handoff-review`. Use `astral context list` for the actual
selections in your repository. A new project does not automatically contain `web`.

Readable documents, work records and explicit exports travel through Git.
Credentials, local thread bindings and proxy state stay outside tracked context.
See the [manifest and selection contract](docs/project-context.md#manifest-and-selection-contract).

The old `.ostk-gpt/` name is still the standalone proxy's default state directory;
it is not `.astral/` project context. `OSTK_GPT_UPSTREAM`, `x-ostk-*` headers and the
internal `ostk-gpt-cache` crate also remain compatibility names. Managed project
proxies use private binding/receipt paths instead. The
[names and storage reference](.astral/core/project-context/README.md#names-and-storage)
distinguishes these locations. Old names inside an exported native window remain
historical; editable handoffs and selected current documents provide updated guidance.

## Start a task, then pick it up again

After defining a `web` subsystem:

```sh
astral work create 'Review web authentication' \
  --acceptance 'Record findings and relevant checks'

# Replace AST-EXAMPLE with the returned work ID.
astral project web --work AST-EXAMPLE -- --model gpt-6-astra

# Later, repeat the same command to reopen that worker.
astral project web --work AST-EXAMPLE -- --model gpt-6-astra

astral status
astral doctor --work AST-EXAMPLE
```

The first launch creates `astral/WORK_ID` and a dedicated worktree from committed
HEAD. Subsequent launches preserve its edits and resume its thread. Commit
selected context changes before first binding; the work-record file is carried
automatically. Work IDs are random, avoiding competing branch counters.

Resumes read context from the bound worktree. Updating documents on `main` does
not update other worker branches automatically; bring the desired Git changes
into the worker's branch first. Its next resume appends changed selected documents.

Without `--work`, repeating `astral project web` starts another fresh thread in
the current checkout. Use a work ID when you want Astral to retain continuation
identity. `status` reports the bound worktree path for review and commits.

Put Codex arguments after `--` to forward them literally:

```sh
astral project web -- --sandbox read-only "Review the subsystem"
astral project web --work AST-EXAMPLE --non-interactive -- \
  --json "Continue the review and report concrete findings."
```

[Bound workers and work records](docs/worktree-handoff.md) explains ownership,
continuation, source-edit preservation and record-aware merging.

## Save a starting point

Native save and launch currently support **Codex 0.154.0**, **gpt-6-astra**, the
built-in **OpenAI provider**, and the **same account**. Start the worker with that
model if you intend to export its native context.

Close the worker, then explicitly export its native context:

```sh
astral save authentication-review --context web --work AST-EXAMPLE --proxy
```

After saving, resume the **same worker** with the original selector and work ID,
adding `--proxy` because its thread now contains native context:

```sh
astral project web --work AST-EXAMPLE --proxy -- --model gpt-6-astra
```

The export is published under `.astral/projections/` in the **worker's checkout**.
Review and commit the code, work records, handoff and bundle there. Once that
commit is available in another checkout:

```sh
astral project projection:authentication-review --proxy
```

Update the readable handoff yourself: save creates a generic handoff for a new
projection and preserves an existing one; it does not generate a fresh task summary.

Astral manages the proxy for this route and rebinds the destination's current tool
declarations after the checkpoint. Cross-account teammate handoff and other
runtimes remain unproven.

For native launches without `--work`, use the printed private launch ID with
`--resume LAUNCH_ID`. That is separate from bound-worker continuation; do not
combine `--resume` and `--work`.

Git can retain multiple native bundles; it cannot semantically merge encrypted
conversations. Reconcile readable decisions and explicitly choose the next
projection. Saving and committing are separate operations.

See [native launch](docs/native-launch.md) and [save and handoff](docs/worktree-handoff.md#save-and-hand-off).

## Fit into the workflow you already use

```sh
astral hooks install git
astral hooks install codex
astral lifecycle check
astral commit -- -am 'Update code and context'
```

Hook setup shows a plan and asks for confirmation in a terminal. `--yes` supports
automation; `--dry-run` prints a formatted JSON preview. Git and Codex are
independent opt-ins, and Codex runtime trust is managed separately.

Hooks give bounded offline advice about context drift and retained work. A
commit does not save a native checkpoint or verify tests. `astral commit`
forwards to Git with its ordinary hooks and signing.

Interrupted work and completed branches have explicit review steps:

```sh
astral recover --work AST-EXAMPLE
astral finish --work AST-EXAMPLE --into main
```

These commands preview a plan. Applying recovery or a supported fast-forward
still requires `--apply PLAN_SHA256`. Ordinary Git merges are recognized on the
next review. See [recovery](docs/worker-recovery.md),
[branch completion](docs/branch-completion.md) and [lifecycle integration](docs/lifecycle-integration.md).

## Responses proxy and working-state host

`astral proxy` maps a client's full original history to a persisted native
projection plus its recent tail. The projection stays fixed between rolls.
Standalone compaction uses the complete native `/responses/compact` output;
the optional inline backend adopts a completed provider checkpoint. Response
streams are forwarded unchanged.

```sh
# Authentication may come from the client or OPENAI_API_KEY.
astral proxy
```

The default endpoint is `http://127.0.0.1:8088/v1/responses`.
The [proxy guide](docs/responses-proxy.md) covers authentication, the full-history
client contract, roll thresholds, cache policy, persistence and accounting.
The [working-state host](docs/working-state.md) adds compact observations,
exact artifact recall and checks bound to declared file versions.

Historical evaluations found tradeoffs: a
[two-seed inline trial](docs/benefit-evaluation.md) reduced input tokens by
**69.4%** versus full history while increasing wall time by **22.1%**.
Provider-managed compaction used fewer input tokens than Astral. A separate
[coding trial](docs/working-evaluation.md) reduced tool-observation bytes by
**93.6%** and passed all six tasks. These small experiments do not establish
invoice savings, lossless compaction or a general performance advantage.

## Guides

| Goal | Guide |
| --- | --- |
| Initialize a project and launch from documents | [Fresh launch](docs/fresh-launch.md) |
| Define subsystems and inspect selections | [Project contexts](docs/project-context.md) |
| Bind worktrees, resume work and export a handoff | [Workers and handoff](docs/worktree-handoff.md) |
| Inspect worker state and diagnose blockers | [Status and doctor](docs/finish-resume.md) |
| Understand native formats and compatibility | [Bundles](docs/native-bundles.md) · [Native launch](docs/native-launch.md) |
| Integrate Git and Codex lifecycle hooks | [Lifecycle integration](docs/lifecycle-integration.md) |
| Operate the proxy or explicit host | [Proxy](docs/responses-proxy.md) · [Working state](docs/working-state.md) |
| Review dated verification and open boundaries | [Context audit](docs/context-refresh-verification.md) · [Lifecycle receipt](docs/lifecycle-verification.md) · [Native launch receipt](docs/native-launch-verification.md) · [Milestones](docs/launch-plan.md) |

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked --bins
cargo +1.85.0 check --locked --all-targets
python3 -m unittest discover -s tests -p 'test_*.py'
```

The automated suite uses local process, Git, filesystem and mock-provider
fixtures. Live-provider trials are separate and explicitly invoked. Build the
release helper before running the Python suite. See
[development checks](docs/development-checks.md) for scanner setup and review
requirements.

Licensed under [MIT](LICENSE).
