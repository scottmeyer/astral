# Architecture

## Accepted requirements

Git should carry durable, reviewable project context: a small core, user-defined
subsystems, named investigation projections, and committed JSONL work items.
Branching and merging should carry that work state with the code. The requested
example is `astral project web --work ISSUE-123`; it remains a future interface.

Import preserves conversational work state. Resume supplies executable
capabilities from the current trusted runtime. Native checkpoints must remain
native; readable documentation is a separate artifact, never evidence of native
recovery. Historical commands require a current instruction before execution.

## Verified current structure

The Rust crate and proxy binary are `ostk-gpt-cache`. `src/proxy.rs` owns HTTP
routing; `engine.rs`, `store.rs`, and `policy.rs` implement the existing projection,
persistence, and cache policies. `working.rs` supports the optional explicit host.
These are distinct from the experimental Codex compatibility path.

`ast000.rs` validates Responses Lite inputs and reasserts current runtime tool
declarations after the effective native checkpoint. `ast000_ws.rs` owns one
downstream/upstream socket pair and its isolated incremental state. The adapter
does not execute tools or grant permissions. Codex owns those responsibilities.
The compatibility path requires pass-through mode, which disables Astral rolling.

See [the lifecycle record](../../docs/ast000-lifecycle.md) for the tested version,
route, transitions, limitations, and evidence classification. Earlier evaluation
claims in `docs/` retain their original scope and dates.

## Proposed implementation

The TOML files here are a draft documentation schema. No `astral project` command
loads them yet. A future resolver could compose core context, selected subsystems,
work-item state, and available native projections, then bind the destination
runtime and workspace explicitly. A private availability map would locate native
artifacts without putting machine paths, credentials, or thread IDs in portable
identity fields. Native payload storage and sharing policy remain undecided.

## Open questions

- Manifest validation, compatibility/versioning, and projection selection rules.
- Native artifact access, account/model compatibility, retention, and sharing.
- Conflict reconciliation for concurrent context and JSONL work-item changes.
- Which runtime metadata is essential when moving a native window between hosts.
- Protocol negotiation and additional lifecycle transitions beyond the tested path.
