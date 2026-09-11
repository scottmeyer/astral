# Catalog forwarding and code health

Date: 2026-09-11. Implementation based on `db29ff4`. Checks below were rerun in
the integration checkout on macOS; earlier receipts remain historical.

## Model discovery

The managed proxy previously returned 404 for Codex's model-catalog refresh.
Explicitly selected supported models still ran, which made the refresh error
nonfatal. Astral now forwards `GET /models`, `/v1/models`, and
`/backend-api/codex/models` to its configured upstream base plus `/models`.
The raw query, including `client_version`, is preserved.

Model discovery shares inference's identity validation, authentication fallback,
header filtering, configured TLS roots, redirect policy and request deadline.
Provider status, end-to-end response headers and body bytes are preserved.
The body is collected within `max_body_bytes` before returning it. Transport
failure, timeout, interruption or an oversized body returns a sanitized 502;
partial provider content is not returned as a successful catalog. No catalog
cache, model rewriting, inference observer, native transformation, projection
lane or usage-ledger entry is involved. Other unrecognized routes remain 404.

## Module boundaries

| Area | Before | Current responsibilities |
| --- | --- | --- |
| Codex adapter | `codex.rs`, 1,047 lines | 24-line public facade; `options.rs` (297) parses caller arguments; `rpc.rs` (437) owns processes, bounded RPC and shutdown; `staging.rs` (405) defines fresh, native and worker flows |
| Project resolver | `project.rs`, 1,418 lines | Resolver/validation stays in `project.rs` (947); `schema.rs` (225) holds public types and limits; `reader.rs` (271) owns confined reads and read budgets |
| HTTP proxy | `proxy.rs`, 672 lines | Routing/inference stays in `proxy.rs` (569); `http.rs` (184) shares forwarding helpers and implements catalog GETs |

Public Rust import paths remain stable. The staging flows share initialization,
context-message construction, proxy configuration checks, start/resume operations,
and process cleanup. Their policies remain distinct: native import retains raw
JSON item serialization, its 24 MiB request allowance and exact model/provider
checks; ordinary staging retains its 4 MiB request bound. Fresh document staging
does not gain the worker path's model/provider requirements. Cleanup preserves
the primary failure and still requires clean shutdown before reporting success.
Ownership callbacks remain after preflights and immediately before thread creation.

This is a responsibility split, not a net reduction in lines. The public facades,
named helpers, documentation and catalog behavior add code. Private receipts,
confined document readers and publication writers retain separate filesystem
policies; combining them into a general file helper would obscure different
ownership, mutation and failure requirements. `workspace.rs` and
`projection_save.rs` remain the largest modules, with their Unix implementations
still inline. Further extraction should follow a coherent responsibility and
preserve those policies rather than use a line-count threshold alone.

The Codex split was implemented by a real worker launched through Astral with
selected project/work context, its own bound branch/worktree and a bounded file
assignment. Parent integration and an independent read-only review checked the
result against the original adapter. The worker's isolated tests were not used
as a substitute for the complete integration checks.

## Package review

The executable remains `astral`; the existing package/library name
`ostk-gpt-cache` remains compatible. The package description now includes project
contexts, bound worktrees and native handoff. All direct dependencies have source
uses; no dependency or crate split was justified by this review.

The direct `futures-util` dependency now explicitly enables `sink`, required by
the transport's `SinkExt` use. That feature was already active transitively;
the declaration no longer depends on another dependency enabling it. There are
no dependency upgrades or lockfile changes in this pass. Existing multiple
versions of `getrandom`, `syn` and `webpki-roots` were reviewed as dependency-graph
constraints, not blindly overridden. Broad Tokio/Axum feature settings remain.

The declared minimum remains Rust 1.85. The locked graph and all targets check
successfully using Rust 1.85.0, and CI now includes that check alongside stable
Linux/macOS/Windows jobs. This pass did not run a vulnerability/advisory audit or
execute those remote CI jobs. It does not claim every supported platform was
verified locally.

## Current checks

- Formatting and whitespace checks pass.
- Strict Clippy passes across all targets with the lockfile fixed.
- All **304 Rust tests** pass; none failed or were ignored.
- Locked release builds of all binaries pass.
- All **18 Python tests** pass.
- `cargo +1.85.0 check --locked --all-targets` passes.

Five new catalog tests use real local HTTP transports: all aliases, exact query
and body bytes, identity/conditional headers, duplicate-header rejection, account
separation without a cache, provider errors/redirects/304, byte limits, deadlines,
interrupted bodies and absence of conversation state. Two additional staging
regressions exercise the distinct request-size and runtime-identity policies.
Existing native, proxy, project, worktree, publication and process tests all ran.

An early catalog invocation encountered an incomplete module move while an agent
was still editing. The corrected fixture also sets a small valid roll threshold
when testing a small body cap. The worker's launch tests first encountered host
Git signing configuration, then three managed-proxy startup failures under its
workspace-write sandbox. All 11 launch tests passed in the parent checkout; its
test command disabled fixture commit signing without changing host Git settings.
These failed intermediate runs are retained in private evidence.

The required staged static UBS scan covered 11 Rust files and completed without
failed scanner modules. It exited 1 with **4 critical, 383 warning and 197
informational** heuristic matches; this is not a clean scan. Source review found
the critical matches were the unchanged confined reader's `assume_init`, guarded
by successful `fstatat`, and three outgoing request-header builder calls mistaken
for request-derived response headers. Those calls use typed `HeaderName` and
`HeaderValue` inputs or static values after the existing filtering policy.

Reviewed warnings include test assertions/unwraps and fixture mutexes, validated
resolver indices, bounded allocations and the response builder's already-typed
status/headers. No introduced defect was confirmed and no scanner suppression
was added. UBS's Cargo/dependency-audit phases were explicitly skipped using
`--no-cargo`; the independent Cargo checks above ran in the full checkout.

## Live catalog control

Installed Codex **0.154.0** ran an owned app-server against the rebuilt Astral
proxy, with the built-in OpenAI provider and existing runtime-supplied account
authentication. A private loopback relay recorded only request paths, response
status, byte/hash counts and model counts, without logging headers or catalog
payloads. It observed two actual requests to
`/backend-api/codex/models?client_version=0.154.0`: both returned **HTTP 200**,
**392,252 bytes**, and **8 models**, including `gpt-6-astra`. Astral's request
counter advanced from 0 to 2, and Codex's `model/list` returned all 8 models.
No inference was started for this control. This establishes actual upstream
catalog forwarding rather than a response served only from Codex's local cache.

The rebuilt launcher also imported the repository's explicit native review
projection into a new disposable thread, then cold-resumed that same thread with
a new owned proxy. Both runs produced one actual terminal `pwd` invocation,
exit 0, and the exact integration working directory. They used read-only sandbox
policy and never approval; neither stderr contained a catalog 404. These smoke
tests check current native launch/execution continuity, not exact opaque recall,
compaction/save, or denial enforcement, which were not rerun live in this pass.

The live result applies to that runtime, route and account. Synthetic transports
cover other response shapes; they do not establish live cross-account switching
or broader native-checkpoint compatibility. Private commands, event streams and
verification logs remain outside tracked source. Authentication and original
recovery sessions were not read or patched.
