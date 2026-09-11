# Bound workers and native handoff

`astral project --work ID` creates or reopens one Git worktree and one local Codex
thread for a work record. Every bound worker currently requires Codex 0.154.0,
including workers started from documents. Without `--work`, launch keeps the
existing checkout behavior. Direct Codex remains the default; native checkpoints require explicit
`--proxy` on the supported Codex 0.154.0 / OpenAI / `gpt-6-astra` route.

## Start and continue work

```sh
astral work create 'Review web authentication' --acceptance 'Record findings and relevant checks'
astral project web --work AST-EXAMPLE --inspect
astral project web --work AST-EXAMPLE -- --model gpt-6-astra
astral project web --work AST-EXAMPLE --non-interactive -- --json 'Continue the review.'
```

Replace the illustrative ID with the ID returned by `work create`. Inspection is
an unlocked, no-write observation. It creates neither a branch nor a reservation.

The first launch creates `astral/ID` at the invoking checkout's committed HEAD.
Its default location is a sibling of the main checkout:
`.astral-worktrees/REPOSITORY_HASH/ID`. `ASTRAL_WORKTREE_ROOT` can name a trusted,
absolute external parent. Existing branches or paths are not adopted implicitly.
Git common-directory and filesystem identities distinguish repositories, even
when they have the same project ID or were copied locally.

Commit `.astral` context changes before creating a worker. The validated configured
work-record file is carried automatically, including dependencies, so a newly
created record can launch before it is committed. Other source edits remain in
the invoking checkout. Repeated launch uses the bound worktree's current content;
it preserves worker edits and resumes the same thread. Changed selected documents
are appended as current project context. Old native history is not reinjected.

Each work ID has one private ownership lock, held through staging, Codex exit,
and proxy cleanup. Other IDs can run concurrently. Private receipts and proxy
state live below the Git common directory's `astral/work-bindings`, outside tracked
content. A copied receipt, changed branch association, missing worktree, or competing
Astral owner is an error. Failed creations remain for inspection; Astral does not
stash, reset, prune, or delete user work. These locks coordinate Astral processes;
direct Git/Codex processes do not automatically honor them.

An incomplete initial context copy or an unknown initial staging outcome is
reported explicitly on retry. Use `astral recover --work ID` to preview supported
repairs from retained evidence. Astral does not overwrite the retained worktree
or create another thread implicitly. See [worker recovery](worker-recovery.md)
for acknowledged staging, uncertain outcomes and explicit apply hashes.
Failures before a thread could be created, such as a missing Codex executable,
unsupported version, or failed initialization, can be corrected and retried with
the same work ID.

`--work` owns continuation identity, so it cannot be combined with `--resume`.
The latter retains its meaning for native launches without a work ID. All caller
Codex arguments after `--` remain literal arguments of the launched Codex child;
manifests and receipts cannot supply runtime flags or permission grants.

## Save and hand off

Close the worker before saving it:

```sh
astral save web-review --context web --work AST-EXAMPLE --proxy
```

Save resolves the existing binding and exclusively opens that stopped worker.
It explicitly requests a new native compaction through the owned rebinding proxy,
waits for a matching successful completion, and reads the pinned local rollout
format without editing it. The export is exactly the new native replacement
window. Display transcripts, readable summaries, and unresolved history references
are not substitutes for opaque content.

The named projection is published in the worker's checkout:

```text
.astral/projections/web-review/
  projection.toml
  handoff.md
  bundles/<manifest-sha256>/
    manifest.json
    window.json
```

Bundle bytes are immutable. A later save advances the named reference and retains
prior bundles and parent hashes. Publication validates the prospective project
and atomically replaces only the named reference. Staging evidence is private in
the Git common directory. The initial publisher requires staging and the target
checkout to share a filesystem. Save does not commit or push.

Save does not write an updated task summary: a new projection receives a generic
handoff, and an existing projection keeps its authored handoff text. Review and
update readable decisions and `handoff.md` explicitly before committing a handoff.

The original worker now contains native context. Resume it using its original
selector and work ID with `--proxy`, including a worker that originally launched
directly from documents:

```sh
astral project web --work AST-EXAMPLE --proxy -- --model gpt-6-astra
```

A worker may save under multiple names. Its selected projection is tracked
separately from its latest export, so saving `p`, then `q`, still allows the worker
bound to `p` to continue. Existing projections retain their authored subsystem
list; scope comparisons include transitive subsystem dependencies.

Review and commit the worker's code, work records, and explicit export. Merge or
fetch that commit into another checkout, then launch:

```sh
astral project projection:web-review --proxy
```

Use a new work record and `--work NEW_ID` to make the imported continuation another
bound worker. An explicit native projection selects one conversation; its linked
subsystems still supply current documents. Selecting a subsystem whose dependencies
contain distinct native histories remains an error.

The destination supplies current executors, tool declarations, workspace, routing,
authentication, approvals, and sandbox policy. Native bundles preserve conversational
state; they do not restore the source's host authorization or working-state sidecars.
Same-account compatibility is the initial supported scope. Cross-account teammate
portability, other runtimes/providers, semantic compaction fidelity, deterministic
answers, and token or invoice savings are not established by this feature.

Save accepts only standalone, bounded Codex 0.154.0 rollouts with a newly completed
supported compaction. Referenced/forked histories, rollback reconstruction, unknown
metadata, concurrent writing, and incomplete boundaries fail explicitly. A failed
save may still have compacted the worker; its private binding then requires
`--proxy` for subsequent continuation. No artifact is published until validation
and completion checks succeed.

If publication succeeds but the final private receipt update fails, the command
reports `SAVE_PUBLISHED_METADATA_FAILED` and the published artifact handles.
Preserve those artifacts; a new work ID can import the published projection.
The existing worker will not silently adopt an unrecorded changed seed.
New saves retain prospective publication evidence, allowing `astral recover`
to validate the exact published bundle and explicitly finish that bookkeeping.

## Work records and Git conflicts

```sh
astral work list
astral work update AST-EXAMPLE --status in_progress --expected RECORD_SHA256
astral work merge --base base.jsonl --ours ours.jsonl --theirs theirs.jsonl
astral context validate
```

Creation uses short random IDs. Updates compare the exact observed record digest
and retain other fields. Merge returns JSON containing merged JSONL, or explicit
record-ID conflicts without a partial result (exit status 1). Independent additions combine;
different concurrent edits, different additions under the same ID, and edit/delete
disagreements conflict. The merged dependency graph is validated. After applying a merged file,
validate the project to check subsystem references as well.
Identical semantic additions on both branches coalesce. Duplicate IDs within
one input file are invalid regardless of content.

Do not install a concatenating JSONL merge driver. Distinct native artifacts can
coexist, but encrypted histories cannot be semantically merged by Git. Preserve
both bundles, reconcile readable decisions and records, and explicitly choose
a starting projection or create a reconciliation worker.

`astral finish --work ID --into main` previews committed branch results and
remaining stages. An explicit apply hash can authorize a checked fast-forward;
ordinary Git merges are recognized on the next preview. See
[branch completion](branch-completion.md) for context selection and limits.
