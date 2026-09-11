# Native launch milestones

Launch is implemented in bounded milestones. The current executable validates
context, inspects selected inputs, allocates work ID proposals, initializes new
indexes and launches fresh document contexts in Codex. It also validates typed
native bundles and reports their local availability. Native restoration and
managed proxy processes remain pending.
The earlier successful native lifecycle tests establish a feasible route, with
the version and protocol limits in [the lifecycle receipt](native-recovery-lifecycle.md).

## Accepted behavior

- `astral project` selects `project-context`; an explicit name selects another
  subsystem or projection. `--inspect` prints indented JSON without launching.
- Explicit native exports are committed through Git. Export writes a durable
  bundle; the operator controls when it is committed or shared. Runtime
  credentials, executable connections, permissions and machine/thread bindings
  are supplied by the destination runtime rather than copied from the bundle.
- Native import creates a new local thread from an immutable checkpoint window
  and its complete continuation. Reusing an active original thread is not the
  default. Saving a newer projection is an explicit operation.
- Direct Codex is the default; `--proxy` opts into managed Astral routing. A bundle
  requiring rebinding reports that requirement if the direct route cannot support
  it. The launcher must not silently enable a proxy or replace native state with
  a readable summary.
- User-supplied Codex arguments remain unchanged, including permission flags. If
  they change the effective model, route or workspace, compatibility is checked
  against that effective destination. Conflicts are errors, not silent rewrites.
- A referenced artifact that is missing, corrupt or incompatible is an explicit
  error. No historical test receipt becomes current verification after import.

When a context has **never** had a saved checkpoint, start a clearly labeled fresh
session from its selected documents. If `.astral/` is absent, the interactive
command offers an explicit initializer: a versioned prompt embedded in the
binary asks inference to inspect the repository and build a small, best-effort
index. An explicit initialization command supports noninteractive use without
waiting for a hidden prompt. Validate the resulting manifests and references;
record unknowns and suggested commands as unverified. This user-selected behavior
does not turn a missing referenced checkpoint into a fresh-session fallback.

## Milestones and acceptance

| Milestone | Work ID | Deliverable | Acceptance gate |
| --- | --- | --- | --- |
| 1. Fresh launch and initialization | `AST-33ydmjdkmx4k` | Launch a fresh context from selected documents through an owned Codex adapter. Offer initialization when `.astral/` is absent, using a versioned prompt embedded in the binary, and validate the generated index. | A disposable repository produces a valid, best-effort index; an existing context starts in the correct checkout with literal user arguments and current permissions; missing referenced native artifacts still fail. Neither the initializer nor its observations are misreported as native recovery or verified tests. |
| 2. Native bundle contract | `AST-httszq3svat1` | Versioned Git-trackable artifact and manifest: complete native item window, byte digest/size, format/protocol/model compatibility, source revision/input provenance, capture boundary and ancestry. Extend the resolver's typed availability reporting. | Synthetic round trips preserve every native field and ordering; missing/corrupt/unsupported/incomplete windows reject; no payload-supplied executable code or permissions are adopted. |
| 3. First usable native launch | `AST-7esjfvymym3v` | An owned Codex app-server adapter stages a new thread from a bundle and hands it to interactive Codex in the existing checkout. Add explicit proxy readiness/lifetime handling and local launch receipts. | On the supported route, a disposable native bundle imports, executes a real terminal command, passes independent continuity checks and cold-resumes. Current runtime restrictions remain effective; failures leave a diagnosable receipt and clean up only owned processes. |
| 4. Work and Git integration | `AST-nge5zwyyktzr` | `--work ID` binds a validated record to a branch/worktree; add bounded work-record creation/update and record-aware merge rules. | Dirty source work is preserved; repeated launches reuse the intended workspace without duplicate sessions; concurrent branches allocate distinct identities; divergent edits to one record surface as conflicts; references/graphs validate after merge. |
| 5. Save, hand off, continue | `AST-hqx0p9xedayb` | An explicit save operation captures a completed native boundary, writes an immutable Git-trackable bundle, and advances a named projection. Import from another checkout and continue. | Work → compact → capture → new import → cold resume → continue passes with native hashes preserved, no observation-mode detour and no plaintext substitution. Concurrent native histories remain distinguishable. |

The first milestone is complete with the runtime limits documented in
[fresh launch](fresh-launch.md) and current results in its
[verification receipt](fresh-launch-verification.md). The second milestone is
complete: the [native bundle contract](native-bundles.md) and its
[verification receipt](native-bundle-verification.md) keep artifact integrity
separate from destination runtime binding. The next milestone adds usable native restoration in
the existing checkout. Milestone four introduces automatic worktree creation.
Existing broad work items
retain their IDs and close only when their acceptance criteria are actually met.

## Expected command experience

Currently available:

```sh
astral project
astral project web -- --model gpt-6-astra
astral project --inspect
astral project web --inspect --work AST-3k9v4n6x2m7q
astral init
astral init --inspect
astral work id
```

Later milestones extend the established interface:

```sh
astral project that-really-hard-problem --proxy
astral project web --work AST-3k9v4n6x2m7q -- --model gpt-6-astra
```

`--work` currently adds the selected record to fresh context; milestone four adds
automatic branch/worktree binding to that same command.

The sample work ID is illustrative. `astral work id` proposes a random ID but
does not yet persist a record. Capture/export command spelling will be settled
with the bundle API; it is not an implemented command today.

On success the user enters the Codex session with the chosen context and working
directory. On failure the launcher reports the failed stage and artifact/runtime
compatibility information without claiming that a session was restored. Owned
proxy processes live as long as their launched session needs them; unrelated
running proxies and source sessions remain under their existing ownership.

## Merge and portability expectations

Short random work IDs address identity allocation. They do not make adjacent
JSONL edits automatically conflict-free. A three-way merge must distinguish
independent additions from concurrent changes/deletions to an existing record;
a simple concatenation/union merge driver is insufficient.

Opaque checkpoint bytes cannot be semantically merged by Git. Preserve immutable
artifacts from both branches, reconcile readable decisions/work records, then
choose a checkpoint or create a new reconciliation session. Do not invent a
combined native state by joining encrypted payloads.

Initial native compatibility is limited to the experimentally supported Codex
protocol/model route. Import across accounts, different providers or changed
native formats remains unproven and must not be advertised as working teammate
portability until tested. The milestone-five second-checkout test starts within
the supported identity scope; cross-account qualification remains separate work.
No deterministic output, cache-hit, token-saving or invoice-saving guarantee follows
from context portability.

The trial harness will inform an owned adapter, not be invoked as production
launch code: it depends on an external capsule's Python helper and hardcodes test
model/policy settings. A production bundle is data and cannot supply an importer
to execute. Unresolved inherited history must be resolved by supported APIs into
a complete native window or reported unavailable.
