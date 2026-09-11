# Worktree binding and native handoff review

This projection references a locally validated native checkpoint bundle. Runtime tools, permissions, workspace and account routing are supplied at launch. Recorded commands and test outcomes are historical until verified in the destination workspace.

## Current continuation guidance

The checkpoint captures the worktree/save review described below, not the current
repository's complete implementation state. Its original follow-up, model-catalog
forwarding (`AST-dj8adv3zgkft`), is complete. Status/doctor, recovery, branch
completion, Git/Codex advisory hooks and terminal hook confirmation were also
implemented afterward. Use the [current subsystem overview](../../core/project-context/README.md)
and work register to select remaining work.

The saved window includes historical document copies and earlier runtime names.
Those bytes are part of the immutable export and have not been rewritten. Native
launch appends the currently selected repository documents; they provide updated
project guidance alongside the historical conversation. The standalone proxy now
uses `.astral-runtime/`, `ASTRAL_UPSTREAM` and `x-astral-*`; see the
[current naming and storage contract](../../core/project-context/README.md#names-and-storage).
Current CLI summaries are human-readable by default; scripts must request `--json`.

## Saved review provenance

Work: `AST-e093mvsf0xvj`, completed 2026-09-11. The worker was launched and
resumed through Astral in its automatically bound Git worktree. It reviewed
`e20ddf2`, identified three integration defects, then reviewed their fixes in
`984ef02` in the same conversation:

- Retry known failures before thread creation without creating duplicate workers.
- Preserve the selected native projection independently of later named exports.
- Compare resolved subsystem dependencies while retaining authored scope lists.

The worker found no remaining concrete issue in that fix. It reviewed source and
tests; the parent ran the checks and live controls recorded in
[the dated handoff receipt](../../../docs/worktree-handoff-verification.md) (297 Rust tests, 18 Python tests, formatting,
strict Clippy, release builds, native saves, Git transfer, recall, execution and
permission controls). These receipts describe the recorded implementation.

Resume this saved review with:

```sh
astral project projection:worktree-handoff-review --proxy
```

Use a new work record plus `--work ID` for a separate bound continuation. The
default `project-context` selection remains the project's document-based entry.
Same-account Codex 0.154.0 / OpenAI / `gpt-6-astra` is the supported
native lane; this export does not establish cross-account portability.
