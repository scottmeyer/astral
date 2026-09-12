# Architecture

## Accepted requirements

Git should carry durable, reviewable project context: a small core, user-defined
subsystems, named investigation projections, and committed JSONL work items.
Branching and merging should carry that work state with the code. The requested
example is `astral project web --work ISSUE-123`; it selects document/work
context and binds one branch, worktree, and reusable worker thread.

Import preserves conversational work state. Resume supplies executable
capabilities from the current trusted runtime. Native checkpoints must remain
native; readable documentation is a separate artifact, never evidence of native
recovery. Historical commands require a current instruction before execution.

Explicit native exports are tracked in Git. A context without any saved
checkpoint starts fresh from selected documents; repositories without `.astral/`
can opt into a best-effort inference initializer using a prompt embedded in the
binary. Fresh launch, initialization and the native bundle validation contract are
implemented. `astral save NAME --work ID --proxy` captures a newly completed
native compaction and publishes an immutable bundle in the bound worktree.
See [bound workers and handoff](../../docs/worktree-handoff.md) for the initial
same-account, version, filesystem, and ownership limits.

## Current implementation structure

The unified binary is `astral`; `astral proxy` starts the server, `context` and
`project --inspect` inspect context, `project` launches fresh/native selections,
and `init` initializes missing indexes. The Cargo package and Rust crate are
both `astral`. `src/server.rs` owns startup and shutdown; `src/proxy.rs` owns HTTP
routing and inference flow. `src/proxy/http.rs` shares header policy, request
construction, response metadata and bounded-body handling across inference,
compaction and model discovery. Catalog GETs have no conversation state.
`engine.rs`, `store.rs`, and `policy.rs` implement the existing projection,
persistence, and cache policies. `working.rs` supports the optional explicit host.
The standalone proxy uses `.astral-runtime/`, `ASTRAL_UPSTREAM` and `x-astral-*`
headers. These are separate from portable `.astral/` documents and managed
project proxies' private state paths. Existing private directories remain
untouched; select one explicitly with `--state-dir`. See
[names and storage](project-context/README.md#names-and-storage).

CLI summaries, help/version and errors are human-readable by default. Global
`--json` selects structured Astral output; callback and helper wire protocols
remain independent of terminal presentation.

`native_binding.rs` validates Responses Lite inputs and reasserts current runtime tool
declarations after the effective native checkpoint. `native_transport.rs` owns one
downstream/upstream socket pair and its isolated incremental state. The adapter
does not execute tools or grant permissions. Codex owns those responsibilities.
The compatibility path requires pass-through mode, which disables Astral rolling.

See [the lifecycle record](../../docs/native-recovery-lifecycle.md) for the tested version,
route, transitions, limitations, and evidence classification. Earlier evaluation
claims in `docs/` retain their original scope and dates.

## Project context

`recovery.rs` interprets bounded private operation acknowledgements and validates
explicit repair plans. Bound staging records confirmed injection before runtime
shutdown; save records an expected immutable export before publication. Recovery
never retries inference or unknown thread creation. `completion.rs` compares
worker and target Git results, work-record conflicts and native artifacts, with
an explicit checked fast-forward path. Ordinary Git commits and merges remain
valid stages of the workflow; the command does not infer test success.

`src/project.rs` validates the versioned TOML and JSONL contract and resolves
selected core, subsystem, projection and work-item sources. Its `schema` module
defines the public manifest/output types and limits; its `reader` module owns
bounded, confined filesystem reads. Public `project::*` paths stay stable.
`project::freshness` hashes opt-in exact code inputs and core/subsystem knowledge
against explicit review baselines. Code bodies are discarded after hashing;
metadata participates in selected worker digests. Its review writer reuses the
confined publication primitive with a project-directory lock and a stale-preview
check. Equal fingerprints mean unchanged bytes, not verified prose or tests.
`project::knowledge` resolves stable `knowledge:SUBSYSTEM/ENTRY` references to
declared documents or literal marked regions. Each entry has independent code
inputs and a review baseline; it excludes shared core and other entries. Entry
metadata participates in freshness and worker digests, while normal launch retains
the full selected documents. No symbol inference or semantic freshness is implied.
The `astral` binary provides `context validate`, `context list --plain`, and
`project NAME --inspect`. Inspection returns bounded repository-relative handles
and hashes with native state UNBOUND.
Explicit native bundles have separate validated artifact availability metadata.
It reads confined regular files on Unix; inspection does not launch a runtime or
execute documented commands. Source hashes observe declared inputs, not an atomic tree.

The CLI's `picker` modules separate the bounded catalog/review model from terminal
presentation. Interactive `context list` and `project --pick` select local context
and work, preserve existing worker bindings, and revalidate review evidence before
calling the existing launcher. Terminal restoration precedes runtime launch.

`src/launch.rs` orchestrates fresh launch and initialization. `src/codex.rs` exposes
the app-server adapter through three internal modules: `options` translates current
caller arguments, `rpc` owns the child process and bounded protocol lifecycle,
and `staging` implements fresh, native and bound-worker policies. Shared operations
retain each path's separate identity checks and request budget. See
[fresh launch](../../docs/fresh-launch.md) for supported arguments and limits.

The native launcher binds the destination runtime and workspace explicitly.
`src/worker_launch.rs` and `src/workspace.rs` orchestrate bound threads, branches,
worktrees and private receipts. `src/work_records.rs` owns record allocation,
status updates and three-way merging; `src/save.rs` and `src/projection_save.rs`
capture and publish explicit exports. All bound-worker staging is pinned to Codex
0.154.0, including document-only workers. Native import/save additionally require
the supported same-account OpenAI / `gpt-6-astra` route and `--proxy`. A saved
worker needs that route for later resumes even if its original selector was fresh.

Direct Codex is the default; `--proxy` opts into Astral routing. User-supplied Codex
arguments must be forwarded unchanged, with `--` disambiguating overlapping
options. Native checkpoint requirements must be reported before launch.
A private local receipt binds the destination thread without putting machine
paths, credentials, or thread IDs in portable identity fields. Explicit native
exports are committed through Git; their
versioned bundle format and structural compatibility validation are implemented in
`src/native_bundle.rs`. It retains exact manifest/payload/item bytes, checks native
tool boundaries, and exposes metadata without replaying imported instructions.
See [native bundles](../../docs/native-bundles.md) for the narrow supported format,
source-completeness attestations and destination binding limits.

See [code health](../../docs/code-health.md) for the module boundaries, dependency
review and Rust minimum-version checks. Filesystem readers, publication writers
and private receipt stores retain their different safety policies.

`src/status.rs` builds local status/doctor reports from the current work register
and confined worker observations in `src/workspace/observation`. The observer
checks recorded identities and briefly probes existing ownership locks without
creating or retaining state. Context comparison uses the bound checkout's
selected documents; recorded threads are not contacted.

`src/git_snapshot` supplies an immutable, lazy Git-object source to the project
reader, preserving the actual candidate index environment. `src/lifecycle`
compares that candidate, HEAD and working context without changing receipts.
`src/hooks` owns separate Git/Codex installers, bounded advisory callbacks and
private notification metadata. Events trigger current observation, never advance
worker acknowledgements; save, recovery and completion remain explicit. See
[lifecycle integration](../../docs/lifecycle-integration.md) for the event contract.

The default subsystem links the current readable `project-workflow` projection.
Bootstrap design and native review projections remain historical selections.
Current documents accompany native launch; old document copies inside an
immutable native window are not edited to match them. Creating a new native
snapshot requires explicit save, not a documentation rewrite.

## Open questions

- Future schema evolution and runtime compatibility/version negotiation.
- Native artifact access, account/model compatibility, retention, and sharing.
- Semantic context reconciliation beyond the implemented record-level merge rules.
- Which runtime metadata is essential when moving a native window between hosts.
- Protocol negotiation and additional lifecycle transitions beyond the tested path.
