# Review and finish a worker branch

`astral finish` reviews committed work before integrating it into an explicitly
selected target checkout. It does not save a conversation, stage files, commit,
change branches, mark a work record complete, push, or remove either worktree.
The first release supports read-only plans and reviewed fast-forward integration.

Close the worker, review its code and decisions, run the relevant checks, and
commit the intended changes using ordinary Git. If a native handoff is wanted,
explicitly save it first, review the export, and commit it with the work. Then
invoke finish from the checkout already on the destination branch:

```sh
astral --root /path/to/main-checkout finish --work AST-EXAMPLE --into main
astral --root /path/to/main-checkout finish --work AST-EXAMPLE --into main \
  --context projection:reviewed-save
```

Use a real work ID. `--context` selects the reviewed next starting context; it
neither rewrites a projection nor changes the original worker's binding. Omit it
to use the worker's stored selector when the native choice is unambiguous.

The plan is bounded JSON. It includes both committed revisions, their merge base,
path changes grouped as code, readable context/manifests, work records or native
artifacts, record-level three-way conflicts, retained artifact object IDs and the
chosen context's source handles. Source documents and opaque payload bodies are
not printed. All declared project inputs, including unselected contexts and
native data, must be tracked regular files whose committed bytes exactly match
the confined reader's observations. A declared ignored bundle is not portable
merely because it exists in the worker checkout.

A plan reports one of these states:

- `fast_forward_ready`: the target can advance to the reviewed worker commit and
  required context/artifacts are retained.
- `manual_integration_required`: Git histories diverge, records conflict, native
  context needs a choice, tracked files would be deleted, or retention evidence
  needs review. Use ordinary Git and explicit reconciliation, then replan.
- `already_integrated`: worker ancestry is present, required artifacts and chosen
  context are retained, and committed worker changes remain as reviewed. Valid
  later work-record status/evidence updates do not undo that result.

`ancestry_integrated` is reported separately. An ordinary merge followed by a code
revert can retain ancestry while losing the reviewed result. Nonexact changes to
worker-modified code or readable documents conservatively require manual review;
this is not a semantic merge verifier. Reconciled work registers are validated
separately. A deleted target work record cannot count as retained completion.

## Apply a reviewed fast-forward

Copy `plan_sha256` from the reviewed plan:

```sh
astral --root /path/to/main-checkout finish --work AST-EXAMPLE --into main \
  --context projection:reviewed-save --apply PLAN_SHA256
```

Use the same selector and arguments that produced the plan. Apply reacquires the
worker lock, rebuilds the plan and compares its hash before mutation. It checks
both refs and working states again, then performs only `git merge --ff-only` into
that checkout. Hooks, fsmonitor, submodule recursion and automatic maintenance
are disabled for Astral's plumbing; no repository script is run. Ordinary Git
integration remains available with its normal configured behavior.

After success, explicitly update the work record with reviewed evidence using
`astral work update`, then commit that update. Finish reports both target and
worker record statuses but never changes them. A returned `next_context_argv` is
an unexecuted suggestion to start a new session from the chosen context; it adds
`--proxy` when that native context requires it. The original worker remains
available under its existing work ID.

If the caller loses the result after Git advances, replan. An already-integrated
result is a no-op, including a retry with the earlier valid-format plan hash.
A stale hash never permits another mutation. If Git fails or post-apply evidence
cannot be verified, retain both checkouts and inspect/replan; finish performs no
rollback, reset or repeated commit. No private progress file is needed because
the branch and committed artifacts are the observed progress evidence.

## Preserve native histories and local data

Distinct target/worker native inventories require an explicit next-context
choice. That choice does not permit discarding the other history. The plan checks
all currently declared artifact paths and every tracked `.astral/**/bundles/**`
path, including superseded bundles no longer named by a projection. Conflicting
bytes at an immutable path require manual reconciliation. Keep both artifacts,
reconcile readable decisions and records, then select a projection explicitly.
Finish never concatenates opaque payloads or claims semantic native equivalence.

Both checkouts must be clean and the worker must have a complete recorded staging
state with no pending save/recovery. The existing worker ownership lock is held
through planning/apply and released on return. It excludes cooperating Astral
owners; unrelated Git or raw Codex writers must also be stopped by the operator.
No live runtime or thread query is made, so lock availability is not proof that
an independently opened Codex session is inactive.

Unrelated ignored build output may remain. Ignored paths that overlap incoming
tracked files are refused, and apply also uses `--no-overwrite-ignore`. No ignored
file is deleted or overwritten as cleanup. Uncommitted tracked or ordinary
untracked changes are refused without automatic stashing.

The policy preflight reads Git's normal configuration, including system/global
configuration, includes and process-scoped `GIT_CONFIG_*` settings. It does not
execute configured hooks, filters or verification programs. The automatic lane
rejects tracked Git attribute files, Git info attributes, configured checkout
conversions or external filters, submodules, branch-specific merge options, and
configured merge signature verification. Actual Git mutations still use the
isolated plumbing configuration; the preflight prevents that isolation from
silently bypassing an operator's unsupported policy. It does not weaken a
verification policy or invoke filters to make the operation work. These
cases require ordinary Git integration; replan only within a supported checkout
policy. Tracked deletions also require ordinary Git, rather than automatic removal.

The initial bounds are 4,096 committed paths per tree and compared path union,
1 MiB ordinary Git output, up to 8 MiB for a native payload blob, and 2 MiB plan
JSON. Every Git subprocess uses the existing 30-second deadline. Project input
budgets still apply. Ref and file observations are rechecked, but this is not an
atomic transaction against uncooperative filesystem or Git writers.

The [finish/resume roadmap](finish-resume.md) distinguishes this bounded workflow
from hook integration, broader semantic reconciliation and other runtime formats.
Historical test receipts remain historical; finish does not run or verify tests.
