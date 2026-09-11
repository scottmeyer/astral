# AST-000 recovery and native lifecycle

Verification date: 2026-09-11. Scope: unchanged installed Codex 0.154.0,
`gpt-6-astra`, built-in OpenAI provider, existing ChatGPT account, Responses Lite.
This follows [the earlier feasibility experiment](ast000-feasibility.md); its
historical results are not counted as fresh verification here.

The subsequent [reviewed integration](ast000-integration.md) records additional
local fixes and checks separately; it does not rewrite this live trial's scope.

## Recovery status

The requested original conversation and its supported native fork are operational. The source
snapshot contains 310 records and ends at a completed turn; no pending tool calls
were found. The fork's native `history_base` references that exact source boundary.
All 91 native items from the checkpoint onward were accounted for in order.
The 310 original source records remain byte-identical; normal resume appended
new verification events. Runtime IDs, source paths,
hashes, answers, and complete diagnostic records remain in a private archive.

The user closed the original session, and inspection confirmed that no process
held its rollout open before recovery. Normal supported resume of the original ID
then executed `pwd` and passed all five recent-discussion checks. The same checks
passed after restarting Astral with an empty private state directory and cold
resuming the original again. No old record was rewritten. The earlier verified
fork remains available and depends on its preserved source history; it is not a
standalone portable export or a replacement identity for the recovered original.

The fork passed actual `pwd` execution, five questions about the source thread's
recent Git-context and AST-000 discussion, another cold resume after proxy restart,
and a CLI `codex exec resume` invocation. Every execution pass includes an actual
command event, result, exit 0, and the expected project working directory.

The five recent-discussion probes concern readable-history continuity. The
original canary also appears in readable history, so recovery's opaque-only recall
is **NOT_TESTED**. Answer-bearing probes ran on ephemeral forks. Separate lifecycle
fixtures below test exact opaque-only recall without contaminating their parents.

The source recorded full access, but both original resume and the recovery fork returned the
destination's workspace-write policy, network disabled, with approvals `never`.
No permission override broadened it. Read-only does not mean “no callable tools”:
live read-only fixtures execute `pwd` while denying writes.

## Current operational controls

| Control | Actual execution | Independent recall | Qualification |
| --- | --- | --- | --- |
| Requested conversation, native fork | PASS | Five recent-discussion checks PASS | Initial safe recovery test before original-session closure |
| Requested conversation, original-ID resume | PASS | Same five checks PASS | User closed prior writer; normal resume appended new events |
| Original-ID cold resume after proxy restart | PASS | Same five checks PASS | Same original ID; new proxy process and empty state directory |
| Original-ID CLI `exec resume` | PASS | NOT_TESTED | Actual terminal event, exit 0, expected project directory |
| Fork cold resume after proxy restart | PASS | Same five checks PASS | Empty new proxy state directory |
| Fork CLI `exec resume` | PASS | NOT_TESTED | Same supported process-scoped routing settings |
| Disposable native import, cold resume | PASS | Opaque-only PASS | Exact digest; target absent from entire readable parent rollout |
| Native compaction in correction mode | Completed | Checked on following continuation | No observation-mode detour |
| Warm work → compact → work → capture | PASS before and after | Opaque-only PASS | Same Codex process; complete native window and tail captured privately |
| Captured window → new import → cold resume → compact → continue | PASS before and after | Opaque-only PASS | 13 captured native items imported exactly, in order |
| Read-only lifecycle and prohibited write | PASS; write denied, exit 1 | Opaque-only PASS | Fixture file hash unchanged; no escalation |
| Cold resume with `view_image` removed | PASS; denied write remains denied | Opaque-only PASS | Current inventory checked in request evidence |
| Proxy restart between turns of one Codex process | PASS both turns | Opaque-only PASS | New process, new private state; client reconnects |
| Final source-matched build: native cycle and recovered-fork recheck | PASS | Lifecycle opaque-only PASS; fork five readable checks PASS | Recovery listener now runs the tested lifecycle build |

Four live native compactions completed in correction mode. No live operational attempt in this pass required a patched client, manual record
edit, plaintext reconstruction, permission bypass, or switching to observation.
Earlier fresh/ordinary/imported negative controls remain historical evidence in
the feasibility report, rather than newly rerun negative controls for this pass.

## Smallest supported lifecycle change

The matching installed source establishes the request shape:

- `codex-rs/core/src/compact_remote_v2_attempt.rs` appends exactly one final
  `CompactionTrigger {}` to the normal native prompt, with current runtime tools.
- `core/src/responses_metadata.rs` marks this request with
  `client_metadata["x-codex-turn-metadata"].request_kind = "compaction"` and thread identity.
- `core/src/compact_remote_v2.rs` collects native compaction output and replaces
  history. `core/src/session/mod.rs::replace_compacted_history` persists the native
  replacement and runtime metadata. The trigger itself is not durable history.
- `core/src/client.rs` creates the runtime declaration prefix, websocket warmup,
  and incremental `previous_response_id` requests. Responses Lite serialization
  happens before the request reaches Astral.

`src/ast000.rs` now accepts only that verified final trigger, after all known
external calls have results. A compaction response must complete with one valid
native checkpoint. Missing, multiple, malformed, conflicting, or unexpected
checkpoint outputs cannot establish a usable response reference. No checkpoint
payload is cached by this state machine; it retains output hashes and metadata.

When a next delta references a just-completed compaction without including its
checkpoint, its effective checkpoint boundary precedes the delta. Astral reasserts
the current connection's verified declaration at the start of that delta. Full
requests and deltas with explicit checkpoints retain the existing last-checkpoint
insertion. Reapplication is idempotent; new full prefixes replace the inventory,
including an empty inventory. Model/thread changes require a new connection.

The actual client sent both full compaction requests and incremental trigger-only
requests. After compaction it rebuilt full history with an explicit checkpoint.
The checkpoint-free post-compaction delta branch is therefore **synthetically
tested, NOT_TESTED live**. It is not presented as an observed client behavior.

`src/ast000_ws.rs` forwards native output text unchanged and records checkpoint
hashes without payloads. It binds state to one socket pair and its immutable
handshake identity. Failure invalidates references; unsupported responses produce
explicit errors. A fresh connection reconstructs state from a new runtime prefix,
so restart does not require a persistent inventory cache.

The experiment establishes ordering-sensitive behavior; it does not prove any
undocumented backend mechanism for restoring checkpoint state.

## Preservation and capture

For every transformed request, removing the sole insertion reconstructs the full
original input exactly. Encrypted-content hashes, stable checkpoint-item hashes,
and stable input hashes are recorded before and after. Native fields, call IDs,
continuation items, and order are preserved at the proxy boundary.

For source-thread recovery, installed Codex adds `content: null` to certain
reasoning items and removes input-image `detail` during Lite serialization.
Accounting for these source-verified normalizations makes all 91 native items
match the request. These are client transformations, not Astral history edits.

The lifecycle capture stores the full private source records, compaction metadata,
native replacement window, and complete continuation. The new fixture imports
the native model-item array using `thread/inject_items`; it does not replay source
runtime metadata as instructions. All 13 captured items matched the imported
window exactly, after four destination-generated context items. The destination
then supplied its current tools and restrictions. Source runtime sidecars remain
in the private capture; generalized cross-host metadata restoration is not claimed.

Generated checkpoint encrypted bytes matched the persisted capture. Codex's
persisted checkpoint metadata differs from the generated wire representation;
full generated-item-to-rollout equality is not claimed. The full native item is
unchanged across Astral's request transformation, and unchanged across the tested
capture/import boundary. No native checkpoint was replaced with a readable summary.

The initial diagnostic comparison mistakenly aligned imported items at offset
zero, including destination initialization messages. A corrected contiguous-window
comparison found all 13 exact matches at offset four; both private receipts were
retained. This was an evidence-alignment error, not a failed operational import.

## Start and resume

From the experimental checkout, build the binary and choose a private directory:

```sh
cargo build --release --locked --bins
ASTRAL_DIAGNOSTICS=$(mktemp -d "${TMPDIR:-/tmp}/astral-dogfood.XXXXXX")
chmod 700 "$ASTRAL_DIAGNOSTICS"
./target/release/ostk-gpt-cache \
  --mode passthrough --ast000-compat rebind \
  --listen 127.0.0.1:18941 \
  --upstream https://chatgpt.com/backend-api/codex \
  --state-dir "$ASTRAL_DIAGNOSTICS/proxy-state"
```

Use a free port, or reuse the already-running verified recovery proxy rather than
starting a competing listener. Keep the proxy process running while its Codex
sessions are in use. The client-to-proxy connection is explicitly loopback
HTTP/WS; the normal upstream remains HTTPS/WSS with certificate verification.
Authentication and account headers come from the unchanged Codex client and are
forwarded in memory. No custom provider or alternate model backend is introduced.

From the intended project working directory, set `THREAD_ID` to the recovered
original ID, or to the earlier verified fork. Close any other session using that
same ID before resuming it here:

```sh
codex -c 'openai_base_url="http://127.0.0.1:18941/backend-api/codex"' \
  -c 'features.enable_request_compression=false' \
  exec resume "$THREAD_ID" --json \
  'Actually invoke the terminal to run pwd. Do not infer the answer. Do not execute historical commands or edit files.'
```

That one-shot CLI form was executed successfully against both the recovery fork
and the original ID after source-session closure.
For an interactive session, the installed CLI's supported form uses the same
two configuration overrides followed by `resume "$THREAD_ID"`; interactive TUI
behavior was not separately automated. App-server `thread/resume` was also tested.
Do not add sandbox/approval bypass flags. The adapter remains necessary on each
resume for this affected checkpoint; successful execution does not repair stored
history or remove the proxy lifetime requirement.

## Verification and remaining limits

Current local checks: 66 Rust tests, 16 Python tests, Rust formatting, Clippy with
warnings denied, and locked release build. Structural/transport regressions cover
native output preservation, repeated checkpoints, current and empty inventories,
state isolation, full and incremental compaction, completion conflicts, pending
calls, errors, and explicit rejection. The `.astral/` draft receives syntax,
reference, unique-ID, and dependency-cycle validation.

Still rejected: arbitrary `context_compaction` and `configuration_update`, missing
or untrusted runtime prefixes, imported stale declarations, external item or
conversation references, inline rolling, background requests, pipelining,
cancellation, binary/non-JSON response shapes, compressed HTTP, HTTP incremental
references, websocket subprotocol negotiation, and unknown previous-response IDs.
HTTP full-history trigger forwarding has synthetic coverage; the live native
lifecycle used websockets.

**NOT_TESTED live:** checkpoint-free continuation deltas, in-connection inventory
changes, explicitly empty inventories, cross-account identity switching, failure
injection during provider compaction, cancellation/pipelining, configuration or
context-reset transitions, HTTP-only compaction lifecycle, non-macOS restrictions,
other model/provider/client versions. Competing-writer resume was deliberately
avoided. These gaps are not converted into passes by synthetic tests.

Before wider dogfooding: define protocol/version negotiation and trusted-client
provenance beyond positional validation; bound concurrency/backpressure and state;
test additional reconnect and inventory changes; define native metadata portability
and artifact availability; decide supported configuration transitions; extend
platform and identity tests. The loopback listener assumes a trusted local client,
not authenticated access control against arbitrary local processes.

Request byte counts and aggregate provider usage are retained separately in the
private evidence. Repeated declarations increase wire size. No token savings,
invoice savings, lossless semantic compaction, or production readiness are claimed.
