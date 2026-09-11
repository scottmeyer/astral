# Bootstrap scope and identity

Date: 2026-09-11. Historical bootstrap decision; its requirements were accepted
and the version-one schema was subsequently implemented. Current workflow:
[project-workflow](../../../projections/project-workflow/handoff.md).

The user requested a small Git-native context tree and JSONL work register after
native recovery. A supported fork first verified the requested conversation's
snapshot. After the user closed its existing session, the original ID passed
execution and continuity checks, including a cold resume after proxy restart.
The native lifecycle is tested separately on disposable fixtures.

This bootstrap uses repository-relative paths and logical names. Its
`schema_version = 1` began as a local draft, not an existing public standard.
The launcher is now implemented. Runtime IDs and native artifact locations are
resolved privately and are not portable project identities.

Historical proposed work IDs AST-001 through AST-006 are reserved and retained
with their original meanings. AST-007 through AST-010 add the bounded lifecycle,
bootstrap, launcher coordination, and further protocol qualification work. This is an initial JSONL snapshot:
one object per item, unique IDs, explicit dependencies and acceptance criteria.
The later [work-ID decision](work-identifiers.md) defines the implemented
snapshot update and three-way record merge behavior. This is not an event log.

The projection handoff is a readable design record. It does not replace the
native conversation, establish opaque recall, or supply executable capabilities.
