# One proxy for multiple projects and providers

Run a single Astral process outside any particular repository. Clients retain
their own model choice, tool execution, authentication and permissions. No OSTK,
kernel, project index or replacement agent harness is required.

```sh
cargo install --path . --locked --bin astral
astral proxy --state-dir "$HOME/.local/state/astral/proxy"
```

The listener defaults to `127.0.0.1:8088`. Choose one persistent state directory
for this process; changing projects does not require restarting it. The legacy
default remains `.astral-runtime` relative to the proxy process's working
directory. `ASTRAL_PROXY_STATE_DIR` can set a persistent default.

## Connect clients

Configure each harness or SDK's provider base URL once:

| Client protocol/provider | Base URL | Generation endpoint |
| --- | --- | --- |
| OpenAI Responses | `http://127.0.0.1:8088/v1` | `/responses` |
| OpenAI Chat Completions | `http://127.0.0.1:8088/v1` | `/chat/completions` |
| Anthropic Messages, including Claude Code | `http://127.0.0.1:8088` | `/v1/messages` |
| Named OpenAI route | `http://127.0.0.1:8088/providers/openai/v1` | `/responses` or `/chat/completions` |
| Named Anthropic route | `http://127.0.0.1:8088/providers/anthropic` | `/v1/messages` |
| Codex ChatGPT backend | `http://127.0.0.1:8088/providers/codex/backend-api/codex` | `/responses`, HTTP or WebSocket |

For clients that honor these environment variables:

```sh
export OPENAI_BASE_URL=http://127.0.0.1:8088/v1
export ANTHROPIC_BASE_URL=http://127.0.0.1:8088
```

Harnesses with their own provider configuration need the equivalent base URL
there. These variables do not override every harness's configuration. A harness
that hardcodes its destination or uses a different protocol needs an adapter.
Codex authentication and backend selection remain the client's responsibility;
an API key is not a substitute for a ChatGPT account credential.

For a Codex CLI already signed in with ChatGPT, retain that login and configure
the built-in OpenAI provider's destination:

```sh
codex -c 'openai_base_url="http://127.0.0.1:8088/providers/codex/backend-api/codex"' \
  -c 'features.enable_request_compression=false'
```

Codex supplies the credential; Astral forwards it to the configured ChatGPT
backend. A custom Codex provider can instead use `requires_openai_auth=true`.
See [Codex authentication](https://learn.chatgpt.com/docs/auth) and the
[base-URL setting](https://learn.chatgpt.com/docs/config-file/config-reference).
Disabling request compression lets Astral inspect full request histories.

Default destinations are `https://api.openai.com/v1`,
`https://api.anthropic.com/v1`, and `https://chatgpt.com/backend-api/codex`.
`--upstream` / `ASTRAL_UPSTREAM` change the OpenAI destination;
`--anthropic-upstream` / `ASTRAL_ANTHROPIC_UPSTREAM` change Anthropic's.
Routes never infer destinations from model names. Arbitrary models on compatible
APIs can use the same reduction without provider-specific model allowlists.

Client credentials take precedence. With no client credential, the default
OpenAI route can use `OPENAI_API_KEY`, and Anthropic can use `ANTHROPIC_API_KEY`.
The Codex route has no credential fallback. Anthropic version and beta headers,
cache controls, thinking blocks, tool definitions and unknown fields are retained.
The proxy does not translate authentication or API formats.

## Additional providers

```sh
astral proxy --state-dir "$HOME/.local/state/astral/proxy" \
  --providers /absolute/path/providers.toml
```

```toml
[[providers]]
name = "local"
protocol = "openai"
upstream = "http://127.0.0.1:11434/v1"

[[providers]]
name = "team-claude"
protocol = "anthropic"
upstream = "https://your-anthropic-compatible-gateway.example/v1"
api_key_env = "TEAM_CLAUDE_API_KEY"
```

Use `http://127.0.0.1:8088/providers/local/v1` as the local model's base URL.
Custom routes have no environment credential fallback unless `api_key_env` is
explicitly configured. A configured built-in name replaces that route. Upstream
URLs include the API version path and cannot contain credentials, query strings
or fragments. Configuration is read at startup and accepts at most 32 entries.

## What gets removed from model input

The default `--mode tools` archives eligible **text tool-result bodies**, leaving
a short head/tail preview, the original byte count and a retrieval handle.
It preserves the caller's original history locally: only the upstream request
copy changes. It never claims a preview is a semantic summary or a successful
check, and never reruns a command to reconstruct historical output.

Defaults:

- A result must be at least 16 KiB (`--tool-result-bytes`).
- A preview retains at most 2 KiB of original text (`--tool-preview-bytes`),
  plus an archive marker and retrieval instructions.
- Results before the two most recent user turns are eligible
  (`--keep-recent-turns`).
- Within a long turn, older consumed results can also be archived while keeping
  the most recent four results (`--keep-recent-tool-results`). Set this to zero
  to restrict reduction to older user turns.
- The latest assistant's tool-result batch stays complete, even when a parallel
  batch contains more results than the configured retention count. Unresolved,
  duplicate or unmatched client tool calls disable reduction for that request.

Tool-call IDs, tool inputs, message order, user messages, assistant text,
reasoning/thinking, signatures and native checkpoints remain intact. Explicitly
marked tool errors and recognized structured failures remain intact. Arbitrary
terminal text is not classified as success/failure. Multimodal outputs and text
blocks with additional metadata, including cache annotations, are preserved.
Large plain-text errors without a machine-readable status can be archived with
the same explicit preview marker and exact retrieval as any other text.

Text strings are archived as their exact UTF-8 content. Plain text-block arrays
are archived as JSON values, preserving the full original structure. Retrieval
reports which format was stored. Opaque or unfamiliar output shapes pass through.

Repeated reductions of the same output in the same scope produce stable handles
and previews, including after restart. When an output first ages into eligibility,
the upstream prefix changes and may lose a provider prompt-cache hit. This is a
context-size tradeoff, not a promise of token-price or latency savings.

## Give the model exact retrieval

For an MCP-capable harness, register this stdio server once:

```json
{
  "mcpServers": {
    "astral": {
      "command": "astral",
      "args": ["mcp", "--proxy-url", "http://127.0.0.1:8088"]
    }
  }
}
```

The outer configuration shape varies by harness. The command and arguments are
the same. The adapter supports MCP stdio protocol `2024-11-05`, negotiates that
version during initialization, and exposes `astral_recall` and `astral_search`.
It is a retrieval client for the one running proxy, not another proxy or kernel.
Tool declarations are installed through the harness, never injected secretly
into provider requests. Use a supported retrieval route before relying on
archival in a harness that cannot execute local commands.

Models with shell access can use the CLI instead:

```sh
astral recall HANDLE --offset 0 --limit 4096
astral recall HANDLE --query 'specific error'
```

The CLI always returns JSON. `ASTRAL_PROXY_URL` or `--proxy-url` select a
nondefault listener for both CLI and MCP. `search` returns byte offsets; `recall`
returns up to 16 KiB and a `next_offset`. A page that splits a UTF-8 character is
hex-encoded with an explicit encoding field, so byte recovery remains exact.

HTTP clients can GET `/_astral/artifacts/HANDLE?offset=0&limit=4096`, or add
`query=literal-text` to search. Searches return at most 20 matches and a cursor.
The 256-bit opaque handle is a bearer capability: anyone given it can retrieve
that artifact from the listener. There is no archive listing endpoint. Responses
disable caching and browser-origin requests are rejected.

## Isolation and persistence

Handles are keyed by a private per-store secret and scoped to the configured
provider, upstream, credentials, model, prompt/tool contract, conversation hints
and any supplied workspace/harness identifiers. Optional headers are:

```text
x-astral-workspace-id: a-stable-project-identifier
x-astral-session-id: a-stable-conversation-identifier
x-astral-harness-id: your-client-name
```

All `x-astral-*` controls are stripped upstream. Existing `session_id` and
`openai-session-id` are recognized. Anthropic metadata is included in archive
scope without guessing the format of its user/session identifiers. If no session
identifier exists, only immutable identical artifacts can share a handle within
the same scope; Astral does not invent mutable shared conversation state.
Native rolling still requires an explicit stable session identity.

Artifacts are published atomically with file synchronization before a result is
replaced. Original-content hashes and handle identity are checked on retrieval.
Unix directories/files are private to their owner. The existing process lock
allows one writer per state directory, while many projects can use that process.

The archive defaults to 512 MiB (`--archive-max-bytes`). If it is full, an artifact
is corrupt, or a write fails, the original result remains in the upstream input.
There is no automatic eviction that could strand a handle already in a session.
Stop the process before moving/deleting its state, and retain it while saved
conversations still need its handles. Native Git bundles do not package this
local artifact store. This listener is intended for a trusted single-user host;
it does not implement remote multi-user authentication.

## Modes and boundaries

| Mode | Behavior |
| --- | --- |
| `tools` (default) | Provider-independent tool-result archival; no extra inference calls |
| `rolling` | Existing OpenAI Responses native projection planner; Messages/Chat pass through |
| `passthrough` | Unmodified bodies for all protocols; transport/header handling still applies |

Switching from older Astral versions: request `--mode rolling` explicitly to retain
the old standalone default. Combining history rewriting with native rolling is
not enabled implicitly: they have distinct history-ownership contracts.

Full-history Responses requests can be reduced over HTTP or WebSocket.
`previous_response_id`, server-side conversation references, caller-owned
`context_management`, compaction controls and compressed requests pass through
without reduction. The proxy cannot edit history already stored at a provider.
Binary WebSocket frames pass through without decoding. The existing managed
native-launch proxy remains a separate, narrowly qualified tool-rebinding path;
its passthrough and runtime-compatibility contract is unchanged.

Messages token-count requests are forwarded exactly, so they measure the input
the caller supplied. Model catalog GETs are routed to the selected provider.
Uploads, realtime audio and other API families are outside these adapters.
SSE responses are forwarded chunk-for-chunk; no response or tool execution is
replayed. Error status, retry metadata and provider usage remain provider-owned.

The ledger records reduction counts, byte savings and unavailable-artifact counts
without tool-output contents or retrieval handles. Anthropic cache-read/write
tokens are included when normalizing its input total; OpenAI input totals already
include cache reads. Missing usage stays unreported. These are request/response
observations, not verified billing savings.

For Responses HTTP forwarding, `request` ledger rows record body sizes once
upstream response headers arrive. `response_terminal` rows record an observed
SSE terminal event and its available usage before yielding that chunk downstream.
Both carry a `request_id`, also included in the clean-EOF `response` row. This keeps
measurements available when a client disconnects before EOF. A terminal event
does not prove clean EOF or commit a rolling projection; those checks remain in
the final response path. Sum request sizes from `request` rows and usage from
`response_terminal` rows without adding the duplicated final-response values.
Failed sends before response headers and missing terminal events require separate
error/coverage accounting. These new rows do not cover WebSocket or Messages
traffic.

## Verification

`tests/tool_history.rs` checks three protocol shapes, exact text/JSON retrieval,
long-turn retention, parallel batches, failures, opaque content, quotas, corruption
and restart. `tests/universal_proxy.rs` uses real local HTTP, SSE and WebSocket
servers, routes multiple providers through one listener, and retrieves a hidden
detail through the actual MCP subprocess. These tests do not establish live
provider compatibility or a quality/performance result for arbitrary workloads.

The [2026-10-01 verification record](universal-proxy-verification.md) includes
clean-patch checks, concurrent compiled-binary measurements, the WebSocket close
regression and the unsuccessful live-provider startup probes.

Wire references: [Anthropic tool use](https://platform.claude.com/docs/en/build-with-claude/tool-use/overview),
[Anthropic streaming](https://platform.claude.com/docs/en/build-with-claude/streaming),
[MCP stdio](https://modelcontextprotocol.io/specification/2024-11-05/basic/transports).
