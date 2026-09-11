# Subsystem context freshness

Available in source builds after v0.1.0. This first slice detects changes against
an explicit review baseline. It does not decide whether documentation is true,
run tests, or qualify historical verification as current.
The [verification record](context-freshness-verification.md) separates fixture
results from the remaining long-horizon evaluation work.

## Declare the inputs

Add an optional table to a registered subsystem's `subsystem.toml`:

```toml
[freshness]
inputs = ["src/web.rs", "src/web/routes.rs", "tests/web.rs"]
```

Paths are exact files relative to the repository root. There is no glob expansion
or directory traversal. Subsystems without this table remain untracked for
freshness; absence of a declaration is not a claim that their knowledge is current.
Use `astral context freshness web` to inspect a selection and its dependencies,
or omit `web` for `project-context`. Projections can be inspected with
`projection:NAME`; review acknowledgement targets one subsystem at a time.

The fingerprint covers declared code bytes, shared `ARCHITECTURE.md`, `RUN.md`
and `TEST.md`, and the selected subsystem/dependency manifests and documents.
Project manifest semantics are also included. Review digests themselves are
excluded, so acknowledging a dependency does not invalidate its dependents.
Code bytes are hashed and discarded; they are not added to model context.

The baseline excludes unlisted files, work-record contents, projection handoffs,
and native conversation history. Adding a new file to the repository does not
discover it: add its path to the declaration when it matters. A deleted or renamed
declared file becomes unavailable until the declaration is repaired. Runtime
configuration and executable-mode changes are outside this byte comparison.
Code and document fingerprints compare raw bytes; manifest fields normalize TOML
formatting and exclude review digests. Git clean/smudge filters or line-ending conversion
can make working and stored versions differ even when their meaning is equivalent.

## Review and acknowledge

```sh
astral context freshness web
astral context review web
# In a terminal, review the displayed scope and confirm the acknowledgement.
astral context review web --dry-run --json
# For scripts, after reviewing the inputs and their knowledge:
astral context review web --yes
# Or explicitly apply a previously inspected preview:
astral context review web --apply PLAN_SHA256
git diff -- .astral/core/web/subsystem.toml
```

In a terminal, the review command shows the plan and asks for confirmation.
Enter, `no`, EOF and partial answers make no changes. Redirected or `--json`
invocations default to a read-only preview. `--dry-run` always previews; `--yes`
explicitly acknowledges the current inputs without prompting. These flags and
`--apply` are mutually exclusive. Every application checks the exact preview
against the current checkout, manifest and observed inputs, then writes
`freshness.reviewed_fingerprint` to the subsystem manifest. TOML comments and
formatting are preserved. A changed preview is rejected; run the review again
after inspecting the new changes. This acknowledgement records that an explicit
review was requested, not who performed it or whether it was correct. Git carries
its provenance and can compare or merge it alongside the code.

| State | Meaning | Next step |
| --- | --- | --- |
| `unreviewed` | Inputs can be read, but no baseline is recorded | Review the knowledge against its inputs, then acknowledge |
| `unchanged` | Observed fingerprint equals the review baseline | Keep historical test results historical |
| `needs_review` | Code, documents or declarations differ from the baseline | Inspect the changes, update knowledge if needed, then review |
| `unavailable` | One or more declared inputs cannot be safely observed | Repair the path, conflict, mode, permissions or budget issue; acknowledgement is refused |

All commands provide human output by default and formatted machine output with
`--json`. Successful freshness reports exit zero even if review is needed;
scripts should inspect each `state`. Invalid declarations and failed apply
operations return errors. `context validate` still validates the manifest and
document contract; its freshness rows are a separate observation.

## Git and workers

`astral lifecycle check` observes working, staged and committed freshness
independently. Staging the review manifest without the corresponding code cannot
make a different staged implementation unchanged. Missing staged inputs do not
fall back to working files. The same diagnostics reach opt-in Git/Codex hooks;
hooks never acknowledge a review or block a commit.

Stage intended code, documents and their review manifest together. After a merge,
run another check: a baseline from either branch is useful only if its complete
fingerprint still matches the resulting inputs. Divergent review fields remain
ordinary Git conflicts; there is no automatic semantic merge or latest-wins rule.

Selected freshness observations also participate in worker context digests and
accompany current document context. A relevant code change can therefore prompt
worker context refresh even when no document changed. It does not run inference,
save a new native checkpoint, or change a worker receipt during inspection.

## Bounds and remaining work

There are at most 64 distinct code inputs across a project. They share the existing
1 MiB per-file, 16 MiB aggregate and 2,048-file reader budgets with declared context.
Inputs must be regular files outside `.git` and `.astral`; symlinks, unsafe Git
modes and unmerged entries are unavailable. The checks use exact Git-object reads
and bounded working-file reads, independent of unlisted repository size.
Hooks retain their existing deadline; a slow or oversized observation may report
unavailability rather than completing within it.

Review publication uses Unix descriptor-relative, owned directories and regular
single-link files that are not writable by other users. Cooperating reviewers and
work-record writers lock `.astral` while applying. The manifest is replaced
atomically after rechecking the inputs; ordinary editors do not take that lock.
Stop other edits during acknowledgement. This is not an atomic snapshot of every
file, and a later change requires another observation. A storage error after
publication may require inspection before retrying; retained temporary files are
not review authority.

This slice supplies byte-change diagnostics. Per-claim evidence, supersession,
budgeted retrieval over long histories, semantic stale-claim detection, glob or
directory discovery, and evaluations of recall quality remain future work.
