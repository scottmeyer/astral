# Live evaluation protocol

Local tests establish protocol and state-machine invariants. They do not establish model quality, cache availability, spend reduction, or a production throughput claim.

## Experiment arms

Run matched tasks on the same model snapshot, tools, instructions, reasoning settings, and initial repository:

1. `--mode passthrough`: the untouched client request-body baseline.
2. `--mode rolling`: native projections at the configured byte boundaries.
3. Optionally, the client's own native compaction as a third arm. This proxy passes `context_management` and `previous_response_id` traffic through, so combining them does not accidentally create two compactors.

Separate credentials/projects or independently scoped cache keys as appropriate to avoid one arm priming the other. Keep each arm's key stable. Randomize arm order across independent tasks. Label cold starts and rollovers; report them as well as steady operation. Do not remove cache dips just because they lower the result.

## Score the actual task

Record completion against a predefined rubric or executable acceptance check. Include constraints introduced well before rollover, corrections of prior facts, unresolved decisions, and references to earlier artifacts. Include multi-call tools, tool errors, images/files when supported, long reasoning cycles, user edits, and process restart.

For each task, collect all generation and compaction usage, errors, retries, total wall time, final acceptance result, and any user intervention. Distinguish HTTP retries from extra model calls needed to recover forgotten facts. Never judge quality from prompt-size reduction alone.

## Compare economics

Use provider billing and current rates for the actual model. Treat reported input as inclusive and keep read and write reporting separate. Add compactor charges and output/reasoning costs. Cache writes are not assumed free for all GPT generations. Avoid dollar claims on subscription/OAuth backends unless the billing relationship is established independently.

Primary outcome: total verified spend per successfully completed task, alongside success rate. Secondary outcomes: end-to-end latency, provider-observed hit rate, compaction frequency, transport bytes, and extra recovery calls. Report sample sizes and uncertainty. Byte ratios are not token ratios.

## Suggested release gate

Choose the allowed task-quality regression and minimum cost improvement before looking at results. Require no loss of active tool items, no state commits on failed or interrupted generations, and no cross-session projection reuse. The current [live receipt](validation.md) verifies generation, tool execution, and full-history recall but fails native rollover on the available gateway. Promotion still requires an accepted recursive projection and its reuse after restart on the intended client/backend. An empty projection surviving restart does not satisfy that gate.
