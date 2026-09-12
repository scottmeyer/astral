# Project contexts and read-only inspection

Astral supports [fresh launch and initialization](fresh-launch.md),
[native launch](native-launch.md), and [bound worktrees, records and explicit save](worktree-handoff.md).
The resolver and inspection contract below remains applicable independently of
those operations.

Astral's project-context work includes **read-only inspection** of documents and
explicit [native bundles](native-bundles.md), separate from runtime launch. The
repository carries shared context and explicitly exported native data; runtime
credentials and thread bindings remain private. Runtime tool rebinding is a
separate layer used by the native launcher.
The dated [first integration receipt](project-context-verification.md) records
checks and the launch boundary at that historical milestone.
The subsequent [unified CLI receipt](cli-verification.md) records the binary
rename, default context and repaired development scanner.

## Historical first boundary and current inspection

AST-001 introduced a bounded manifest validator and named-context resolver before
launch or Git operations. The read-only commands still do not launch Codex,
execute scripts, fetch artifacts, change Git state, or convert a readable handoff
into native recovery. The v1 manifest contract is repository-local
and experimental, not a general standard for arbitrary agent checkpoints.

The implemented read-only interface is:

```sh
astral --root /path/to/repository context validate
astral --root /path/to/repository context list --plain
astral --root /path/to/repository project --inspect
astral --root /path/to/repository project project-context --inspect --work AST-001
astral --root /path/to/repository project git-native-context-bootstrap --inspect
```

These commands print readable summaries. `context list --plain` is always a
report; without `--plain`, terminal input/output opens the picker described below.
Redirected output remains a report. Add global `--json` for machine-readable
output with two-space indentation, for example `astral --json context list` or
`astral project --inspect --json`. The final
output limit includes formatting and its trailing newline. Inspection with
`--work` observes the proposed or existing binding without creating a branch,
worktree, thread or reservation. Omitting `--inspect` launches the selected
readable context or supported native bundle; native launch requires explicit
`--proxy`. `--work` binds a reusable worker in its own branch/worktree.
The root defaults to the current directory;
it is not an instruction to search other projects or the user's session archive.
Without an explicit context, `astral project` selects `project-context`. An explicit
name must be the first argument after `project`; after options or `--` begin,
positional arguments belong to Codex. For example, `astral project --inspect --
"Continue the work"` previews a prompt for the default context. A missing default
context is reported as unavailable, never silently replaced with another context.

## Interactive context and work selection

```sh
astral context list
astral project --pick
astral project web --pick --work AST-EXAMPLE
astral project web --pick --proxy -- --model gpt-6-astra
```

`context list` opens **Choose a context** when stdin and stdout are terminals
and the terminal supports the picker. `project --pick` requests it explicitly;
otherwise `project` keeps its direct launch behavior. `--plain`, `--json`, or
redirected list output suppress automatic interaction. An explicit picker request
without a usable terminal returns an error rather than waiting for input.

Use Up/Down and Enter, or type to filter. Context search includes its selector and
purpose. The work screen includes **Continue without a work item**, then every
local work record ordered by in-progress, open, blocked and complete status, with
IDs sorting within each status. Search work by ID or title. Backspace revises a
filter; no-match results cannot be selected. An explicit context name or `--work`
sets the initial highlighted row rather than immediately launching it.

**Review launch** shows the effective context, work item, branch, worktree, route
and explicit Codex arguments. Existing workers use their recorded selector and
checkout, even when a different context was chosen on the first screen. Busy or
unavailable bindings show a diagnostic and Back instead of a launch action.
A required native route is named in **Launch with --proxy** or **Resume with
--proxy**; selecting it explicitly opts into the managed proxy. Other runtime,
model, permissions and committed-context requirements remain unchanged.

Escape returns to the previous screen and exits from the context screen.
Ctrl-C cancels from any screen. Browsing and cancellation do not create bindings,
branches, worktrees or runtime sessions. Terminal settings are restored before
launch or exit. After the final action, Astral revalidates the context, Git state
and binding; changes invalidate the review before launch.

`project --pick` cannot be combined with Astral's `--json`, `--plain`, `--inspect`,
`--non-interactive` or receipt `--resume`. `--proxy` is retained. Arguments after
the first literal `--`, including Codex's own `--json`, remain literal child
arguments and are shown on the review screen. Use Page Up/Page Down to read
wrapped review details. For a new bound worker, omit Codex `--cd`/`-C` and let
Astral choose the managed worktree directory.

## Launcher argument contract

The requested route defaults to direct Codex. `--proxy` opts into Astral routing.
Inspection can show that request and the Codex arguments without starting either
process:

```sh
astral project project-context --inspect --model gpt-6-astra "Continue the work"
astral project project-context --inspect --proxy -- --model gpt-6-astra
astral project project-context --inspect -- --dangerously-bypass-approvals-and-sandbox
```

Before the first literal `--`, Astral consumes `--root`, `--work`, `--inspect`,
`--proxy`, `--non-interactive`, `--resume`, `--pick`, `--plain`, and `--json`;
remaining arguments retain their original order. After the separator, everything belongs to Codex, including
its own `--json` option. The same `--json` boundary applies to `astral init`. Use it when a Codex option name or an option's value
could resemble an Astral option. Astral does not need to recognize every Codex
flag. An explicitly supplied permission flag remains unchanged; none is added
by default. The argument is passed as written, with Codex responsible for its
validity and behavior.

The process builder uses raw OS argument strings and `std::process::Command`,
never a shell. Inspection requires UTF-8 for an exact argument preview; it fails
instead of displaying a lossy replacement. The low-level process API preserves
non-UTF-8 arguments on platforms that support them. Limits are 1,024 arguments,
64 KiB per argument and 256 KiB aggregate argument bytes; final inspection output
also remains within the resolver's output budget.

Fresh project launch uses these argument primitives with the
[documented staging limits](fresh-launch.md). Native launch checks runtime
compatibility and manages its proxy's readiness and lifetime. The requested route
in inspection is a preview, not a verified effective route. Native staging checks
effective configuration and rejects conflicting overrides before injection.
A native checkpoint requiring tool rebinding reports that requirement before
direct launch; it never triggers an implicit proxy or plaintext substitution.

`--non-interactive` selects `codex exec resume` with stdin closed; otherwise launch
uses interactive `codex resume`. Use the flags accepted by that Codex mode.
`--resume LAUNCH_ID` names a private native launch receipt for launches without
`--work`. Bound workers continue by repeating the same `--work ID` and selector;
combining the two continuation mechanisms is an error.

## Manifest and selection contract

- `.astral/project.toml` identifies the project and explicitly registers subsystem
  directories. Each registry key matches the subsystem manifest's logical ID.
- Shared core consists of `ARCHITECTURE.md`, `RUN.md`, and `TEST.md` beneath the
  configured core directory.
- `subsystem.toml` selects its README, rules, decisions and optional subsystem
  dependencies. Dependencies are explicit and acyclic, never model-inferred.
- Optional `[freshness]` declares exact repository-relative code `inputs` and an
  explicitly recorded `reviewed_fingerprint`. See [context freshness](context-freshness.md)
  for byte-change diagnostics, review acknowledgement and the precise scope.
- Optional `[[knowledge]]` entries name selected documents or explicit regions,
  with independent code inputs and review baselines. Use references such as
  `knowledge:web/session-revocation` with `context show`, `freshness` and `review`.
  See [named knowledge](knowledge-entries.md) for authoring and scope.
- Named projection directories contain `projection.toml`, a handoff reference,
  subsystem selections and scoped source provenance. Discovery is bounded.
- A `native-checkpoint` projection declares an explicit digest-pinned
  `native_bundle` reference. The [bundle contract](native-bundles.md) defines
  its manifest, immutable payload and structural compatibility validation.
- Work items are one object per JSONL line, with unique IDs, explicit dependencies
  and acceptance criteria. V1 is a snapshot format, not an append-only event log;
  duplicate IDs, missing references and dependency cycles are errors.
- New work IDs come from `astral work id`: `AST-` plus twelve random lowercase
  Crockford base32 characters (60 bits). The command proposes an ID without
  creating/reserving a record. `work create` persists a new record; `work update`
  checks its observed digest, and `work merge` performs a record-aware three-way
  merge. Existing IDs remain stable and duplicate/conflicting records are errors.
  Do not allocate by incrementing a counter.
- An unqualified name must identify exactly one subsystem or projection. If both
  exist, select `subsystem:NAME` or `projection:NAME` explicitly.
- Source descriptions and availability statements are data, not filesystem paths
  to chase or instructions to execute. Scripts mentioned by a document are inert.

Validation checks the declared graph, source files and work register. Inspection
resolves the selected subsystem dependency closure (or the projection's selected
subsystems), shared core and an optional exact work item. It returns ordered
repository-relative source handles, byte counts and SHA-256 hashes, not every
document's text. Indexing the backing store is not injecting it all into a prompt.

The selection digest binds logical selection and declared source bytes. It must
not depend on absolute machine paths, timestamps, or runtime thread IDs. It is
not proof of provider-rendered prompt equivalence, current check validity, or a
transactional snapshot of a concurrently changing repository.

## Native availability is not inferred

The bootstrap is a `reviewable-design-context`, not a packaged native checkpoint.
It explicitly declares `native_payload_in_repository = false`. Inspection reports
the native binding as unbound; it must not suggest the Markdown handoff can stand
in for native memory. Explicit version-one native bundles are now read and
validated through confined paths. Their availability is reported separately from
destination binding, and their bytes never appear in inspection output.

Native launch now supplies destination compatibility checks and private bindings.
Explicit save captures a supported completed boundary and publishes a named
projection in the worker's checkout. See [native launch](native-launch.md) and
[bound workers and handoff](worktree-handoff.md) for the current commands and
limits. The earlier [native lifecycle record](native-recovery-lifecycle.md)
remains historical evidence for the runtime route.

## Safety and platform boundary

The resolver rejects unsupported versions, malformed manifests and JSONL,
ambiguous names, escaping paths, symlink components and nonregular source files.
Default limits are 1 MiB per ordinary file, 64 KiB per native manifest, 8 MiB per
native payload, 16 MiB aggregate input, 2,048 files, 4,096
discovery entries, dependency depth 64, and 2 MiB JSON output. Exceeding a limit
is an explicit error. These are filesystem/output budgets, not token estimates.

The confined reader uses descriptor-relative file access on Unix. Other
platforms must report unsupported confinement rather than silently use a weaker
reader. This limits execution portability, not the logical names or Git format.
Individual file observations do not constitute an atomic multi-file snapshot.

## Remaining workflow

Bound worktrees, record updates/three-way merging and explicit native save now
have [implemented commands](worktree-handoff.md). Broader adapter integration,
cross-account portability and semantic reconciliation remain separate work.
Work state and code travel together; conflicting histories still require an
explicit choice and historical verification is never promoted to current truth.

The [launch milestone plan](launch-plan.md) records the accepted Git export and
fresh/bootstrap behavior and completed acceptance gates. The implemented
[finish and resume](finish-resume.md) workflow adds status, recovery, completion
and opt-in advisory hooks.
