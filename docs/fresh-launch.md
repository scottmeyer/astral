# Fresh project launch and initialization

`astral project` now opens a fresh Codex session seeded with the selected core,
subsystem/dependency documents and optional work record. It defaults to
`project-context`. An explicit readable projection also includes its handoff.
This is document context, not native checkpoint restoration.

```sh
astral project
astral project web -- --model gpt-6-astra "Work on the selected subsystem"
astral project --work AST-33ydmjdkmx4k -- --sandbox read-only
astral project --inspect
```

The current checkout is used; `--work` adds the selected record without creating
a branch or worktree. Each launch creates a new local thread. The printed thread
ID can later be resumed directly with Codex. Launch does not commit files or
advance a saved projection.

## Initialize a repository

When `.astral/` is absent, interactive `astral project` offers initialization.
Accepting starts the embedded initialization task; afterward rerun the original
project selection. Noninteractive callers receive an explicit initialization
instruction instead of a hidden question.

```sh
astral init --inspect
astral init
astral init --non-interactive -- --model gpt-6-astra --sandbox workspace-write -c 'approval_policy="never"'
```

`init` supplies its own versioned prompt. Pass Codex options, not a subcommand or
an additional prompt. `--non-interactive` selects `codex exec` and closes stdin;
otherwise it opens Codex interactively. Use options accepted by that Codex mode:
for example, `exec` takes approval policy through `-c`, while interactive Codex
also accepts `--ask-for-approval`.

The prompt asks for a bounded, best-effort scan and a small schema-v1 index. It
preserves existing work, creates `.astral/` exclusively, and labels observed
commands and test configuration as unverified. It does not ask the model to run
project scripts or tests. Prompt instructions guide inference; Codex's current
sandbox remains the enforcement boundary. After Codex exits successfully,
Astral validates the resulting index and default fresh context. Invalid or
partial generated files remain available for review. Existing `.astral` entries,
including symlinks and malformed indexes, never trigger automatic reinitialization.

## Runtime boundary

Astral owns a bounded JSON-RPC adapter to the installed Codex app-server. It starts
a new durable thread, inserts one user-role message with selected document text,
waits for persistence acknowledgment and clean app-server shutdown, then invokes
`codex resume THREAD_ID` with the original caller argument vector as its suffix.
It does not replace base/developer instructions or read/patch session files.
The [app-server API](https://learn.chatgpt.com/docs/app-server#api-overview)
supports history-only `thread/inject_items`. Staging sends no `turn/start`, but
Codex startup can initialize tools, authentication and network prewarming.

Documents come from the same bounded file observations as their source hashes.
They are not reread after resolution, and this is not an atomic repository
snapshot. Fresh selection fingerprints also include linked projection manifests
used to reject native or unknown kinds. Allowed readable kinds are
`fresh-context`, `reviewable-design-context`, and `reviewable`. Unknown native
references, missing declared files, or `native_payload_in_repository=true` fail;
there is no plaintext fallback for a native checkpoint.

Codex owns authentication, execution and permission enforcement. Explicit model,
config, feature, approval and sandbox choices are reflected during staging;
their original arguments are forwarded unchanged to the interactive process.
No permission bypass is added by default. `ASTRAL_CODEX_BIN` may select a trusted
installed executable; repository manifests cannot select it or supply flags.

This first implementation uses the Unix confined reader and is live-tested with
Codex 0.154.0 on macOS. Fresh staging rejects `--last`, remote runtimes, managed
worktrees, profile-v2/OSS selection, extra writable directories and conflicting
workspace/store overrides. Standalone app-server cannot reproduce all those TUI
settings yet. Matching `--cd` is accepted; choose a different checkout through
Astral's `--root`. Direct initialization can forward profiles, OSS selection and
extra writable directories because it does not stage a thread.

Native bundles, managed `--proxy` launch, automatic branch/worktree binding and
save/handoff remain the subsequent [milestones](launch-plan.md). Missing native
support is an explicit error, not a successful launch claim.

See the dated [verification receipt](fresh-launch-verification.md) for local and
live-runtime results, scanner findings and untested boundaries.
