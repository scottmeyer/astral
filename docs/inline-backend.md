# Astral-scheduled inline checkpoints

`--compaction-backend inline` lets Astral schedule native compaction within a normal generation response. It is an opt-in alternative to the default standalone compact endpoint.

```sh
./target/release/astral proxy \
  --upstream https://chatgpt.com:18080/backend-api/codex \
  --allow-compatible-compaction \
  --compaction-backend inline \
  --inline-threshold-tokens 8192 \
  --roll-bytes 64000 \
  --keep-recent-turns 1
```

The client keeps sending its complete original history, including every output item returned by the proxy. It does not prune the history or adopt a proxy-internal checkpoint itself. The proxy forwards response bytes unchanged.

## Scheduling and ownership

Astral uses the existing size, cooldown, and safe new-user-boundary checks to decide which requests may compact. On an eligible request, it injects `context_management` with the configured token threshold. The provider decides whether its rendered context crosses that threshold and emits any native compaction item in the response stream. `x-ostk-roll: 1` bypasses Astral's size and cooldown gates; it does not bypass the provider's token threshold.

`--inline-tool-boundaries` additionally allows an eligible request ending with
the last outstanding external tool result. All known calls must have exactly
one matching result; duplicates, orphans and partial batches cannot use this
boundary. The entire effective window is forwarded intact. The new explicit
host enables this option; the bare proxy retains its original default.

The two thresholds measure different things: `roll-bytes` measures Astral's serialized effective input; `inline-threshold-tokens` is interpreted by the provider. Inline mode does not make a separate preparatory generation or standalone compact request.

In standalone mode, `keep-recent-turns` reserves an exact raw tail outside the compacted prefix. In inline mode it helps determine user-boundary eligibility, but the provider may compact the entire effective window, including the latest user message. Inline mode therefore does not promise a particular number of raw retained turns. Astral never schedules inline compaction while a known tool call still lacks its result. After a provider-issued checkpoint, its documented replacement semantics govern the retained context.

Caller-supplied `context_management` continues to bypass Astral's planner entirely. That is the provider-managed reference mode. API Platform cache policy and caller controls retain their existing behavior; inline mode does not change cache TTL or reasoning-context settings.

## Mapping the client's original history

Suppose the current original input is `H`, and the completed response output is `O`. The latest native compaction item is `O[j]`. The client will next send `H ++ O ++ new_items`.

After a successful complete response, the candidate mapping is:

```text
projection = [O[j]]
cut = len(H) + j + 1
expected original prefix = H ++ O[:j+1]
next effective input = projection ++ next_original_input[cut:]
```

This preserves the checkpoint and every output item following it exactly once, including reasoning, assistant phase, and tool calls. Response items before the checkpoint are represented by the authoritative checkpoint. The entire expected original prefix is fingerprinted, so a missing or changed replayed item resets to the client's authoritative history instead of silently skipping it.

Only the checkpoint is persisted as the projection. The client remains responsible for replaying the subsequent output tail. All checkpoint fields remain opaque and unchanged.

## Commit and failure behavior

The observer collects complete native output items with bounded aggregate storage. It supports both canonical JSON output and SSE gateways that emit items in `response.output_item.done` but leave the terminal output array empty. Missing indices, conflicting repeated items, malformed checkpoints, and observation overflow prevent checkpoint adoption. Transport errors, truncation, and client disconnects prevent commit even if the checkpoint has already appeared in the stream.

On a completed response, a valid checkpoint must also satisfy the configured wire-size savings gate before Astral adopts it. A rejected checkpoint leaves the previous local mapping in place; response bytes are still forwarded. Persistence failure retains the previous durable state. Successful commits store expected output-prefix hashes alongside the checkpoint, allowing restart without requiring another model call.

The ledger records whether inline compaction was scheduled, how many checkpoints were observed and adopted, the resulting epoch/cut, a projection hash, and the actual effective-input hash. These hashes permit audit without logging encrypted payloads. Changing backend or its configured threshold opens a separate lane.

Inline compaction work is reported within generation responses rather than separate compact ledger rows. Zero standalone calls does not imply zero compaction cost. Reconcile provider billing before making dollar claims.

## Validation

Integration tests exercise two checkpoints with the full original client history, a checkpoint after an earlier output item, active tool continuations, exact response bytes, unchanged caller cache controls, restart, edits to replayed checkpoints, truncation, incomplete output-item sequences, and disconnect after a checkpoint is received. Observer tests cover canonical JSON/SSE equivalence, duplicate conflicts, and aggregate capacity limits.

`scripts/benefit_trial.py` compares this backend against full history and provider-managed inline compaction using matched random fixtures, no early answer leakage, a correction, and proxy restart. See the [benefit evaluation](benefit-evaluation.md) for measured results and limits.
