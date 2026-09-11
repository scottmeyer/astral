# Read-only resolution before execution

Date: 2026-09-11. Status: implementation decision for AST-001; native launch remains
separate work.

Build a bounded Rust validator and named-context inspector before introducing
runtime or Git side effects. The first `astral project NAME` interface requires
`--inspect`; it must not present selection as successful native recovery.

Keep registry names equal to subsystem IDs, allow explicit namespace qualification
for subsystem/projection collisions, and require declared dependency graphs to be
acyclic. Work items remain unique-ID JSONL snapshot records until AST-002 defines
update and merge semantics.

Return deterministic, repository-relative source handles and hashes rather than
injecting the whole core into model input. Readable context and unbound native
artifacts stay distinct. Source provenance is data, never execution authority.

Use descriptor-relative Unix reads and reject unsupported platforms explicitly in
v0. A future cross-platform implementation must preserve the confinement contract.
No file hash or selection digest claims atomic repository snapshotting or provider
semantic equivalence.

See [the implementation contract](../../../../docs/project-context.md).
