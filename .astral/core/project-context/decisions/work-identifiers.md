# Work identifiers across branches

Status: accepted requirement and implemented allocation policy, 2026-09-11.

New work IDs use `AST-` followed by twelve lowercase Crockford base32 characters,
carrying 60 bits from OS randomness. Never allocate by incrementing the largest
ID in the current branch. Titles, timestamps and branch names do not determine
identity, so editing a work item does not change its ID.

`astral work id` validates the current project and proposes an ID absent from its
observed work register. It prints the ID; it does not create or reserve a record.
The allocator checks at most 4,096 observed IDs and retries a known collision up
to sixteen candidate draws. Entropy failure is an error, without a weaker fallback.

Existing sequential IDs remain stable for historical references. New records,
including launch milestones, use random IDs. Randomness makes cross-branch
collisions unlikely, not impossible: writers and merge validation must still
reject duplicates. Never silently renumber one side of a conflicting merge.

Unique IDs do not by themselves reconcile a shared JSONL file. The planned
record-aware three-way merge unions independent additions and reports divergent
edits/deletions of the same ID. Dependency validation runs after reconciliation.
Native checkpoint histories require separate handling; they are not work records.
