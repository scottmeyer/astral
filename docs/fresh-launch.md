# Fresh project launch and initialization

Selecting a readable context with `astral project` seeds Codex with the selected
core and subsystem/dependency documents. The default selector is `project-context`.
An explicit readable projection also includes its handoff. This is document
context, not native checkpoint restoration; selections containing a native bundle
use the separate [native launch path](native-launch.md).

```sh
astral project
astral project web -- --model gpt-6-astra "Work on the selected subsystem"
astral project --work AST-33ydmjdkmx4k -- --sandbox read-only
astral project --inspect
```

Inspection previews use readable summaries by default. Add `--json` for
structured previews and Astral command errors. For `project` and `init`, the flag
belongs to Astral before the first literal `--`, and to Codex after it; for
example, `astral project --inspect --json -- --json` previews forwarding Codex's
JSON event option. Launch progress on stderr and Codex's own process streams
retain their existing formats; `--json` does not wrap them.

Without `--work`, a fresh launch creates a new local thread in the current
checkout. The printed thread ID can later be resumed directly with Codex.
`--work ID` includes the selected record and creates or reopens a bound branch,
worktree and reusable thread. Repeating the command continues that worker and
appends selected documents when their fingerprint changes. Commit context changes
before first binding; the configured work-record file is carried automatically.
Bound workers require exactly Codex 0.154.0, including fresh document workers.
Unbound fresh staging does not enforce that exact-version gate; it still depends
on the installed app-server API and has only the recorded runtime qualification.
See [bound workers and handoff](worktree-handoff.md) for source-edit preservation
and retry behavior. Inspection creates no binding. Launch does not commit files
or advance a saved projection; [save](worktree-handoff.md#save-and-hand-off) is explicit.

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

Astral owns a bounded JSON-RPC adapter to the installed Codex app-server. For a
new fresh thread, it inserts one user-role message with selected document text,
waits for persistence acknowledgment and clean app-server shutdown, then invokes
`codex resume THREAD_ID` with the original caller argument vector as its suffix.
A bound worker resumes its recorded thread, updating current document context
when needed. Headless launch uses `codex exec resume THREAD_ID` and closes stdin.
Fresh staging does not replace base/developer instructions or read/patch session
files.
The [app-server API](https://learn.chatgpt.com/docs/app-server#api-overview)
supports history-only `thread/inject_items`. Staging sends no `turn/start`, but
Codex startup can initialize tools, authentication and network prewarming.

Documents come from the same bounded file observations as their source hashes.
They are not reread after resolution, and this is not an atomic repository
snapshot. Fresh selection fingerprints also include linked projection manifests
used to reject native or unknown kinds. Allowed readable kinds are
`fresh-context`, `reviewable-design-context`, and `reviewable`. Explicit native
bundles can be [validated and inspected](native-bundles.md), then launched through
the separate [native launch path](native-launch.md) with explicit `--proxy`. Unknown native references and
missing declared files fail; there is no plaintext fallback for a native checkpoint.

Codex owns authentication, execution and permission enforcement. Explicit model,
config, feature, approval and sandbox choices are reflected during staging;
their original arguments are forwarded unchanged to the launched Codex process.
No permission bypass is added by default. `ASTRAL_CODEX_BIN` may select a trusted
installed executable; repository manifests cannot select it or supply flags.

The implementation uses the Unix confined reader; the dated verification receipts
record controls with Codex 0.154.0 on macOS. Fresh staging rejects `--last`, remote
runtimes, Codex-managed worktree options, profile-v2/OSS selection, extra writable
directories and conflicting workspace/store overrides. Standalone app-server
cannot reproduce all those TUI
settings yet. Matching `--cd` is accepted; choose a different checkout through
Astral's `--root`. Direct initialization can forward profiles, OSS selection and
extra writable directories because it does not stage a thread.

`astral project --non-interactive -- ...` runs a headless Codex worker with
selected context and the caller's task. Native launch, automatic worktree binding,
record operations and explicit save/handoff are implemented within the limits in
the [milestone plan](launch-plan.md). Fresh bound workers can opt into `--proxy`;
fresh launches without `--work` use the direct route and do not support Astral's
receipt-based `--resume`. The implemented
[finish and resume](finish-resume.md) workflow adds status, recovery, branch
completion and opt-in advisory hooks. A bound worker that is later saved natively
needs `--proxy` for subsequent continuation, even when its original selection
was document-only.

See the dated [verification receipt](fresh-launch-verification.md) for local and
live-runtime results, scanner findings and untested boundaries.
