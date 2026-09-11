# AST-000 reviewed integration

Date: 2026-09-11. This is a **new local verification pass**, distinct from the
[repair session's live lifecycle evidence](ast000-lifecycle.md). The original
workspace was clean at the historical baseline before integration; the repair
worktree contained the uncommitted adapter, lifecycle tests and context bootstrap.

## Review changes

Independent transformation and transport reviews found two concrete gaps:

1. The relay discarded successful upstream websocket metadata consumed by Codex.
   It now forwards only `x-codex-turn-state`, `x-reasoning-included` and
   `openai-model` on the downstream handshake. Rejected upgrades preserve status
   and allowlisted retry/auth-classification metadata, without copying upstream
   error bodies, credentials, cookies or arbitrary headers. Connection-nominated
   hop headers are excluded. A local reconnect fixture reuses the sticky token.
2. Client-executed native tool searches were not counted as pending external calls.
   Compaction now waits for matching completed client tool-search output. Search
   execution mode, IDs and result envelopes are checked; provider-side searches
   remain provider-owned. Discovered schemas do not replace the current runtime
   declaration inventory. The private capture helper also rejects unresolved,
   duplicate, orphan and mismatched external call/result boundaries.

Observation mode additionally forwards binary client frames unchanged while
recording that structural inspection is unsupported. Correction still rejects them.

## Current local verification

Environment: macOS, Rust/Cargo 1.95.0, Python 3.14.7.

- Rust formatting: pass.
- Clippy, all targets with warnings denied: pass.
- Locked Rust suite: **75 passed** (13 library, 23 adapter, 9 transport,
  25 existing proxy, 5 host-working-state).
- Locked release binaries: build passes.
- Python suite: **18 passed**.
- Diff whitespace checks: pass.

The UBS staged audit could not run: after the sandboxed attempt could
not download its Rust module, the authorized retry rejected that module because
its checksum did not match the scanner's expected checksum. The integrity check
was not bypassed. UBS is **unavailable**, not a passed source audit.

The initial sandboxed transport run failed to bind loopback fixtures with
`Operation not permitted`. The full Rust suite then passed with the authorized
local test execution outside that sandbox. This was an environment failure, not a
silently skipped transport test.

These are local tests using synthetic upstreams. The review fixes have **not**
been rerun against the live provider in this pass. The current conversation's
successful terminal probes establish operational recovery on the previously
running build, not deployment of these new changes. No recovery proxy was
restarted, no installed Codex binary or original session was patched, and no
authentication settings were changed by this integration pass.

## Still experimental

The original opt-in/trusted-loopback restrictions remain. Production work includes
version negotiation, stronger client provenance, concurrent connection and output
bounds, bounded writes/closes and backpressure, additional live identity/inventory
coverage, and explicitly supported cancellation/configuration transitions.
Read timeouts are inactivity timers; they do not yet bound every write or the
total lifetime of a bridge. AST-010 tracks these limitations.

Historical live results retain their original scope. No new provider compatibility,
token savings, invoice savings, or production-readiness claim follows from this
local integration.
