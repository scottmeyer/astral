# Astral

A standalone Rust proxy for the **OpenAI Responses API**, with persistent rolling projections, a frozen prefix between rolls, model-aware cache controls, and an unmodified response stream. An optional host layer adds compact tool observations, exact artifact retrieval, versioned workspace verification, and durable action replay. No `haystack` or other private dependencies.

The unified executable is `astral`: use `astral proxy` for the server and `astral project` for project context. The internal Rust crate remains `ostk-gpt-cache`. See [the trial runners](docs/codex-trials.md) for Codex and Responses clients. The default backend uses standalone native compaction; the opt-in [inline backend](docs/inline-backend.md) lets Astral schedule and persist checkpoints through the normal Responses route. The [validation receipt](docs/validation.md) distinguishes these paths and records the remaining environment limitations.

The opt-in [native lifecycle experiment](docs/native-recovery-lifecycle.md) adds a
Codex Responses Lite websocket relay and checkpoint tool rebinding. It requires
pass-through mode with Astral rolling disabled. The [project-context inspector](docs/project-context.md)
validates the committed `.astral/` manifests and JSONL work register, then resolves
named subsystems or projections to source handles and hashes. Run
`astral context list` or `astral project --inspect --work AST-001`; the omitted
context defaults to `project-context`.
`astral project` now launches fresh document contexts in Codex; `astral init`
asks Codex to build a best-effort index in an uninitialized repository. See
[fresh launch](docs/fresh-launch.md) for commands and current runtime limits.
Explicit [native bundles](docs/native-bundles.md) can now be validated and inspected
as Git-trackable data. Native capture and restoration remain pending.

Inspection prints formatted JSON. `astral work id` proposes a short random work
ID for new JSONL records, avoiding per-branch counters. See the
[launch milestones](docs/launch-plan.md) for fresh initialization, Git-tracked
native bundles, launch, worktrees and handoff.

Use [the working-state integration](docs/working-state.md) to run the new agent host: `astral-state` manages configured files, checks and immutable artifacts; `examples/agent.py` connects them to Responses tools. It records immediate state changes while freezing model-visible snapshots between native checkpoints. The supplied coding trial compares native compaction, native compaction with the same adapter, and Astral with the adapter and state layer.

The [first coding evaluation](docs/working-evaluation.md) passed all six tasks,
including unseen executable checks and exact retrieval after restart. The
adapter reduced tool-observation bytes by 93.6%. Astral finished faster in this
small experiment while reporting more input tokens; verified cost savings and
a general advantage over adapted native compaction remain unestablished.

In a [two-seed live comparison](docs/benefit-evaluation.md), Astral inline reduced input tokens by 69.4% against full history, increased wall time by 22.1%, and passed all recall/restart checks. Provider-managed compaction used fewer input tokens than Astral. These are synthetic-task measurements, not verified cost savings.

For the standalone backend, the projection is the **complete native `/responses/compact` output**. The proxy maps the client's original full history to that compacted prefix, then appends the uncompressed recent tail. The next rollover compacts the previous projection plus newly completed turns. Inline mode instead adopts the latest native checkpoint from a completed generation and maps the original history through that output item. The client continues sending ordinary full-history requests in both modes.

## Build and run

Use a current stable Rust toolchain (edition 2024).

```sh
cargo build --release --locked --bins
cargo test --locked

# Authentication can be supplied by the client, or inherited from this environment.
export OPENAI_API_KEY='your-api-key'
./target/release/astral proxy

# In another terminal, run a full-history client:
python3 examples/chat.py --model gpt-5.5
```

The API is available at `http://127.0.0.1:8088/v1/responses`. The upstream defaults to `https://api.openai.com/v1`. Set `--upstream` to a base URL that already includes the API version or backend path; the proxy appends `/responses` and, by default, `/responses/compact`. `--compact-path` overrides only the latter path for both autonomous and caller-initiated compaction. Changing the path opens new state lanes. An alternate route must still return the native compact contract; an ordinary generation, even with encrypted reasoning, cannot replace history.

For a managed upstream with a private CA, the proxy adds certificates from `SSL_CERT_FILE` or the explicit `--upstream-ca-bundle /path/to/ca.pem` option to its trusted roots. Certificate verification stays enabled; an unreadable, empty, or invalid configured bundle fails startup.

`POST /responses`, `/v1/responses`, and `/backend-api/codex/responses` all map to the configured upstream's `/responses`. Corresponding `/responses/compact` routes and the `/compact` alias forward caller compaction bodies unchanged. `GET /healthz` checks the listener and reports `requests_received`: entries into generation and caller-compaction handlers since process startup, including requests rejected by identity checks. Health probes and internal upstream compaction do not increment it. This counter distinguishes absent client traffic from traffic without a completed ledger entry. The opt-in native tool binding mode also handles websocket GET on the generation routes. Other routes, including file upload, are intentionally absent.

## Client contract

Send a stable `x-ostk-session-id` header and the **complete original input array**, including replayable output items from earlier responses. The proxy also recognizes `session_id` and `openai-session-id`, in that priority order after the explicit header. A changing request ID is not a session identifier.

```sh
curl http://127.0.0.1:8088/v1/responses \
  -H 'Content-Type: application/json' \
  -H 'x-ostk-session-id: project-a-thread-1' \
  -d '{"model":"gpt-5.5","store":false,"input":[{"role":"user","content":"Inspect the design."}]}'
```

The example uses the proxy's environment credential. A client's `Authorization`, `api-key`, or `x-api-key` header takes precedence. Raw credentials are never written into lane files or the ledger. Internal `x-ostk-*` headers are removed before forwarding.

Requests with no session identity, a string input, item references, `previous_response_id`, `conversation`, `context_management`, or `background:true` pass through without projection or cache-parameter mutation. These forms do not give this proxy ownership of a complete explicit conversation. It never expands server-side history or competes with a caller's compactor.

For SDK callers, set the base URL to the proxy and attach `x-ostk-session-id` as a default header for each conversation. Keep your original history on the client; **do not replace it with a proxy-internal projection**. The proxy doesn't alter response IDs, usage, output items, reasoning, or assistant phase.

## What rolls, and when

With no projection, the entire input forwards normally. Once effective input exceeds `--roll-bytes` (160,000 by default), the proxy can compact older completed turns. The default standalone backend keeps the most recent two user turns and everything after them intact. Inline mode lets the provider checkpoint the effective window and does not reserve a fixed raw tail; see [its ownership contract](docs/inline-backend.md).

By default, rolls happen only on requests whose final input item is a new user message. All known external tool calls must have corresponding outputs. The inline backend's opt-in `--inline-tool-boundaries` also permits scheduling immediately after a complete tool-result batch, with the entire effective input intact. Mid-tool-cycle requests retain the previous projection and complete active tail. A size trigger never authorizes deleting a pending call or its reasoning.

An accepted roll requires a nonempty canonical output containing an encrypted compaction item and at least `--min-savings` wire-size reduction. If compaction times out, fails, returns malformed output, or does not reduce the prefix enough, the existing projection and full remaining history are used. There are no automatic retries of the main generation request.

`x-ostk-roll: 1` forces an attempt at the next eligible boundary, bypassing size and cooldown checks. It does not bypass tool-pair safety, an enabled economic gate or the output acceptance gate. `--idle-roll-seconds` enables an optional inactivity heuristic; it is disabled by default. Time is never used to assert that an upstream cache has expired. The optional [economic decision hook](docs/working-state.md#roll-only-when-the-boundary-and-estimate-allow-it) accounts for caller-estimated checkpoint, cache and recovery costs.

```sh
./target/release/astral proxy \
  --roll-bytes 160000 \
  --keep-recent-turns 2 \
  --min-compact-bytes 32000 \
  --min-roll-seconds 60 \
  --min-savings 0.15
```

These are **byte budgets**, not tokenizer estimates or model context limits. Select a trigger with enough room for instructions, tool schemas, the active tail, and output. An incomplete tool cycle cannot be compacted by the proxy. The provider can still return a context-limit error. The proxy does not truncate input to hide that error.

## GPT cache policy

Policy snapshot: 2026-09-10. Model support is deliberately explicit in `src/policy.rs`.

| Model/wire | Proxy behavior when the caller omitted cache controls |
| --- | --- |
| Known GPT-5.6 / GPT-6 Astra families on API Platform | Adds `prompt_cache_options: {mode: "implicit", ttl: "30m"}` |
| Known earlier models on API Platform | Preserves provider/org defaults; `--retention extended` opts into `24h` |
| GPT-5.5 on API Platform | Rejects an `in-memory` override |
| Unknown model | Preserves defaults; rejects unverified retention overrides |
| Compatible backend, including ChatGPT OAuth | Injects no Platform retention fields; autonomous compaction needs `--allow-compatible-compaction` |

Caller-supplied cache controls and keys win. Generated keys are stable per credential/project/session/model and never contain an epoch or timestamp. The proxy leaves explicit breakpoints and tool ordering intact. A retained prefix is an opportunity for a cache hit, not a guarantee.

The official [prompt caching guide](https://developers.openai.com/api/docs/guides/prompt-caching) documents the model-generation differences, retention behavior, and cache-write reporting. Verify capabilities when adding another family; do not extend the modern policy to every unknown future model.

For compatible backends, `--allow-compatible-compaction` is an operator assertion that the configured endpoint implements the native compact contract. It does not add authentication or demonstrate that the backend supports that endpoint. A Platform hostname embedded in a different URL never activates Platform policy.

## Persistence and failure behavior

Lanes are isolated by upstream, compact path, compaction backend and inline threshold, effective credentials (`Authorization`, `api-key`, and `x-api-key`), ChatGPT account (`chatgpt-account-id`), OpenAI project and organization, session, and model. Each lane is serialized for the complete upstream response lifecycle; distinct lanes run concurrently. Duplicate identity headers are rejected. Backends with additional account-selection headers need those headers added to the identity contract before sharing state across accounts.

Only a successful HTTP response with a completed response object/event and clean EOF commits a candidate. A network failure, truncated SSE stream, incomplete response, or dropped downstream body leaves the previous snapshot in effect. Snapshots use atomic rename after file sync; Unix also syncs the containing directory. A process lock prevents two proxies writing the same directory.

On restart, snapshots load lazily. Matching full history resumes the previous projection. Edited, truncated, or branched history resets to the caller's new history. Changes to instructions, tools, model settings, or other prompt contract fields also reset it. Stream settings and metadata are excluded from this fingerprint. Corrupt snapshots produce an error instead of silently dropping history.

The default directory is `.ostk-gpt/`:

```text
lanes/<hash>.json    committed projection and original-item hashes
ledger.jsonl        observed response and compaction accounting
process.lock        single-writer lock
```

Projection files can contain provider-retained user text as well as encrypted state. The directory is owner-only on Unix. Keep it private, exclude it from git, and remove it when its data is no longer needed. Lane count defaults to 1,024 with a hard admission limit. There is no automatic disk garbage collection; use another directory or archive expired lanes while the process is stopped. Each snapshot and request is size-limited. Ledger retention is operator-managed.

## Baseline and accounting

```sh
./target/release/astral proxy --mode passthrough --state-dir .ostk-gpt-baseline
./target/release/stats --ledger .ostk-gpt/ledger.jsonl
```

Passthrough performs **no request-body rewrite** when `--native-tool-binding` is disabled
(the default), including malformed JSON, force-roll headers, reminders, and
existing cache controls. The explicit native tool binding modes have their own
documented transformation contract. Normal HTTP hop-header handling still
applies. Streaming response bytes are forwarded as received; a bounded observer
reads completion and usage without rebuilding SSE frames.

The stats binary groups generation and compaction calls separately. It reports inclusive input, cache reads, reported writes, output, and measured wire bytes. Cache hit rate is `cached_tokens / input_tokens`. Missing write counts remain unreported. Rejected compact outputs include a `rejection_reason` in the ledger, without copying their contents. It emits no fixed model prices or hypothetical dollar savings.

A client disconnect can prevent final usage from arriving. Such calls cannot be fully billed from this local ledger; use provider billing for reconciliation. Compaction calls that completed before cancellation remain separately recorded. `first_byte_ms` and generation elapsed time start after internal compaction; compaction latency has its own row. Neither is presented as total user-perceived latency.

See [architecture](docs/architecture.md), [evaluation protocol](docs/evaluation.md), and [validation record](docs/validation.md).

## Development

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked --bins
python3 -m unittest discover -s tests -p 'test_*.py'
```

Integration tests use real loopback HTTP servers with mocked Responses/compact payloads. They require no OpenAI credential and make no model calls. The CI workflow contains Linux, macOS, and Windows jobs; only Linux was executed during initial development.

## Scope

The proxy implements native opaque projections. The optional explicit host adds human-readable state, narrow deterministic check adapters and recall tools; it does not intercept arbitrary tools or decode provider reasoning. Verification covers declared inputs and configured checks. Natural-language constraint interpretation, arbitrary command sandboxing, artifact garbage collection and autonomous economic estimates remain outside this reference implementation. Neither layer claims lossless semantic compaction. The official [compaction guide](https://developers.openai.com/api/docs/guides/compaction) describes the native output contract.
