# Git-native context: the first executable boundary

Fresh launch and repository initialization are now available; see
[fresh launch](fresh-launch.md). The resolver and inspection contract below
remain applicable independently of launching.

Astral's project-context work includes **read-only inspection** of documents and
explicit [native bundles](native-bundles.md), separate from runtime launch. The
repository carries shared context and explicitly exported native data; runtime
credentials and thread bindings remain private. The native proxy experiment is a
separate layer.
See [the current integration receipt](project-context-verification.md) for tests
rerun on the merged implementation and the remaining launch boundary.
The subsequent [unified CLI receipt](cli-verification.md) records the binary
rename, default context and repaired development scanner.

## Scope of the first implementation

AST-001 adds a bounded manifest validator and named-context resolver. It does not
launch Codex, execute scripts, fetch artifacts, change Git state, or convert a
readable handoff into native recovery. The v1 manifest contract is repository-local
and experimental, not a general standard for arbitrary agent checkpoints.

The implemented read-only interface is:

```sh
astral --root /path/to/repository context validate
astral --root /path/to/repository context list
astral --root /path/to/repository project --inspect
astral --root /path/to/repository project project-context --inspect --work AST-001
astral --root /path/to/repository project git-native-context-bootstrap --inspect
```

These commands return JSON; `--inspect` uses two-space indentation. The final
output limit includes formatting and its trailing newline. Omitting `--inspect`
starts a fresh session for a supported readable context; unsupported native
contexts fail explicitly.
The root defaults to the current directory;
it is not an instruction to search other projects or the user's session archive.
Without an explicit context, `astral project` selects `project-context`. An explicit
name must be the first argument after `project`; after options or `--` begin,
positional arguments belong to Codex. For example, `astral project --inspect --
"Continue the work"` previews a prompt for the default context. A missing default
context is reported as unavailable, never silently replaced with another context.

## Launcher argument contract

The requested route defaults to direct Codex. `--proxy` opts into Astral routing.
Inspection can show that request and the Codex arguments without starting either
process:

```sh
astral project project-context --inspect --model gpt-6-astra "Continue the work"
astral project project-context --inspect --proxy -- --model gpt-6-astra
astral project project-context --inspect -- --dangerously-bypass-approvals-and-sandbox
```

Before the first literal `--`, Astral consumes `--root`, `--work`, `--inspect`, and
`--proxy`; remaining arguments retain their original order. After the separator,
everything belongs to Codex. Use it when a Codex option name or an option's value
could resemble an Astral option. Astral does not need to recognize every Codex
flag. An explicitly supplied permission flag remains unchanged; none is added
by default. The argument is passed as written, with Codex responsible for its
validity and behavior.

The process builder uses raw OS argument strings and `std::process::Command`,
never a shell. Inspection requires UTF-8 for an exact JSON preview; it fails
instead of displaying a lossy replacement. The low-level process API preserves
non-UTF-8 arguments on platforms that support them. Limits are 1,024 arguments,
64 KiB per argument and 256 KiB aggregate argument bytes; final inspection output
also remains within the resolver's output budget.

Fresh project launch uses these argument primitives with the
[documented staging limits](fresh-launch.md). Native artifact staging, destination
compatibility checks, proxy health/lifetime management, and branch/worktree
binding remain subsequent work. The requested route is not a verified
effective route: explicit Codex configuration overrides may change routing.
A native checkpoint requiring tool rebinding must report that requirement before
direct launch; it must not trigger an implicit proxy or plaintext substitution.

## Manifest and selection contract

- `.astral/project.toml` identifies the project and explicitly registers subsystem
  directories. Each registry key matches the subsystem manifest's logical ID.
- Shared core consists of `ARCHITECTURE.md`, `RUN.md`, and `TEST.md` beneath the
  configured core directory.
- `subsystem.toml` selects its README, rules, decisions and optional subsystem
  dependencies. Dependencies are explicit and acyclic, never model-inferred.
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
  creating/reserving a record. Existing IDs remain stable; future writes and
  merges must still reject duplicates. Do not allocate by incrementing a counter.
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

Native capture, destination compatibility checks, private binding stores and
lifecycle launch are subsequent work. The [native lifecycle record](native-recovery-lifecycle.md)
describes a tested runtime route, not an artifact registry or general launcher.

## Safety and platform boundary

The resolver rejects unsupported versions, malformed manifests and JSONL,
ambiguous names, escaping paths, symlink components and nonregular source files.
Default limits are 1 MiB per ordinary file, 64 KiB per native manifest, 8 MiB per
native payload, 16 MiB aggregate input, 2,048 files, 4,096
discovery entries, dependency depth 64, and 2 MiB JSON output. Exceeding a limit
is an explicit error. These are filesystem/output budgets, not token estimates.

The first implementation uses descriptor-relative file access on Unix. Other
platforms must report unsupported confinement rather than silently use a weaker
reader. This limits v0 execution portability, not the logical names or Git format.
Individual file observations do not constitute an atomic multi-file snapshot.

## Remaining workflow

AST-002 defines work-item updates, reconciliation and adapter boundaries. AST-003
binds branches/worktrees without overwriting local changes. AST-004 adds explicit
native artifact composition/export/resume. AST-009 combines these in the guarded
launcher. Work state and code will travel together, but semantic conflicts and
historical verification will never be silently promoted to current truth.

The [launch milestone plan](launch-plan.md) records the accepted Git export and
fresh/bootstrap behavior, concrete acceptance gates, and the next work IDs.
