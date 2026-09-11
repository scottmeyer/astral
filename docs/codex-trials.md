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

Some installations require an offline model catalog during nested startup:

```bash
codex debug models --bundled > /tmp/astral-models.json
# Add --catalog /tmp/astral-models.json to the trial command.
```

The runner disables request compression, WebSockets, provider-managed context, apps, and plugins for these sessions. It keeps the full input-array contract and uses a stable `x-ostk-session-id` per arm. It supplies a provider override for each fresh or resumed invocation; it does not edit the user's Codex configuration. The CLI must support `exec resume`, `--ignore-user-config`, and the documented custom-provider settings used in the script.

## Reading results

The output directory must be new. `report.json` contains arm order, pass/fail gates, end-to-end invocation time, and separate generation/compaction usage totals. Missing cache-write records remain explicitly unknown. `result.json` within each arm is also updated during the run. Raw CLI events, stderr, and private proxy state stay in that directory, which is excluded from git by default. No projection bodies or credentials are included in the report.

Compare both arms and retain failed trials. A single successful fixture does not establish semantic equivalence, production cache-hit rates, or spending improvements. See [evaluation.md](evaluation.md) for a broader matched-task protocol.

## Current environment receipt

On 2026-09-10/11, this workspace's nested Codex CLI stalled in its filesystem-helper startup before emitting its first request. A separate attempt to match the outer execution mode was rejected by the nested runtime's requirements. The bounded runner records this as a failure; no live model or compaction result is claimed. The local Rust HTTP tests exercise recursive rollover, complete tool-history preservation, snapshot restart, and the failure paths independently.
