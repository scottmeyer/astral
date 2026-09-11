# Opaque reasoning and live rollover

Verified on 2026-09-11 with `gpt-6-astra` through the configured gateway. **Native inline compaction works**, even though the standalone compact endpoint does not.

## Preserve complete typed items

Preserve `reasoning.encrypted_content` as an opaque string within its complete reasoning item, including its ID, summary, and any other fields. Preserve item order, tool call IDs/results, and assistant phase. Astral forwards response bytes unchanged; the client must replay complete output items in subsequent input arrays. When streaming gateways leave the terminal output array empty, reconstruct it from complete `response.output_item.done` items in index order.

An encrypted reasoning item is not interchangeable with an encrypted compaction item. Native compaction produces a typed `compaction` checkpoint representing the provider's replacement state. Never relabel an ordinary reasoning item or treat it alone as the compacted conversation.

OpenAI documents opaque reasoning replay with `store:false`, including prior-turn reasoning under a supported `reasoning.context` mode. The gateway used `all_turns` in these probes. Preserve the caller's context setting rather than silently selecting a different reasoning lifetime. [Reasoning contract](https://developers.openai.com/api/docs/guides/reasoning).

## Working gateway path

Use the normal `/responses` route with:

```json
{
  "store": false,
  "reasoning": {"effort": "medium", "context": "all_turns"},
  "context_management": [
    {"type": "compaction", "compact_threshold": 8192}
  ]
}
```

For inline compaction, append all output items to the current history, then optionally discard items before the latest compaction item. Keep that item and everything after it intact. This pruning rule applies to inline compaction only. A standalone `/responses/compact` response is already the full canonical replacement window and must not be pruned. [Compaction contract](https://developers.openai.com/api/docs/guides/compaction).

Astral deliberately passes caller-supplied `context_management` requests through. The provider controls these compactions; Astral's autonomous planner is not active in this mode. The separate [inline backend](inline-backend.md) allows Astral to schedule its own checkpoints when the caller supplies full original history and omits context management.

## Executed checks

The initial three-call probe produced a 3,064-byte encrypted compaction string and a separate 1,484-byte encrypted reasoning string. Both were replayed with unchanged SHA-256 hashes, including after a proxy restart. All answers were correct. That probe's visible first answer repeated the facts, so its later recall alone did not prove the facts survived in opaque state. [Probe receipt](receipts/opaque-inline-2026-09-11.json).

The stronger `scripts/inline_trial.py` test used four random string values plus a corrected numeric budget. Values were supplied only through two actual tool results of approximately 110 KB each. Before the final question, assistant replies were restricted to `READY`.

| Check | Result |
| --- | --- |
| Actual tool reads | Both fixtures loaded |
| Native inline compactions | Two |
| Recursive input | First checkpoint hash present in the request producing the second |
| Original fact strings in final plaintext input | None |
| Restart | Proxy restarted and saved client window reloaded |
| Opaque window after reload | Identical hashes |
| Final recall | All four random strings and corrected budget recovered exactly |
| Generation calls | Six completed; ingress 5 before restart, 1 after |
| Proxy shutdowns | Both exit code 0 |

The input array submitted for final recall was 5,072 serialized bytes. The two requests carrying large fixture results were 112,261 and 116,040 bytes. These are input-array wire measurements, not tokenizer estimates or savings against a matched baseline. [Canary receipt](receipts/inline-canaries-2026-09-11.json).

Earlier low-effort matched trials produced zero encrypted reasoning items. Their successful full-history recall did not exercise live encrypted reasoning replay. The new three-call probe covers that missing case; the canary trial covers recursive native compaction without visible-answer leakage.

## Remaining validation

The two successful inline trials establish a working native transport reference. They do not prove general semantic fidelity, the causal contribution of reasoning replay, cache retention duration, or a cost advantage. Inline compaction shares the response stream, so zero standalone compact ledger rows must not be interpreted as zero compaction work or cost.

Astral's standalone planner still needs a live backend implementing its compact contract. The subsequent [benefit evaluation](benefit-evaluation.md) validated Astral's new inline backend on two task seeds, including two owned checkpoints, actual projected-input hashes, restart, and no plaintext fact leakage. It compared full history and provider inline, finding input reduction versus full history but slower complete tasks. Local interrupted-stream controls retain old state even when a checkpoint arrives before disconnection. Broader task quality and reliable cached production economics remain unverified.

Cache checks should separately verify stable checkpoint bytes between rolls, stable routing keys and caller cache controls, and provider-reported cache reads/writes. Persisting an opaque checkpoint does not pin the provider's prompt cache or establish its TTL.
