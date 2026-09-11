# Bootstrap scope and identity

Date: 2026-09-11. Requirement status: accepted. Schema status: proposed.

The user requested a small Git-native context tree and JSONL work register after
native recovery. A supported fork first verified the requested conversation's
snapshot. After the user closed its existing session, the original ID passed
execution and continuity checks, including a cold resume after proxy restart.
The native lifecycle is tested separately on disposable fixtures.

This bootstrap uses repository-relative paths and logical names. Its
`schema_version = 1` is a local draft, not an assertion of an existing public
schema or implemented launcher. Runtime IDs and native artifact locations are
resolved privately and are not portable project identities.

Historical proposed work IDs AST-001 through AST-006 are reserved and retained
with their original meanings. AST-007 through AST-010 add the bounded lifecycle,
bootstrap, launcher coordination, and further protocol qualification work. This is an initial JSONL snapshot:
one object per item, unique IDs, explicit dependencies and acceptance criteria.
Merge/event semantics require a later schema decision.

The projection handoff is a readable design record. It does not replace the
native conversation, establish opaque recall, or supply executable capabilities.
