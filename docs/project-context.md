# Git-native context: the first executable boundary

Astral's project-context work starts with **read-only inspection**, before a
launcher creates worktrees, accesses native artifacts, or starts an agent. The
repository carries the shared context; runtime credentials, native payloads and
thread bindings remain private. The native proxy experiment is a separate layer.
See [the current integration receipt](project-context-verification.md) for tests
rerun on the merged implementation and the remaining launch boundary.

## Scope of the first implementation

AST-001 adds a bounded manifest validator and named-context resolver. It does not
launch Codex, execute scripts, fetch artifacts, change Git state, or convert a
readable handoff into native recovery. The v1 manifest contract is repository-local
and experimental, not a general standard for arbitrary agent checkpoints.

The implemented read-only interface is:

```sh
astral --root /path/to/repository context validate
astral --root /path/to/repository context list
astral --root /path/to/repository project project-context --inspect --work AST-001
astral --root /path/to/repository project git-native-context-bootstrap --inspect
```

These commands return JSON. Omitting `--inspect` from `project` must fail rather
than imply that an agent was launched. The root defaults to the current directory;
it is not an instruction to search other projects or the user's session archive.

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

These are argument inspection and invocation primitives. The project CLI does
**not** yet call the process builder. Native artifact staging, runtime compatibility
checks, proxy health/lifetime management, and branch/worktree binding remain
required before real project launch. The requested route is not a verified
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
- Work items are one object per JSONL line, with unique IDs, explicit dependencies
  and acceptance criteria. V1 is a snapshot format, not an append-only event log;
  duplicate IDs, missing references and dependency cycles are errors.
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
the native binding as unavailable/unbound; it must not suggest the Markdown
handoff can stand in for that missing checkpoint. In-repository native payload
loading is not supported by this first resolver.

Native artifact access, compatibility checks, private binding stores and lifecycle
launch are subsequent work. The [native lifecycle record](native-recovery-lifecycle.md)
describes a tested runtime route, not an artifact registry or general launcher.

## Safety and platform boundary

The resolver rejects unsupported versions, malformed manifests and JSONL,
ambiguous names, escaping paths, symlink components and nonregular source files.
Default limits are 1 MiB per file, 16 MiB aggregate input, 2,048 files, 4,096
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
