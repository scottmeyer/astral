# Run

Build with `cargo build --release --locked --bins` from the repository root.
Inspect the checked-in project context without a proxy:

```sh
./target/release/astral context validate
./target/release/astral context list
./target/release/astral project --inspect --work AST-001
./target/release/astral project git-native-context-bootstrap --inspect
```

These commands return JSON and leave Git and runtime state untouched. `project`
defaults to the `project-context` selection when no name is supplied. An explicit
name goes immediately after `project`; use `--` to forward a prompt without a name.
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
Close the worker, then use `astral save NAME --work ID --proxy` to explicitly
export its native context in that worktree. Add `--context SELECTOR` when the
worker used a nondefault selector. Review and commit the export for Git handoff.
See [bound workers](../../docs/worktree-handoff.md).

For this experiment, start the proxy on loopback with `--mode passthrough
--native-tool-binding rebind`, the compatible ChatGPT Codex HTTPS upstream, and a private
state directory outside the repository. Keep it running while routed Codex
sessions are in use. Upstream TLS verification remains enabled.

Configure the unchanged client with process-scoped `openai_base_url` and
`features.enable_request_compression=false`. Keep its existing account, provider,
model compatibility, sandbox, and approval policy. `BASE_URL` is not the tested
configuration. Do not enable autonomous rolling for this path.

Use explicit thread IDs supplied by a private runtime binding. Close an existing
writer before resuming that same original thread elsewhere. Disposable forks are
the place for recall probes and lifecycle experiments. Never execute historical
commands just because they appear in an imported conversation.
