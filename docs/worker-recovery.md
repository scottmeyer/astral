# Recover interrupted worker operations

Recovery repairs private destination bookkeeping from explicit evidence. It does
not start Codex, repeat a compaction or injection, edit a checkout, or grant runtime
permissions. First inspect the work:

```sh
astral status --work AST-EXAMPLE
astral recover --work AST-EXAMPLE
```

`recover` prints a readable plan with its classification, diagnostics and next
action. Add `--json` for the full structured evidence, proposed metadata and
`plan_sha256`, for example `astral recover --work AST-EXAMPLE --json`. Review the
bound checkout and proposed repair, then supply the full plan hash:

```sh
astral recover --work AST-EXAMPLE --apply PLAN_SHA256
astral doctor --work AST-EXAMPLE
```

The apply command reacquires exclusive worker ownership and checks the receipt,
filesystem/Git identity and relevant context/artifact hashes again. A changed
plan is an error; inspect a new plan. No reservation is held between preview and
apply. A successful repair updates only private bookkeeping and retains repair
evidence. Repeating an already recorded repair does not repeat its mutation.
The subsequent launcher still performs its runtime preflights.

## Supported repairs and uncertain outcomes

| Observed state | Behavior |
| --- | --- |
| Worktree creation with a durable post-creation identity proof | Explicitly finish recording that same creation; preserve the checkout and then inspect its initial context |
| Legacy creation without pinned identities | Report unproven creation; do not adopt a matching-looking branch or directory |
| Ready worktree whose initial context acknowledgement is missing | Offer to accept its current validated selected context and work record; do not copy over user edits |
| Missing or invalid context/work record | Report the problem; ordinary reviewed edits can repair the checkout before another preview |
| Acknowledged worker creation/context injection with interrupted shutdown or bookkeeping | Record that exact thread, model/provider and injected context digest; no duplicate creation or injection |
| Thread creation or injection with no durable acknowledgement | Retain the unknown outcome and refuse to clear it automatically |
| Exact saved bundle published while final bookkeeping was interrupted | Validate the bundle and its recorded source digest, then record its saved and selected hashes |
| Pending save whose expected bundle is absent or differs | Preserve the save attempt and artifacts; do not infer publication or repeat compaction |

New bound-worker staging records an acknowledgement after the runtime confirms
the injection and before closing its app-server process. New saves retain the
captured bundle hash before publication. Older receipts cannot acquire proof
retroactively. In particular, matching paths or finding a thread ID in a transcript
does not prove that the intended native window and current context were injected.

Readable documents may have changed after acknowledged staging. Recovery retains
the acknowledged digest so the next real resume can append the newer documents.
A changed selected native history is a different starting context and blocks this
repair. A repaired save keeps native rebinding required and preserves immutable
exports; it does not claim provider decryptability or successful continuation.

An acknowledged staging operation may still have failed while closing its local
app-server. Ownership coordinates Astral processes, and does not exclude direct
Codex/Git operations by the user. Close any independently running writer before
applying a repair or resuming a worker.

## Find retained state outside the current work register

```sh
astral recover
astral recover --offset 32 --limit 32
astral recover --launch-state-root /absolute/private/launches
```

The binding inventory includes IDs missing from the invoking checkout's work
register. It reads only the current repository's private binding directory;
`ASTRAL_WORKTREE_ROOT` retains its existing meaning as trusted host configuration.
Receipt-only native launches are scanned only when an absolute private launch root
is supplied explicitly. Astral does not scan home directories or conversation
rollouts to discover a possible match. Valid foreign workspace/project launches
are excluded; invalid entries produce bounded diagnostics, not raw receipt text.

Pagination defaults to 32, with at most 64 observations per page. Files, directory
entries and Git subprocess outputs/deadlines are bounded. Launch inventory permits
at most 1,024 total directory entries and 16 KiB per receipt; unknown names count
toward that limit without being opened or displayed. Binding inventory permits
at most 4,096 work IDs. Missing locks, unsafe
paths, changed identities or concurrent owners prevent a repair. Observation
supports Unix, matching bound worktrees. Applying a repair requires Linux or
macOS for atomic evidence publication; this pass was tested on macOS.

Preview and inventory success mean a report was produced; inspect its diagnostics
and `repair` field. Invalid arguments, stale plans, unsupported repairs and failed
applications exit 2. After repair, `doctor` can report remaining local blockers.
Neither command verifies remote threads, test results, opaque recall or execution.

Use [branch completion](branch-completion.md) once the worker is stopped and its
results are ready for review. The broader [finish/resume plan](finish-resume.md)
tracks lifecycle hooks and semantic context reconciliation separately.
