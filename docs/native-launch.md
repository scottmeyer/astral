# Project launch and delegated workers

`astral project` launches the selected documents or native bundle in the current
checkout. The default selector is `project-context`. `--work ID` adds the exact
validated JSONL work record and binds a dedicated branch, worktree, and reusable
worker thread. See [bound workers and handoff](worktree-handoff.md) for that flow;
the receipt-based native launch below applies when `--work` is omitted.

## Run a worker with selected context

```sh
astral --root /path/to/worktree project web --work AST-3k9v4n6x2m7q \
  --non-interactive -- \
  -c 'sandbox_mode="read-only"' -c 'approval_policy="never"' \
  --json 'Review the selected task and report concrete findings. Do not edit files.'
```

This stages the selected context, closes the staging app-server, and invokes
`codex exec resume THREAD_ID` with the original caller arguments as a contiguous
suffix. It uses no shell and closes stdin. Supply an explicit bounded task.
Without `--non-interactive`, Astral invokes interactive `codex resume`.
Codex's `exec resume` and interactive `resume` accept different options; use the
options supported by the chosen command. In Codex 0.154.0, the configuration
overrides above work for headless sandbox and approval selection.

Astral options are recognized before the first literal `--`. Put Codex arguments
after it, especially if their values resemble Astral options. Astral does not
add permission bypasses. Fresh contexts launch directly. Native contexts on the
currently supported route require explicit `--proxy`:

```sh
astral project my-checkpoint --proxy
astral project my-checkpoint --proxy --non-interactive -- \
  --model gpt-6-astra --json 'Continue the selected work; begin by checking the checkout.'
```

## Native staging and cold resume

Native launch supports Codex **0.154.0**, `gpt-6-astra`, the built-in `openai`
provider and OpenAI Responses Lite within the same account. The bundle is
validated and read into memory before launch. Astral checks the installed
runtime version, the effective base URL/compression/provider configuration and
the thread's model, provider, workspace, durable storage and identity before
injecting its native window. It sends each raw native item directly, followed
by the current selected documents and work record as a user-context message.
Staging does not start inference or execute a tool.

The managed proxy binds an ephemeral loopback port and verifies its readiness.
Its upstream is fixed to `https://chatgpt.com/backend-api/codex`, with TLS
verification enabled. It runs in pass-through mode with native tool rebinding
and Astral rolling disabled. Current Codex supplies authentication, executors,
tool declarations, trusted instructions and permissions. The exported context
does not supply executable authority. An incompatible model, custom provider
override or conflicting route fails rather than sending the native payload to
another destination.

Astral prints a random local launch ID and the destination thread ID. Resume
with the same selector, work record and checkout:

```sh
astral project my-checkpoint --proxy --resume LAUNCH_ID
astral project my-checkpoint --proxy --resume LAUNCH_ID --non-interactive -- \
  --json 'Continue with the next bounded task.'
```

Resume starts a new owned proxy and reopens the recorded thread. It does not
reinject the original native bundle or selected documents. The receipt binds
the canonical workspace, project, selector, selected-input digest, work ID and
bundle manifest digest. Changed bindings fail explicitly. Start a new launch
when intentionally selecting changed context.

Receipts default to `~/.local/state/astral/launches`; the trusted host environment
may set an absolute `ASTRAL_LAUNCH_STATE_DIR` outside the workspace. They use
private directories/files and an exclusive advisory lock for each launch.
They contain identity, status and error codes, without transcript payloads,
credentials or raw arguments. Failures retain receipts for diagnosis. These
locks exclude cooperating Astral launchers; they cannot prevent someone from
opening the thread separately with raw Codex.

The proxy has its own task runtime. Normal exit and handled failure stop its
listener and active connections, including upgraded websockets. Unrelated
proxies and original/source threads are untouched. Killing the launcher process
also ends its in-process relay; abrupt termination can leave a stale receipt
status, but the OS releases its lock.

## Import and portability limits

The bundle reader preserves arbitrary supported native fields as inert data.
Codex's typed `thread/inject_items` API is narrower: it discards unknown fields
and ignores host-owned call metadata. Launch rejects those shapes with
`NATIVE_IMPORT_UNSUPPORTED` instead of silently stripping them. Numeric values
that typed JSON import would round are also rejected. Optional
null/default fields and destination-generated metadata may be normalized by
Codex. Exact artifact bytes and exact serialized raw items sent to the API do
not imply byte-identical destination rollout records.

A selection can refer to the same digest-pinned bundle more than once. Distinct
native histories produce `MULTIPLE_NATIVE_CONTEXTS`; Astral cannot combine
opaque histories by concatenating them. Missing referenced bundles remain
errors. There is no readable-context fallback for a required native checkpoint.

Fresh bound workers may opt into `--proxy`. Fresh receipt-based `--resume`
without `--work` is unsupported.
Profile/remote/provider selection, ephemeral staging, alternate state stores,
extra write roots and configuration/rules bypass flags that staging cannot
reproduce are rejected. Cross-account transfer, changing runtime inventories,
other Codex versions and arbitrary checkpoint formats are not qualified by this
launcher. Capture/export, automatic worktrees and record-aware Git merging
are described in [bound workers and handoff](worktree-handoff.md).

See the dated [verification receipt](native-launch-verification.md) for current
checks, live controls, scanner findings and untested boundaries.

The launcher enables a parent agent to start an Astral worker with selected
context and a task. It does not automatically intercept the host's subagent
tool, save the worker's context, merge its edits or reconcile competing native
histories automatically. Parents explicitly invoke Astral launch/save/work commands.
