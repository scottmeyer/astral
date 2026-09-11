# README and portable context refresh — 2026-09-11

Audited from `f3fc85affa12a468cd9997eae4dcfcad0d0753f2` on
`docs/current-context-audit`. This pass changes documentation, context manifests
and work-record descriptions; runtime source, dependencies and native bundle
bytes are unchanged. The results below describe this pass, not future edits.

## Source-backed corrections

- All bound workers require Codex 0.154.0, including fresh document workers
  (`src/codex/staging.rs`). Unbound fresh staging has no equivalent exact-version
  gate, which does not establish compatibility with arbitrary app-server versions.
- Native save records a tool-rebinding requirement; later continuation of the
  original worker needs `--proxy` (`src/save.rs`, `src/worker_launch.rs`).
- Save creates a generic handoff for a new projection and retains an existing
  handoff. It does not refresh its task summary (`src/projection_save.rs`).
- Recovery, completion, model-catalog forwarding and advisory hooks are
  implemented. Older descriptions of them as the next milestone were corrected.
- Named projection references can advance; bundle bytes remain immutable.
  Record merging accepts identical semantic additions and conflicts on different
  concurrent additions under the same ID (`src/work_records.rs`).
- `.ostk-gpt`, `OSTK_GPT_UPSTREAM`, `x-ostk-*` and the internal Cargo/crate names
  remain actual compatibility identifiers. Documentation now distinguishes those
  from portable `.astral/` context and managed proxies' private storage.

The default subsystem now links the readable `project-workflow` projection.
The early bootstrap and native worktree review remain explicit historical
selections. Two early milestone decisions remain in Git but are no longer in the
default decision list. Editable handoffs describe current continuation guidance;
captured old document copies in the native window remain intact.

## Checks performed

All **87 tests** in `project`, `native_project`, `launch_cli`, `work_records` and
`hook_confirmation` passed with locked, offline Cargo and isolated Git configuration.
The Rust library built successfully. The full Rust/Python suite was not repeated
for this documentation-only pass, and no live provider or Codex session was started.

The installed CLI validated the actual index and inspected all four selectors.
A local probe against the built Rust library also called `Project::launch_context`
for each selection: the default supplied **8 current documents**, explicit
projections supplied **9**, and only `worktree-handoff-review` supplied a native
bundle. Linked readable projection manifests participate in launch fingerprints;
metadata-only inspection does not list them as selected document sources.
Historical milestone decisions and native payloads were absent from the readable
document sets. This establishes assembly and integrity, not model recall.

Local Markdown links/anchors, code fences and `git diff --check` were checked.
The native bundle's manifest SHA-256 remains
`4d252b65c558eaf519f694ff7ce00f6df104d203e4d7791c8a7f714bc9b0f76e`, and its window
SHA-256 remains `484c64603e0c4e745c4d91c38c43a6ad63862d94f9054f25ab7c145860fe7116`.
The banner is also unchanged. No checkpoint was regenerated or rewritten.

The required staged UBS invocation has no supported source language in this
Markdown/TOML/JSONL change and returns `no-supported-languages` (exit 3); it is
not a source-code scan pass. Private probe and command logs remain outside Git.
