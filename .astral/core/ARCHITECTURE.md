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

The unified binary is `astral`; `astral proxy` starts the server, while `project`
and `context` inspect project context. The internal Rust crate remains
`ostk-gpt-cache`. `src/server.rs` owns startup and shutdown; `src/proxy.rs` owns HTTP
routing; `engine.rs`, `store.rs`, and `policy.rs` implement the existing projection,
persistence, and cache policies. `working.rs` supports the optional explicit host.
These are distinct from the experimental Codex compatibility path.

`native_binding.rs` validates Responses Lite inputs and reasserts current runtime tool
declarations after the effective native checkpoint. `native_transport.rs` owns one
downstream/upstream socket pair and its isolated incremental state. The adapter
does not execute tools or grant permissions. Codex owns those responsibilities.
The compatibility path requires pass-through mode, which disables Astral rolling.

See [the lifecycle record](../../docs/native-recovery-lifecycle.md) for the tested version,
route, transitions, limitations, and evidence classification. Earlier evaluation
claims in `docs/` retain their original scope and dates.

## Project context

`src/project.rs` validates the versioned TOML and JSONL contract and resolves
selected core, subsystem, projection and work-item sources. The `astral` binary
provides `context validate`, `context list`, and `project NAME --inspect`. It
returns bounded repository-relative handles and hashes with native state UNBOUND.
It reads confined regular files on Unix; it does not launch a runtime or execute
documented commands. Source hashes observe declared inputs, not an atomic tree.

A future launcher must bind the destination runtime and workspace explicitly.
Direct Codex is the default; `--proxy` opts into Astral routing. User-supplied Codex
arguments must be forwarded unchanged, with `--` disambiguating overlapping
options. Native checkpoint requirements must be reported before launch.
A private availability map would locate native
artifacts without putting machine paths, credentials, or thread IDs in portable
identity fields. Native payload storage and sharing policy remain undecided.

## Open questions

- Future schema evolution and runtime compatibility/version negotiation.
- Native artifact access, account/model compatibility, retention, and sharing.
- Conflict reconciliation for concurrent context and JSONL work-item changes.
- Which runtime metadata is essential when moving a native window between hosts.
- Protocol negotiation and additional lifecycle transitions beyond the tested path.
