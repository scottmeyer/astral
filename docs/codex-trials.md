# Fresh Codex trials through Astral

`scripts/codex_trial.py` creates two proxy processes with separate state directories and separate Codex conversations. Each conversation resumes across six fresh CLI invocations. The custom model provider is named `trial`; its `base_url` points to that arm's loopback proxy. The runner uses the installed CLI's normal authentication, with no credential-file access or copying.

The experiment loads synthetic facts through large tool results, corrects an old value, checks recall without tools, adds a second large tool result, checks recall again, restarts the proxy, and checks recall once more. It requires at least two accepted rollovers, complete generation commits, and reuse of the same projection across restart. A trial with no observed requests or no rollovers fails. This is a small correctness experiment, not a cost or quality benchmark.

## Prepare without model calls

```bash
python3 scripts/codex_trial.py \
  --model gpt-6-astra \
  --auth api-key \
  --upstream https://api.openai.com/v1 \
  --prepare-only \
  --output .astral-trials/prepared
```

Python 3.11+ and a recent Codex CLI are required for execution. The script intentionally accepts only `read-only` or `workspace-write` sandbox modes and preserves runtime requirements. It does not attempt to bypass a blocked nested runtime. The default is `read-only`; the task only reads its two fixture files.

Use `--codex /absolute/path/to/codex` to choose an isolated CLI installation. This workspace installed the official npm package `@openai/codex@0.154.0` and verified its normal ChatGPT login without reading or copying authentication files.

## API-key trial

Build the release binaries, set `OPENAI_API_KEY` through your normal secret-management method, and run:

```bash
cargo build --release --locked --bins
python3 scripts/codex_trial.py \
  --model gpt-6-astra \
  --auth api-key \
  --upstream https://api.openai.com/v1 \
  --output .astral-trials/api-trial
```

Use `--model` to select a model snapshot your account supports. Execution makes live, potentially billable generation and compaction calls. `--timeout` bounds each CLI invocation; transport retries are disabled for this experiment. Each arm has six user turns plus tool continuations and eligible native compaction calls. The runner stops on a setup failure so it does not repeatedly launch a broken configuration.

## Existing ChatGPT login

Use `codex login status` to confirm the intended CLI login. Supply the Responses base URL configured for that login. For a compatible backend, explicitly opt into its native compact endpoint:

```bash
python3 scripts/codex_trial.py \
  --model gpt-6-astra \
  --auth chatgpt \
  --upstream https://chatgpt.com/backend-api/codex \
  --allow-compatible-compaction \
  --output .astral-trials/oauth-trial
```

Backend availability and routing depend on the runtime. A managed installation may use a different configured upstream. The example is not a compatibility claim. ChatGPT OAuth does not activate API Platform retention controls.

Set `--upstream` to the base URL actually configured for the runtime. A private upstream CA can be supplied through `SSL_CERT_FILE`; Astral keeps certificate verification enabled. `--compact-path` changes the native endpoint suffix, whose default is `/responses/compact`. Do not infer native compatibility from an HTTP 200 alone: the response must include the native encrypted `compaction` item. Ordinary `reasoning` items do not meet that contract.

Some installations require an offline model catalog during nested startup:

```bash
codex debug models --bundled > /tmp/astral-models.json
# Add --catalog /tmp/astral-models.json to the trial command.
```

The runner disables request compression, WebSockets, provider-managed context, apps, and plugins for these sessions. It keeps the full input-array contract and uses a stable `x-astral-session-id` per arm. It supplies a provider override for each fresh or resumed invocation; it does not edit the user's Codex configuration. The CLI must support `exec resume`, `--ignore-user-config`, and the documented custom-provider settings used in the script.

## Reading results

The output directory must be new. `report.json` contains arm order, pass/fail gates, end-to-end invocation time, and separate generation/compaction usage totals. Missing cache-write records remain explicitly unknown. `result.json` within each arm is also updated during the run. The proxy's ingress counter is captured before each shutdown, so a client that never sends a request can be distinguished from an upstream failure. Raw CLI events, stderr, and private proxy state stay in that directory, which is excluded from git by default. No projection bodies or credentials are included in the report.

Compare both arms and retain failed trials. A single successful fixture does not establish semantic equivalence, production cache-hit rates, or spending improvements. See [evaluation.md](evaluation.md) for a broader matched-task protocol.

## Current environment receipt

On 2026-09-11, stable Codex CLI 0.154.0 reported an existing ChatGPT login and successful provider connectivity, but `codex exec` timed out after 45 seconds before emitting events or reaching Astral (`requests_received: 0`). Earlier alpha CLI attempts also stalled during nested startup; an attempt to match the outer execution mode was rejected by the nested runtime's requirements. No requirements were bypassed. The runner records timeout as failure even when the terminated child reports exit code 0.

The standalone client subsequently completed all 16 generation calls and six recall checks across both arms, including restart. All five alternate-route compaction attempts were rejected, so recall used full history and the overall trial failed. Those low-effort calls produced no encrypted reasoning items. A later [inline trial](opaque-replay.md) succeeded using `context_management` on `/responses`, including two native compactions and recall with the original facts absent from plaintext. Astral now also supports an opt-in autonomous inline backend; its [three-arm benefit trial](benefit-evaluation.md) completed 66 calls and validated its own checkpoint adoption and restart on two task seeds.

## Responses client independent of Codex startup

`scripts/api_trial.py` runs a six-turn matched test with actual function calls and two synthetic tool results of approximately 108 KB each. It replays complete native output items, checks a corrected budget and older facts without tools, then checks again after proxy restart. It requires two accepted native projections and a nonempty projection reused after restart. The client also handles gateways that emit canonical items in `response.output_item.done` but leave the terminal response's output array empty.

```bash
python3 scripts/api_trial.py \
  --model gpt-6-astra \
  --auth api-key \
  --upstream https://api.openai.com/v1 \
  --output .astral-trials/api-client
```

The client launches each proxy together with its requests. Each arm receives a stable cache key and a distinct instruction prefix because a managed gateway may override the supplied cache key. The task and tools otherwise match. This reduces cross-arm prefix reuse without asserting control over the provider's cache.

`--auth gateway` sends no credential and is only for an already authorized gateway that supplies its own routing. The routing diagnostic performed in this workspace was:

```bash
python3 scripts/api_trial.py \
  --model gpt-6-astra \
  --auth gateway \
  --upstream https://chatgpt.com:18080/backend-api/codex \
  --allow-compatible-compaction \
  --compact-path /compact \
  --output .astral-trials/gateway-diagnostic
```

That diagnostic **failed the rollover gate**. Generation worked, but this gateway's `/compact` returned ordinary generation output. Its standard `/responses/compact` route returned 404 in an earlier trial. Neither result establishes native compaction support, and `/compact` should not be treated as a working native replacement on this gateway.

## Inline native compaction and opaque replay

`scripts/inline_trial.py` uses the provider's native inline mode over the working `/responses` route. It runs four user turns with two actual fixture reads, a correction, two compactions, and recall after restarting the proxy and reloading the client window. Random facts are never echoed before the final question, and the test rejects a final request that still contains those strings in plaintext.

```bash
python3 scripts/inline_trial.py \
  --model gpt-6-astra \
  --auth gateway \
  --upstream https://chatgpt.com:18080/backend-api/codex \
  --threshold 8192 \
  --output .astral-trials/inline-canaries
```

For Platform, use `--auth api-key` with your configured Platform upstream. The threshold is a provider token threshold; it is distinct from Astral's byte trigger. Native support remains backend-dependent. Astral passes these `context_management` requests through, so the provider owns compaction. The result tests that path and does not certify Astral's autonomous planner. See [opaque replay](opaque-replay.md) for the exact evidence and handling rules.

## Matched benefit trial

`scripts/benefit_trial.py` compares full history, provider-owned inline compaction, and Astral-scheduled inline checkpoints. The client retains complete original history in the Astral arm, allowing the proxy to own and persist its mapping. Two default seeds each run nine user turns with eleven generation calls, two fixtures, two rollovers in each compacted arm, and final recall after restart. Complete commands, measured input/output/latency, and cache limitations are in [benefit-evaluation.md](benefit-evaluation.md). This client is independent of the blocked nested `codex exec` startup.
