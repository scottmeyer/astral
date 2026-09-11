# Run

Build with `cargo build --release --locked --bins` from the repository root.
Inspect the checked-in project context without a proxy:

```sh
./target/release/astral context validate
./target/release/astral context list --plain
./target/release/astral project --inspect
./target/release/astral project projection:project-workflow --inspect
```

These commands print readable summaries and leave Git and runtime state untouched.
Add global `--json` for structured output, such as `astral --json context list`.
Help/version are plain text by default; command errors go to stderr. `project`
defaults to the `project-context` selection when no name is supplied. An explicit
name goes immediately after `project`; use `--` to forward a prompt without a name.
For `project` and `init`, `--json` before that separator belongs to Astral; after
it, `--json` belongs to Codex. Child process streams keep their own format.
In a terminal, `astral context list` and `astral project --pick` open searchable
context/work menus and a launch review. Type to filter, use arrows and Enter,
Escape to go back, and Ctrl-C to cancel. Review details wrap and page with
Page Up/Page Down. Existing workers preserve their recorded context/worktree;
native history offers an explicit launch/resume action with `--proxy`. Browsing
does not start a runtime. Plain, JSON and redirected lists remain reports.
`astral proxy` runs the server. Bare `astral` and direct proxy flags also start it
for compatibility with existing runners. `astral project` launches a fresh Codex
session from selected documents in the current checkout. `astral init` starts
best-effort repository indexing; `astral init --non-interactive` uses `codex exec`.
See [fresh launch](../../docs/fresh-launch.md) for argument and platform limits.
Explicit [native bundles](../../docs/native-bundles.md) can be validated with
`context validate` and inspected with `project NAME --inspect`.
`astral project NAME --proxy` stages a native bundle; `--resume LAUNCH_ID` cold-resumes
its local thread with a new owned proxy. `--non-interactive -- ...` starts a worker
with selected context and an explicit task. See [native launch](../../docs/native-launch.md)
for supported versions, receipt storage, argument examples and remaining limits.

`astral work create TITLE --acceptance CRITERION` persists a random-ID record.
`astral project --work ID` creates or resumes its branch, worktree, and thread;
commit context changes first (the work-record file is carried automatically).
All bound workers currently require Codex 0.154.0. Native save/import additionally
require the same-account OpenAI / `gpt-6-astra` route. Without `--work`, repeating
a fresh launch starts another thread in the invoking checkout.
Close the worker, then use `astral save NAME --work ID --proxy` to explicitly
export its native context in that worktree. Add `--context SELECTOR` when the
worker used a nondefault selector. After save, repeat the original
`astral project SELECTOR --work ID` with `--proxy` to resume that worker.
Review and commit the export for Git handoff.
See [bound workers](../../docs/worktree-handoff.md).

`astral status` shows a page of work records and local worker bindings;
`astral status --work ID --json` selects one record with formatted JSON.
`astral doctor --work ID` reports local blockers and exits 1 when that observation
needs attention. These commands do not start Codex, repair receipts or verify
recorded threads. Use `--offset N --limit N` for further pages. See
[finish and resume](../../docs/finish-resume.md) for workflow limits.

`astral recover --work ID` previews repairs backed by retained operation evidence;
`--apply PLAN_SHA256` explicitly applies an unchanged plan under worker ownership.
Without `--work`, recovery inventories retained private bindings, including IDs
missing from the current work register. See [recovery](../../docs/worker-recovery.md).
`astral finish --work ID --into main` previews branch integration and required
review; its explicit apply supports checked fast-forwards. Ordinary commits and
merges remain supported. See [branch completion](../../docs/branch-completion.md).

`astral lifecycle check --scope index` validates the exact staged context;
the default scope is `worktree`, with committed/staged/working comparisons in
every report. `astral hooks install git` and `astral hooks install codex` show a
plan and ask for confirmation in a terminal. Use `--yes` for unattended application
or `--dry-run` for a preview; add `--json` for scripts. `--apply PLAN_SHA256` remains available for an
explicitly separate review; interactive setup retains and checks that hash itself.
`hooks uninstall TARGET` uses the same choices. Git shims are retained but disabled;
Codex removal preserves other groups and previous bytes. `astral commit -- -am
'Update context'` forwards to real Git. See [lifecycle integration](../../docs/lifecycle-integration.md)
for hook-manager composition, Codex trust, deadlines and reconciliation.

For project launch/save, `--proxy` owns loopback routing, readiness, supported
Codex overrides and shutdown. Do not manually start a recovery proxy for ordinary
`astral project ... --proxy` use. That managed route uses native tool rebinding
with Astral rolling disabled. A fresh launch without `--work` does not support
`--proxy`; use a bound worker when that route is needed.

`astral proxy` separately runs the standalone Responses server. Its default
state path is `.astral-runtime/`; use `--state-dir` to keep an existing private
location. Astral does not migrate or delete old directories. Set `ASTRAL_UPSTREAM`
for an environment upstream override; client controls use `x-astral-*` headers.
See the [proxy guide](../../docs/responses-proxy.md) for client identity headers,
roll policies and accounting. `.astral/` is the portable project context, not
the standalone proxy's runtime store.

`astral project` selects current document guidance. The historical bootstrap and
native worktree review are explicit selections shown by `context list`; their
old commands and check results are not current work instructions. Close competing
writers before resume/save and rerun relevant checks against the current checkout.
