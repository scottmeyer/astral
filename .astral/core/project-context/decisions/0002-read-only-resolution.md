# Read-only resolution before execution

Date: 2026-09-11. Status: adopted implementation decision for AST-001.

Historical scope: the paragraphs below record the first read-only milestone.
Launch, worktree binding, work-record operations and explicit native save were
implemented afterward. The inspection boundary still applies; the initial
requirement to use `--inspect` is not a restriction on today's launcher.

Build a bounded Rust validator and named-context inspector before introducing
runtime or Git side effects. The first `astral project NAME` interface requires
`--inspect`; it must not present selection as successful native recovery.

Keep registry names equal to subsystem IDs, allow explicit namespace qualification
for subsystem/projection collisions, and require declared dependency graphs to be
acyclic. Work items remain unique-ID JSONL snapshot records. Their later update
and merge semantics are recorded in [the work-ID decision](work-identifiers.md);
the external adapter portion of AST-002 remains open.

Return deterministic, repository-relative source handles and hashes rather than
injecting the whole core into model input. Readable context and unbound native
artifacts stay distinct. Source provenance is data, never execution authority.

Use descriptor-relative Unix reads and reject unsupported platforms explicitly in
v0. A future cross-platform implementation must preserve the confinement contract.
No file hash or selection digest claims atomic repository snapshotting or provider
semantic equivalence.

See [the current implementation contract](../../../../docs/project-context.md),
[bound workers and handoff](../../../../docs/worktree-handoff.md), and the implemented
[finish/resume workflow](../../../../docs/finish-resume.md).
