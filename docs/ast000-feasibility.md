# AST-000: native checkpoint tool rebinding experiment

Verdict: **feasible on the tested Codex 0.154.0 OpenAI Responses Lite route**, with the protocol limitations below. This is an opt-in experiment, not the `astral project` launcher or a general solution for every checkpoint protocol.

An unchanged installed Codex client imported native checkpoints through its app-server API, cold-resumed them through Astral, executed `pwd`, recovered an exact opaque-context canary, and enforced a read-only sandbox against a fixture write. No installed client, authentication, original session record, or portable checkpoint was patched. Astral rolling was disabled in every operational control.

## Supported configuration actually exercised

The [official advanced configuration documentation](https://learn.chatgpt.com/docs/config-file/config-advanced) documents `openai_base_url` as an override for the built-in OpenAI provider. The installed CLI accepts process-scoped `-c` overrides. No `BASE_URL` environment variable or custom provider was assumed.

```sh
ostk-gpt-cache --mode passthrough --ast000-compat rebind \
  --listen 127.0.0.1:18933 \
  --upstream https://chatgpt.com/backend-api/codex \
  --state-dir /private/tmp/<private-experiment>/proxy-state

codex app-server --listen stdio:// \
  -c 'openai_base_url="http://127.0.0.1:18933/backend-api/codex"' \
  -c 'features.enable_request_compression=false'
```

Thread start/resume parameters were `model=gpt-6-astra`, `modelProvider=openai`, `approvalPolicy=never`, `sandbox=workspace-write` (or `read-only` for the restriction control), and a disposable workspace. Turns requested `effort=xhigh`. Existing Codex account authentication was forwarded in memory, with the account/provider/model unchanged. Upstream TLS certificate verification remained enabled. Only the loopback client-to-proxy connection used HTTP/WS.

Codex used websocket GET `/backend-api/codex/responses`, then JSON `response.create` frames. Its native protocol remained Responses Lite. The pre-existing Astral build had no websocket GET handler (405), so its initial pass-through controls fell back to HTTP POST on that same route. Those fallback controls are recorded separately. The matched comparison below uses Astral's websocket relay in all three modes: `observe`, `repeat-before`, and `rebind`.

Request compression is disabled through a supported client feature so full HTTP fallback can be transformed. Compression does not affect the JSON websocket frames. Compressed HTTP is explicitly unsupported in correction mode; observation mode reports it and forwards it unchanged.

## Capability boundary and minimum correction

The inspected matching Codex source is `codex-rs/core/src/client.rs`:

- `build_responses_request`, around lines 900–937, serializes runtime tools as `additional_tools`, gives the item an `at_` ID, and prepends it and the base developer instructions to the conversation input.
- The websocket code, around lines 1833–1908, supports incremental input linked by `previous_response_id`. Warmup, around lines 1976–2004, sends the prefix with `generate=false` and waits for completion. The next frame can contain the checkpoint without repeating the prefix.
- `codex-rs/model-provider-info/src/lib.rs` supplies the default ChatGPT Codex upstream and websocket capability; `core/src/config/mod.rs`, around line 3725, applies the configured OpenAI base URL.

Astral sees the relevant native JSON after this serialization and before upstream inference. It can inspect item types and the current runtime prefix without decrypting the checkpoint. In the observed warmup path, the request sequence is:

```text
warmup:       [current additional_tools, runtime developer message]
continuation: [environment/history, checkpoint, readable tail, new user input]
corrected:    [environment/history, checkpoint, current additional_tools, tail, input]
```

The adapter copies only the verified positional prefix from this locally configured Codex client, removes the copied item's already-used wire ID, and inserts it immediately after the last checkpoint in the request. Original items are not edited or reordered. Removing the insertion reconstructs the complete original input exactly. Identical declarations already in the intended position are left unchanged. Both correction and negative-control insertion are idempotent.

For incremental frames, state belongs to one downstream/upstream websocket pair, its immutable handshake identity, model, and thread metadata. A delta must reference that connection's last completed response. A new connection requires a full prefix; it cannot borrow another connection's inventory. Each new full prefix replaces the inventory, including an empty inventory. Stale or arbitrary imported tool declarations are rejected. HTTP correction is stateless and requires full input.

The evidence supports an **ordering-sensitive capability boundary**: runtime declarations reach the proxy, tools execute in fresh/resumed controls, pre-checkpoint repetition fails, and post-checkpoint repetition succeeds. It does not establish how an undocumented backend restores or resets its internal tool state. Tool executors remain supplied by Codex, and its sandbox remains authoritative.

## Operational results

All execution passes require a real invocation/result, exit status 0, and exact fixture working directory. Recall is checked independently against the package's canary digest.

| Case | Actual terminal execution | Exact recall | Evidence / qualification |
| --- | --- | --- | --- |
| A: fresh, observation | PASS | NOT_TESTED | No imported canary in fresh control |
| B: ordinary cold resume, observation | PASS | NOT_TESTED | Same fresh fixture, new app-server process |
| C: imported cold resume, observation | FAIL as expected | PASS | Zero terminal invocations; `TOOL_UNAVAILABLE` |
| D: imported, repetition before checkpoint | FAIL as expected | PASS | Same declaration inventory; negative ordering control |
| E: imported, repetition after checkpoint | PASS | PASS | Native checkpoint preserved |
| F: cold-resume E again, correction enabled | PASS | PASS | No manual fixture/session edit |
| G: restart Astral, cold-resume E | PASS | PASS | New process and empty proxy state directory |
| H: websocket warmup, warm turns, reconnect | PASS | PASS | Two turns; additionally restarted Astral between turns of the same Codex process |
| I: two native checkpoints and changed inventory | PASS | PASS | Last-checkpoint correction; `view_image` disabled then restored on cold resume |
| J: separate alpha/beta inventories | PASS | PASS | Both real marker tools execute; no opposite marker declaration in either inventory |
| K: read-only execution | PASS | PASS | `pwd` succeeds; fixture write returns exit 1 and permission denial; file unchanged |

Recall runs on an **ephemeral fork of each tested imported parent**, through the same proxy mode. The plaintext target is not supplied in a prompt or handoff. The harness hashes the returned answer and checks that it is absent from the parent's entire readable rollout, excluding opaque encrypted fields. This avoids contaminating subsequent parent resume tests with a readable answer. These are fork-based continuity checks, not answers emitted into the execution parent itself.

Every inspected request records encrypted-content hashes and stable native-item hashes before/after transformation, plus hashes of the original input and the transformed input with only the insertion removed. These matched. Codex import itself adds `internal_chat_message_metadata_passthrough` to the source checkpoint item; all source fields are preserved. That normal runtime enrichment is separate from the adapter, which preserves the resulting native item exactly.

Request byte counts and provider-reported usage are recorded separately. Correction increases wire bytes by repeating declarations. Encrypted byte length is not a token measurement. These trials establish no token-saving or invoice-saving claim.

## Failed attempts and limitations

- Explicit `thread/compact/start` produced a reset/compaction request shape rejected as `AST000_UNSUPPORTED_RESET_OR_REFERENCE`. The adapter intentionally does not implement that transition. A subsequent unmodified observation-mode compaction generated a second native checkpoint for the multiple-checkpoint fixture. Astral did not autonomously roll it.
- The first observation-mode compaction retry had an app-server error before a thread was returned. Its type/hash were retained, not its potentially sensitive body. Its precise cause is unestablished; a sequential retry succeeded.
- The first restriction receipt incorrectly counted only `commandExecution` notifications. The denied invocation instead had a linked native custom-tool call/result with exit 1. Inspection established the denial, and the updated harness independently repeated it successfully.
- Early inventory diagnostics counted incidental tool-name mentions in descriptions. Final diagnostics parse actual generated declarations; removed `view_image` is absent. The transformation itself always copied the full current runtime descriptor, not these diagnostic flags.
- Initial Python tests ran before the required release helper binary was built and had five missing-binary errors. After the release build, all Python tests passed. Current Clippy flagged a pre-existing boolean expression in the touched HTTP header helper; an equivalent simplification resolves it.

NOT_TESTED against the live provider: cross-account switching, inventory changes inside an already-open client connection, explicitly empty native inventories, and pipelined/cancelled/reset frames. Structural and transport tests cover empty inventory replacement, isolation across synthetic credentials/accounts, stale references, and rejection of unsupported resets/pipelining. These are not claimed as live-provider passes.

Unsupported correction shapes: non-Lite/missing verified prefixes, empty base-instruction prefix layout, arbitrary imported declarations, external item/conversation references, inline rolling/reset/configuration-update items, compressed HTTP, HTTP incremental references, websocket binary client requests, websocket subprotocol negotiation, concurrent in-flight requests, and deltas referencing another connection or unknown completion. Correction reports errors instead of silently forwarding an uncorrected checkpoint. Observation mode explicitly reports parse/shape failures and remains a pass-through control.

## Code, tests, and production work

- `src/ast000.rs`: transformation, prefix validation, isolated incremental state, hashes and inventory metadata.
- `src/ast000_ws.rs`: opt-in websocket relay, unchanged upstream events, TLS validation, aggregate-only usage recording.
- `src/config.rs`, `src/proxy.rs`: opt-in configuration, loopback/TLS constraints, forced pass-through mode, native routes and HTTP fallback.
- `scripts/ast000_trial.py`: disposable-session controls, native imports, exact recall on ephemeral forks, sandbox result auditing and controlled reconnect.
- `tests/ast000.rs`, `tests/ast000_transport.rs`, `tests/test_ast000_trial.py`: structure, real local HTTP/websocket transport, identity isolation, and denial evidence checks.

Local verification includes the repository's format, Clippy, locked Rust tests, release binary build, and Python test suite. Diagnostic evidence lives outside tracked source in a private experiment directory; this document contains no private transcript, credential, or checkpoint payload.

Before production: define supported protocol/version negotiation and trusted-client provenance beyond positional validation; cover compaction/reset/cancellation/pipelining and all reconnect paths; propagate needed upstream handshake/session metadata; test inventory changes during a connection and real cross-account separation; support or explicitly negotiate compression/fallback; bound concurrent connection state and backpressure; extend restricted-operation tests across supported platforms. The experimental lane has no durable inventory cache, no executor, and no permission override.

The portability contract remains: **import preserves conversational work state; resume supplies executable capabilities from the current trusted runtime.** Export opaque native context and continuation items. Rebind executors, current declarations, workspace association, account/provider routing, approvals, and sandbox policy at the destination. Historical tool descriptions in an exported conversation are not executable authority.
