# Shared proxy verification — 2026-10-01

The patch for `98d71ad` was applied with `git am` to a clean checkout of
`f2a7c8f`. Its resulting tree matched the implementation exactly:
`255edbb725f08d9f886c8753224f4e5e37f04c51`.

## Repository checks

- The full locked Rust suite on stable Rust 1.99 passed 515 tests. One existing
  `hook_deadline` test failed at its initial process-visibility assertion.
  A separate control spawned a live child and confirmed it with `kill(pid, 0)`,
  while `/bin/ps -p PID -o stat=` returned exit 1 and
  `fatal library error, lookup self`. This does not verify that lifecycle test.
- After the WebSocket correction below, all 24 tests in `native_transport`,
  `tool_history` and `universal_proxy` passed on Rust 1.85. These include all
  15 shared-proxy/archive tests. The entire suite was not rerun for that correction.
- Formatting, strict stable Clippy, Rust 1.85 all-target checking and a locked
  release build of all binaries passed.
- All 18 Python tests passed against the release helpers.

These are Linux checks. macOS and Windows were not exercised in this workspace.

## Independent compiled-binary exercise

A separate local Python driver started the release `astral proxy` executable
with default reduction settings and temporary state. Local HTTP providers
captured the forwarded requests. Twenty-four requests ran concurrently: eight
project identities, each using Responses, Chat Completions and Messages.

Each request contained one long turn with sixteen 32 KiB text tool results.
The oldest twelve were archived; the latest four remained complete. Restoring
the twelve bodies produced an object equal to the entire original request.

| Protocol | Original request bytes | Forwarded bytes | Reduction |
| --- | ---: | ---: | ---: |
| Responses | 550,598 | 171,098 | 68.93% |
| Chat Completions | 551,065 | 171,565 | 68.87% |
| Anthropic Messages | 551,287 | 171,787 | 68.84% |

These synthetic byte measurements are not token, quality, latency or billing
results. All eight projects produced the same per-protocol sizes.

Additional controls passed:

- Handles remained distinct across projects and stable on repeated requests.
- Anthropic SSE content, including split multibyte text, was forwarded exactly.
- Paginated retrieval reconstructed the original bytes and matched their SHA-256.
  Pages deliberately split UTF-8 characters and exercised the hex fallback.
- The actual `astral recall` CLI found a detail omitted from the preview.
- Restarting the proxy preserved both retrieval and deterministic handles.
- The existing Rust integration test invoked the actual stdio MCP process and
  retrieved a hidden detail through `astral_search` and `astral_recall`.

## Defect found and corrected

The independent WebSocket control received upstream close code `1013` with
reason `retry later` directly, but the proxy originally sent an empty close.
A Rust regression reproduced this as `Close(None)` before the correction.

The ordinary WebSocket relay now forwards close codes and reasons in both
directions. The regression checks upstream `1013 / retry later` and downstream
`1001 / client done`. Both pass, and the independent exercise passes after
rebuilding. The managed native-binding bridge was not changed.

## Live-provider status

The bundled Codex CLI is `0.154.0-alpha.3`; `codex login status` reports
`Logged in using ChatGPT`. OpenAI API-key absence does not establish that Codex
authentication is unavailable. The first environment check missed this binary
because it was not on `PATH`.

Disposable `codex exec` probes did not start a session within their deadlines.
A direct control without Astral also produced no session events. The Codex
app-server successfully answered `initialize`, but `thread/start` did not return
within 40 seconds. This also occurred with its existing local model catalog and
an HTTP-only custom provider using `requires_openai_auth=true`. That probe's
Astral health check reported `requests_received: 0` and its ledger was empty.
The saved login's inference access was therefore not verified.

No successful live OpenAI or Anthropic inference was observed in this test run.
Anthropic was exercised against local protocol fixtures. No changes were made
to account credentials or user Codex configuration, and trial processes were
stopped. A live workload and model-recall evaluation remain necessary before
claiming provider compatibility or a quality/performance benefit.
