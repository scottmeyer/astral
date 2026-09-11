# Current project workflow

This is a readable starting point for maintaining Astral. It contains no native
checkpoint. `astral project` selects the `project-context` subsystem; an explicit
`astral project projection:project-workflow` also includes this handoff.

The launcher, worktree/thread binding, work-record operations, native save,
status/doctor, evidence-based recovery, reviewed fast-forward completion and
opt-in Git/Codex advisory hooks are implemented. Hook setup supports same-command
terminal confirmation, `--yes`, `--dry-run` and the retained `--apply HASH` mode.
Recovery and completion still use separate preview and `--apply PLAN_SHA256`.

Commands show human-readable summaries and next actions by default. Use global
`--json` for scripts. In `project`/`init`, an argument after the first literal
`--` is forwarded to Codex, including Codex's own `--json`. The standalone proxy
uses `.astral-runtime/`, `ASTRAL_UPSTREAM` and `x-astral-*`; private state in an
existing directory remains available through explicit `--state-dir`.

Use the [subsystem overview](../../core/project-context/README.md) for current
scope and open work, [run guide](../../core/RUN.md) for commands, and
[test guide](../../core/TEST.md) for checks. Inspect current source and local
changes before relying on a claim from any saved conversation.

`git-native-context-bootstrap` preserves early design provenance.
`worktree-handoff-review` preserves a native review from the worktree/save
milestone. Both are retained for explicit selection; neither defines today's
remaining work. In particular, model-catalog forwarding, status, recovery,
completion and lifecycle hooks were implemented after that native review.

Old names and completed-task instructions can still appear inside saved native
windows. Immutable bundle bytes and hashes must remain unchanged. Refresh their
editable handoffs and selected current documents; a new native snapshot requires
an explicit save from a compatible stopped worker. Do not manufacture a new
checkpoint by rewriting a saved window.

Fresh document launch is direct by default. Bound workers require Codex 0.154.0;
native save/import additionally require the documented same-account OpenAI /
`gpt-6-astra` route and explicit `--proxy`. The destination supplies tools,
authentication and permission enforcement. No current test or recall result is
asserted by this handoff.
