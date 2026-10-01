# Shared proxy live verification — macOS, 2026-10-01

**Real archival and model recall passed with explicit full-history Codex HTTP and
Claude Code configurations. Ordinary Codex and Claude Code demonstrated routing
and tool execution, but did not activate archival.** One compatibility defect
was reproduced and fixed on the test branch: Responses `input_text` tool-output
blocks were previously ignored.

**The follow-up review workload performed worse with tools mode.** Passthrough
completed four turns and all ten factual checks in 234 seconds. Tools mode
archived and recalled outputs successfully, but timed out at 240 seconds during
its first turn. Its observed uncached input was already at least 4.90 times the
entire baseline. This is one diagnostic workload, not a general performance result.
The reviewer also found that the original qualification's final prompt favored
archive retrieval: passthrough was told to return `NO_ARCHIVE_HANDLE` rather than
attempt recovery from retained history. See [the follow-up review](#follow-up-review-of-the-test).

**A separate intake prototype showed a narrower benefit.** With the same focused
review task in both arms, omitting known padding before it entered history used
15.08% fewer total input tokens and 18.70% fewer uncached input tokens. Both scored
10/10; cached input remained about 88%. This is one synthetic pair using a private
MCP adapter, not a shipped Astral feature or a default Codex archival result.

## Baseline and changes

- Repository: `scottmeyer/astral`.
- Starting commit: `4806ec2374abce9da19f656775ca29313721eef8`.
- Starting checkout: clean `main`. `git fetch origin` succeeded;
  `git rev-list --left-right --count HEAD...origin/main` returned `0 0`.
  No fast-forward was necessary. Earlier implementation/close-metadata patches
  were already present and were not reapplied.
- Review branch: `test/shared-proxy-live-20261001`.
- Instructions: `/Users/scottmeyer/projects/AGENTS.md`; no repository or nested
  `AGENTS.md`/`CLAUDE.md`. Read both existing universal-proxy documents.
- Laptop: macOS **26.6.2**, build **25G83**, Darwin **25.6.0**, arm64.
- Rust/Cargo: **1.97.1**; minimum supported Rust check: **1.85.0**.
- Codex: **0.159.2**, `/Users/scottmeyer/.local/bin/codex`.
- Claude Code: **2.1.286**, `/Users/scottmeyer/.local/bin/claude`.
- Python: **3.14.7**. Controller uses the standard library.
- Models observed: Codex **gpt-6-astra**; Claude **claude-opus-5-5**.

Both logins worked using normal existing authentication. `codex login status`
reported ChatGPT. `claude auth status` reported `claude.ai` with keychain access.
The sandboxed Claude check had incorrectly appeared logged out; checking outside
the sandbox resolved that. No credential files were printed, copied, or changed
by the controller. No API-key environment variable was needed.

All documented commands ran on this laptop, before and after the fix:

| Command | Original commit | With fix |
| --- | --- | --- |
| `cargo fmt --all --check` | PASS | PASS |
| `cargo clippy --all-targets --locked -- -D warnings` | PASS | PASS |
| `cargo test --locked` | PASS, 517 tests | PASS, 519 tests |
| `cargo build --release --locked --bins` | PASS | PASS |
| `cargo +1.85.0 check --locked --all-targets` | PASS | PASS |
| `python3 -m unittest discover -s tests -p 'test_*.py'` | PASS, 18 tests | PASS, 18 tests |

The complete Rust suite included `hook_deadline`; its process/descendant
termination check passed here. No hosted-environment failure was used to skip it.
The focused `tool_history`, `universal_proxy`, and `native_transport` suites also
passed after the correction. Python reproduction scripts passed syntax and Ruff
checks; the driver and page verifier were exercised against the live listener.

## Configuration outcomes

“FAIL” for archival below means the requested archival qualification did not
pass; intentional preservation of provider-managed history is not a proxy error.

| Harness configuration | Live routing + tools | Actual archival | Agent `astral_recall` + correct marker |
| --- | --- | --- | --- |
| Direct normal Codex control | PASS | NOT TESTED: no proxy | NOT TESTED |
| Normal Codex routing control, routing/compression overrides only | PASS | NOT TESTED: small task | NOT TESTED |
| Normal Codex + temporary fixture/retrieval MCP, before and after fix | PASS | FAIL: none | FAIL: `NO_ARCHIVE_HANDLE` |
| Explicit Codex HTTP, original implementation, approved MCP tools | PASS | FAIL: `input_text` not recognized | FAIL: `NO_ARCHIVE_HANDLE` |
| Explicit Codex HTTP, corrected implementation | PASS | PASS | PASS |
| Two concurrent Codex HTTP projects, corrected implementation | PASS, both | PASS, both | PASS, both |
| Normal Claude Code history management + temporary MCP | PASS | FAIL: provider-managed history | FAIL: `NO_ARCHIVE_HANDLE` |
| Claude Code with explicit `context_management=null` | PASS | PASS | PASS |
| Matched Codex HTTP passthrough control | PASS | PASS: zero, as expected | NOT TESTED: no archive exists |
| Codex/Claude retrieval after restarting Astral with the same state | PASS | Existing artifacts retained | PASS, both |

Live Chat Completions and full-history WebSocket archival: **NOT TESTED**.
Their local protocol tests passed. No default interactive Codex context-reduction
claim follows from the HTTP diagnostic.

### One shared listener

The main qualification used the standalone release binary, never the managed
`astral project ... --proxy` launcher:

```sh
./target/release/astral proxy \
  --listen 127.0.0.1:18088 \
  --mode tools \
  --state-dir /private/tmp/astral-live-20261001/shared-state
```

The port was unused at startup. All providers and project workloads used that
listener. Default thresholds remained unchanged: **16 KiB** minimum result,
**2 KiB** preview, **two** recent user turns, **four** recent results, latest
parallel batch preserved, **512 MiB** archive limit.

The proxy was restarted with the fixed release binary using the same directory.
It was subsequently restarted in `passthrough` mode for the matched control and
another retrieval check. Only the test-owned listener was stopped/restarted.

### Direct and ordinary Codex

The direct control used the existing user configuration, including its model and
authentication. It completed a real shell call, exit 0, and returned **44461**:

```sh
codex exec --skip-git-repo-check --ephemeral --json \
  -C /private/tmp/astral-live-20261001/direct \
  'Run the shell command python3 -c "print(173*257)" exactly once and report the computed integer. Do not use other tools.'
```

The routed control added only:

```sh
-c 'openai_base_url="http://127.0.0.1:18088/providers/codex/backend-api/codex"' \
-c 'features.enable_request_compression=false'
```

It also returned **44461**, with the command completed successfully and Astral's
`requests_received` increasing from **0 to 4**. This establishes inference access
and named-route authentication, not merely successful CLI startup.

The ordinary four-turn workload retained the user configuration and added the
two temporary MCP servers. Both original and fixed binaries completed all turns
and seven fixture/pulse calls, but produced no fixture archive. The original
isolated Codex workload generated **28** `tool_history` records, including **18**
`provider_managed_history` skips and zero archives. The repeated normal workload
also ended `NO_ARCHIVE_HANDLE`; none of the saved artifacts matched its fixture.

Ordinary traffic exercised the WebSocket path. Its ledger has no complete
forwarded-byte or provider-usage accounting. The observed skip reason establishes
provider-owned/incremental-history handling; it does not identify which specific
ownership field was present in each request. An HTTP configuration is a separate
qualification, not an assertion about the normal transport.

Existing unrelated MCP OAuth refresh warnings appeared for some installed
connectors. They did not prevent the direct/routed tasks. No unrelated connector
tools were called by the workload.

### Explicit Codex HTTP diagnostic

The installed CLI accepted these process-local settings, with its existing
ChatGPT login retained:

```toml
model_provider = "astral_trial"
model_reasoning_effort = "low"
features.enable_request_compression = false

[model_providers.astral_trial]
name = "Astral HTTP test"
base_url = "http://127.0.0.1:18088/providers/codex/backend-api/codex"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false

[model_providers.astral_trial.http_headers]
x-astral-workspace-id = "<workload-label>"
x-astral-session-id = "<workload-label>"
x-astral-harness-id = "codex-live-test"

[mcp_servers.astral]
command = "/Users/scottmeyer/projects/astral/target/release/astral"
args = ["mcp", "--proxy-url", "http://127.0.0.1:18088"]
default_tools_approval_mode = "approve"

[mcp_servers.fixture]
command = "/Library/Frameworks/Python.framework/Versions/3.14/bin/python3"
args = ["<fixture_mcp.py>", "<one-shot-fixture-path>"]
default_tools_approval_mode = "approve"

[mcp_servers.fixture.tools.fixture_once]
output_token_limit = 20000
```

The driver passes these with `-c` and uses
`codex -C WORKSPACE exec [resume] --skip-git-repo-check --json
--ignore-user-config -m gpt-6-astra ...`. Initial and resumed invocations use the
same settings. Exact argument arrays are saved per turn. The
[official configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
documents provider transport/auth settings, MCP approval modes, and per-tool
output budgets. The installed CLI's help and actual successful requests were the
version-specific checks.

The first exploratory HTTP run lacked MCP approvals. Codex emitted failed tool
calls even though its text said `LOADED`/`AGED`. That run is **FAIL**, excluded from
qualification. Explicit approval for the four local test tools fixed the harness
setup. Successful subsequent HTTP requests had `skipped:null` in the reduction
ledger; after the code correction they actually archived results.

### Claude Code

The driver used normal keychain authentication with:

```sh
ANTHROPIC_BASE_URL=http://127.0.0.1:18088 \
claude -p --output-format stream-json --verbose \
  --strict-mcp-config --mcp-config /absolute/path/to/temporary-mcp.json \
  --tools '' \
  --allowedTools 'mcp__fixture__fixture_once,mcp__fixture__pulse,mcp__astral__astral_search,mcp__astral__astral_recall' \
  -- 'PROMPT'
```

Later turns add `--resume SESSION_ID`. `--` is necessary because the tool option
accepts multiple arguments; the first controller attempt omitted it and failed
before inference. That setup failure was corrected and excluded from results.

With normal history management, **11/11** requests were successful HTTP 200
responses but were skipped as provider-managed history. The separately labeled
full-history diagnostic added:

```sh
CLAUDE_CODE_EXTRA_BODY='{"context_management":null}'
```

This override is supported by the installed CLI's request-construction code.
It was process-local. The actual diagnostic produced **13/13** eligible
`skipped:null` records, **seven** archive replacements, and successful real MCP
search/recall. Disabling explicit context-management ownership is a meaningful
configuration change; this result is not normal Claude context reduction.

## Workload, exact recovery, and isolation

The controller generated **33,708 UTF-8 bytes** per fixture. Three independent
160-bit random marker values were placed at byte offsets **9,386**, **16,887**,
and **26,247**, far outside the retained head/tail preview. Values and hashes
were stored only in the controller, not in user prompts. The matched arms reused
identical fixture bytes.

Four user turns:

1. Call `fixture_once`; reply `LOADED` without repeating marker values.
2. Make six sequential `pulse` calls; reply `AGED`.
3. No tools; reply `READY`.
4. Search for `hidden_marker_beta`, actually call `astral_recall`, and return its
   value; report `NO_ARCHIVE_HANDLE` if no archive marker exists.

The fixture server deleted its source on first consumption and could not return
it again. Test agents were instructed to use only fixture/retrieval tools; the
recorded completed tool calls show no shell/file/transcript recovery. Expected
values stayed outside their workspaces. Codex sometimes grouped pulse calls
inside a code-mode result, so six MCP calls were not necessarily six Responses
result items. The later user turns independently satisfied default age retention.

For every successful archival run, HTTP paging reconstructed the complete stored
body, verified its SHA-256, decoded its JSON wrapping, and found the **entire
original fixture byte-for-byte**. Thus the harness had not truncated the fixture
before Astral received it. Each artifact took nine pages at a 4,093-byte page size.

| Live artifact | Stored bytes | SHA-256 of recovered stored body |
| --- | ---: | --- |
| Codex matched tools | 34,368 | `3d833e333916da3e07c24c767400a2bd6754c0b13cf200666ddb521d69b94766` |
| Concurrent project A | 34,980 | `b795fd3e01bbe467a8b1b1cb59b9c90200e8bd235f98385dff69f2b1128f1e05` |
| Concurrent project B | 34,368 | `fa52810ce0fd4c43ce5e2a7e002f29c88c8780e3d7815d459eb5626113c2b2c6` |
| Claude full history | 34,280 | `72469bfd1bd8909da65b316f9be42c959f135f802e3dbfc10bffb9d36cbd072d` |

Project A and B ran concurrently from independent directories/sessions with
explicit distinct workspace/session headers. Both recovered their correct
markers; their archive scopes and handles differed, and each retrieved only its
own fixture. Their overlapping execution window was approximately **53 seconds**.
The unit suite separately verifies that identical content gets distinct handles
across scopes. Handles remain bearer capabilities: this is content/scope
isolation on a trusted local host, not authentication against someone who knows
another session's handle.

After restarting Astral with the same state, both Codex and Claude called
`astral_search` and `astral_recall` again and correctly recovered
`hidden_marker_gamma`, which had not been requested previously. Exact HTTP
recovery also passed after restart.

## Matched measurements

Fresh Codex sessions used **gpt-6-astra**, **low** reasoning effort, the same
33,708-byte fixture (SHA-256
`f0aa99e0f260af5e7500ffcad8720c1a7fc0d16b89452e7dd84e96bb96af20f3`),
the same four prompts, and the same HTTP/MCP settings. Labels were
`matched-tools-fixed` and `matched-passthrough-isolated`. The passthrough
measurement was run without overlapping restart traffic.

| Metric | Tools | Passthrough |
| --- | ---: | ---: |
| Completed user turns | 4 | 4 |
| Successful fixture/pulse calls | 7 | 7 |
| Unique archived fixture bodies | 1 | 0 |
| Archive replacements across requests | 5 | 0 |
| Reduction-ledger bytes saved across requests | 158,835 | 0 |
| Actual MCP search / recall calls | 1 / 1 | 0 / 0 |
| Correct hidden-marker recall | PASS | No archive handle |
| Tool errors / timed-out turns | 0 / 0 | 0 / 0 |
| **Total forwarded request bytes** | **Unavailable** | **Unavailable** |
| Observed forwarded bytes in completed response ledger rows | 204,461 | 140,346 |
| Response rows containing those byte observations | 2 | 1 |
| Final harness usage: input tokens | 175,963 | 153,835 |
| Final harness usage: cached input tokens | 131,968 | 124,160 |
| Final harness usage: cache-write input tokens | 0 | 0 |
| Final harness usage: output tokens | 410 | 208 |

The forwarded-byte observations cover different incomplete subsets and **must
not be compared as totals**. In the tools arm, nine inspected requests produced
only two completed response ledger rows, despite a successful harness session.
Code inspection explains the coverage limit in that tested binary: HTTP response/usage accounting was
emitted only after the streaming body reaches EOF; a client that ends consumption
earlier can leave that row absent. No additional byte recorder was installed,
so complete forwarded-byte totals for those original runs remain unverified.
The follow-up below adds accounting for new HTTP runs. WebSocket totals are also
unavailable. Missing rows are not zero traffic or zero usage.

The token numbers are the final `turn.completed.usage` counters emitted by each
Codex session. They are not inferred from bytes. The tools trajectory includes
search, recall, and its returned page, while passthrough reports no handle;
model-generated intermediate calls also vary. The final prompt explicitly
requires an archive handle and tells either arm to return `NO_ARCHIVE_HANDLE`
when none exists. Passthrough's response therefore does not demonstrate an
inability to recover the marker from retained history. This qualifies the archive
mechanism but is not a fair task-quality comparison. This sample does **not**
establish lower token use, latency, or billing.

Anthropic had complete response-ledger coverage for its two principal runs:

| Anthropic observation | Normal history management | Explicit full history |
| --- | ---: | ---: |
| HTTP responses / HTTP errors | 11 / 0 | 13 / 0 |
| Request bytes before Astral | 470,744 | 568,952 |
| Forwarded request bytes | 470,744 | 347,332 |
| Archive replacements / unique fixture artifacts | 0 / 0 | 7 / 1 |
| Saved bytes across requests | 0 | 221,620 |
| Provider input tokens, including cache reads/writes | 166,375 | 119,705 |
| Cache-read tokens | 149,637 | 101,858 |
| Cache-write tokens | 16,710 | 17,815 |
| Output tokens | 637 | 749 |
| Recall | No handle | PASS |

These Anthropic runs used different random markers and different history
settings. They demonstrate functionality and observed traffic, not a controlled
billing comparison. Byte savings are not automatically billing savings.

## Confirmed defect and correction

The approved Codex HTTP run on the original binary completed real tools but
archived nothing. Its synthetic rollout showed `custom_tool_call_output.output`
as an array of plain `input_text` blocks, including the complete fixture. Astral's
`text_payload` accepted only `text` blocks.

`codex_responses_input_text_blocks_archive_and_recover_exactly` reproduced this
with default thresholds: expected one archived result, observed zero (exit 101).
The fix recognizes unannotated `input_text` blocks **only for Responses**, while
retaining existing `text` support. Regression coverage verifies exact paged JSON
recovery, an omitted middle marker, unchanged surrounding history, and preservation
of annotations, images, and that unfamiliar shape on other protocols.

The fixed live run then archived the fixture and completed genuine MCP recall.
No WebSocket close-metadata code was changed.

For identifying the tested correction:

- Implementation/test diff from `4806ec2` (`git diff 4806ec2 -- src/tool_history.rs tests/tool_history.rs`) SHA-256:
  `7f10694d931f162a6bb1036d0e060b8e0e5dd7e35c1ae420f246546e1c140702`.
- Tested release `astral` SHA-256:
  `5a843618b2fbb5d32d0ebd65027a64135e41513bc80e6736d3293b4befec7daa`.

## Reproduce and inspect evidence

Private local evidence is retained at:

```text
/private/tmp/astral-live-20261001/
  baseline.json, fixed-validation.json
  logs/                         # build logs, harness JSONL/stderr, status receipts
  controller/*.expected.json   # original synthetic fixture, markers, hashes
  <workload>/command-*.json     # exact process argument arrays
  <workload>/session.json, summary.json
  shared-state/ledger.jsonl
  shared-state/artifacts/       # private immutable artifacts and store key
  paged-recovery.json           # exact paging receipts; contains local handles
```

The directory is private and outside the repository. Keep it while reproducing
or reviewing this run; operating-system temporary-directory cleanup can remove
it. Raw transcripts, credentials, store keys, and retrieval handles are not in
this report or committed files.

A reusable controller is in [`scripts/shared_proxy_live`](../scripts/shared_proxy_live/).
It uses the real CLIs; it neither reads credentials nor implements inference.
It does not invoke the native-rolling `scripts/codex_trial.py`.

```sh
export ASTRAL_TEST_ROOT="$(mktemp -d /private/tmp/astral-live-repeat.XXXXXX)"
export ASTRAL_TEST_URL=http://127.0.0.1:18088
export ASTRAL_TEST_BINARY="$PWD/target/release/astral"

# Start separately, leaving default archival thresholds intact:
"$ASTRAL_TEST_BINARY" proxy --listen 127.0.0.1:18088 --mode tools \
  --state-dir "$ASTRAL_TEST_ROOT/shared-state"

# In another terminal with the same variables:
python3 scripts/shared_proxy_live/workload.py normal-tools
python3 scripts/shared_proxy_live/workload.py matched-tools --http
python3 scripts/shared_proxy_live/workload.py claude-normal --claude
python3 scripts/shared_proxy_live/workload.py claude-full --claude --full-history

# Concurrent independent projects:
python3 scripts/shared_proxy_live/workload.py project-a --http &
a_pid=$!
python3 scripts/shared_proxy_live/workload.py project-b --http &
b_pid=$!
wait "$a_pid" "$b_pid"

python3 scripts/shared_proxy_live/verify_archives.py
```

Stop only that test proxy and restart it with the **same state directory**.
Run the page verifier again. To repeat the agent restart check, take the prior
`command-3.json` argument array, retain its session/settings, replace only the
last prompt with a request to search and recall `hidden_marker_gamma`, and run
it through `run_exec.py`. Claude additionally needs the two environment overrides
shown above. Check the tool events and compare the answer with the controller;
a model's statement that it recalled something is insufficient.

For the matched control, stop the tools listener and restart it with
`--mode passthrough`, then:

```sh
python3 scripts/shared_proxy_live/workload.py matched-passthrough --http
```

Labels beginning `matched` share controller fixture bytes but use fresh harness
sessions. Keep measurement arms isolated in time. The final scripts also record
failed/unexpected tool calls, count fixture/pulse calls, and distinguish
`workload_pass` from `recall_pass` in their summaries.

## Follow-up review of the test

A real Codex session reviewed the saved synthetic test transcripts, controller,
implementation, report, and selected ledger rows. Both comparison arms used the
same release binary with the HTTP accounting correction committed in `1bd3bf6`,
after qualification commit `7297754`, on the same test branch. Model and installed
versions were unchanged. The tools arm ran after explicit approval of the data
transfer to the Codex service; the earlier approval block is resolved.

The reviewer confirmed:

- Seven denied MCP calls in the initial exploratory run were concealed by
  successful-sounding model acknowledgments. The later approved run completed
  tools but still archived nothing before the `input_text` fix.
- Ordinary Codex and Claude Code established routing and tool execution. They
  did not establish context reduction.
- Explicit full-history configurations established archival and exact retrieval.
- The original final prompt was unsuitable for comparing practical usefulness.
- Before that final turn, Codex tools mode used **103,343** input tokens versus
  **125,149** for passthrough: **21,806 fewer (17.42%)**. Entire-session counters
  were **175,963** versus **153,835**: **22,128 more (14.38%)**. These include cache
  reads and are not billing measurements.

### Review workload and outcome

The new workload asks both configurations to review the same six evidence
packets, over four user turns. Each can reread or search the original evidence;
both have Astral MCP available. The final answer requires ten factual fields,
up to five findings with evidence, and a recommendation. It does not require an
archive handle or any particular retrieval method. Expected answers are kept in
the controller, outside the prompts.

| Packet key | UTF-8 bytes |
| --- | ---: |
| `failed_and_prefixed` | 48,681 |
| `normal_harnesses` | 207,457 |
| `tools_arm` | 45,617 |
| `passthrough_arm` | 43,338 |
| `implementation` | 45,774 |
| `report_and_ledger` | 36,942 |

Packets retain completed tool outputs, including repetitive synthetic fixture
text. They are a test-review workload, not a representative coding benchmark.
The reviewers independently reread and paginated packets, so the runs compare
agent trajectories, not identical sequences of tool calls. Both had the same
240-second limit per user turn. The evidence MCP server returns complete strings;
its configured output limit is 200,000 tokens, and the prompt requests code-mode
output limits of 120,000 tokens. All six packets appear intact in the baseline's
local tool records. The tools run reached only the first two, also present intact
in its local records.

Those local records do **not** prove untruncated provider input. Of 21 stored
artifacts, 17 exactly match a recorded tool-output JSON value; four differ and
contain token-truncation or omitted-text markers. The earlier claim based solely
on full local records was too strong. This review does not qualify untruncated
delivery of all raw packets through Codex. The original, smaller one-shot fixture
qualification remains separate. Per-arm `audit.json` files preserve these checks.

Each arm uses a fresh session and state directory, one standalone proxy on
`127.0.0.1:18088`, and default archival thresholds. The process-local settings are:

```toml
model_provider = "trial"
model_reasoning_effort = "low"
features.enable_request_compression = false
features.shell_tool = false

[model_providers.trial]
name = "Review trial"
base_url = "http://127.0.0.1:18088/providers/codex/backend-api/codex"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false

[model_providers.trial.http_headers]
x-astral-workspace-id = "<label>"
x-astral-session-id = "<label>"
x-astral-harness-id = "benefit-review"

[mcp_servers.evidence]
command = "/Library/Frameworks/Python.framework/Versions/3.14/bin/python3"
args = ["/private/tmp/astral-benefit-20261001/evidence_mcp.py", "/private/tmp/astral-benefit-20261001/packets.json"]
default_tools_approval_mode = "approve"

[mcp_servers.evidence.tools.read]
output_token_limit = 200000

[mcp_servers.astral]
command = "/Users/scottmeyer/projects/astral/target/release/astral"
args = ["mcp", "--proxy-url", "http://127.0.0.1:18088"]
default_tools_approval_mode = "approve"
```

The controller runs `codex -C <workspace> exec [resume] --json
--skip-git-repo-check --ignore-user-config -m gpt-6-astra`, passing the settings
with `-c`. Existing ChatGPT login supplies authentication. Saved command argument
arrays contain the exact prompts and settings for every turn.

| New review metric | Passthrough | Tools |
| --- | ---: | --- |
| Task completion | PASS | FAIL: first turn timed out |
| Completed turns / factual checks correct | 4 / 10 of 10 | 0 / unscored: no final answer |
| Upstream model requests / observed terminal events | 24 / 24 | 29 / 28 |
| Request body bytes before Astral | 9,638,228 | 14,859,180 |
| Forwarded request body bytes | 9,638,228 | 6,532,552 |
| Archive replacements / saved bytes | 0 / 0 | 271 / 8,326,628 |
| Unique stored artifacts | 0 | 21 |
| Agent Astral search / successful recall calls | 0 / 0 | 0 / 16 |
| Observed input tokens including cache reads | 2,005,087 | 1,250,182+ |
| Observed cached input tokens | 1,867,008 | 573,952+ |
| Observed input tokens excluding cache reads | 138,079 | 676,230+ |
| Observed output tokens | 3,330 | 2,718+ |
| Sum of four turn wall times | 233.792 s | Unavailable: first turn stopped at 240 s |
| Failed tools / non-200 model responses | 0 / 0 | 1 / 0 |

The `+` marks a lower bound: usage is available for 28 tools-arm responses; the
29th request had received HTTP 200 headers but was interrupted at the deadline
before its terminal usage arrived. There is no final Codex usage counter for
that run. For passthrough, all 24 request IDs have one terminal event and ledger
usage exactly matches the final Codex cumulative counters. Forwarded bytes cover
requests that received upstream headers, as defined by the accounting below.
The proxy health counter includes non-model traffic and is not this denominator.

The task did not finish in tools mode, so its smaller forwarded total cannot be
called a task-level saving. Within that incomplete run, Astral reduced forwarded
bodies by **56.04%**. Yet even its first-turn usage exceeded the baseline's first
turn: **1,250,182+ versus 865,061 input tokens**, and **676,230+ versus 90,021
uncached input tokens** (at least **7.51 times** as many). The baseline finished
that turn in **80.233 seconds**. These observations show why byte reduction alone
is insufficient; they do not establish billing amounts or a general causal
estimate for prompt-cache losses.

### Timeout and retrieval investigation

The tools run completed five evidence reads and 16 `astral_recall` calls before
the deadline. One earlier recall requested `limit=60000` and failed with
`limit must be 1..16384 bytes`; the model corrected it and continued. The MCP
schema already declares that maximum, so this was a caller error, not an absent
schema bound. No shell, file-editing, or web calls occurred. All model requests
received HTTP 200; the failure was not an authentication or routing stall.

The successful recall pages exactly match their stored bytes and hashes. They
cover **seven complete archived outputs**, including full paginated recovery;
all **21** stored-artifact hashes validate. Thus archival and retrieval succeeded
while the review task failed to complete. The trajectory spent additional calls
rereading evidence and paging archived outputs. The incomplete reviewer had no
final answer to grade. The original 240-second cap was retained, and no selective
retry was used to replace this result.

This is a negative result for the tested review configuration. It supports
keeping normal harness settings for everyday work rather than adopting this
configuration for savings. A future candidate would need to preserve outputs
still being actively read and demonstrate that retrieval and cache overhead do
not erase the reduction. This run does not establish which policy change would
achieve that. No new proxy defect was confirmed in this comparison.

### What the transcripts show

A subsequent local audit matched the visible tool calls, returned data, ledger
timestamps, and archive contents. It excluded private reasoning and required no
new model calls. The timeout involved an interaction between the exhaustive test
prompt, harness truncation, and archival during a single long user turn.

1. **The prompt made the review unnecessarily exhaustive.** It said “Read each
   requested packet completely.” The first packet was 48,681 bytes; 32,940 bytes
   were repeated padding text. The second was 207,457 bytes, including at least
   98,820 bytes of repeated padding. Much of that content was irrelevant to the
   factual review. Both runs attempted full reads, encountered truncation, and
   paginated. The per-tool and code-mode output-limit overrides did not establish
   untruncated forwarding through every harness layer.
2. **The tools run made overlapping reading passes.** After a full read and an
   attempted multi-chunk yield, it emitted slices from character 60,000 to the
   end of `normal_harnesses`. It then started again at zero and emitted the whole
   packet in 20,000-character slices. The baseline used one 24,000-character
   pagination pass over that packet and finished the first turn in 80.233 seconds.
3. **Astral aged those pages out while the review was still in progress.** The
   first archival was on model request 7, about 52 seconds after the first
   request received headers. There was still only one user turn. The four-result
   rule permits archival within a long turn even though two recent user turns
   are normally retained. As further chunks arrived, earlier chunks became
   previews. The proxy does not know whether the reviewer still needs them.
4. **The reviewer returned to already-read evidence.** At 19:02:18 UTC, its
   visible update already correctly said both normal harnesses had completed
   tools without demonstrating archive retrieval. It then rechecked setup
   details. Its first Astral recall was at 19:02:22, roughly 170 seconds after the
   first upstream headers. After one invalid page-limit call, it recovered the
   initial evidence response in four pages, then six earlier chunks covering
   characters 0–120,000 of `normal_harnesses` in twelve pages. Those six origins
   match the recorded tool outputs exactly. The initial response is matched by
   a 24,023-character prefix and its truncation marker, rather than full equality.
5. **Most uncached work preceded explicit recall.** The first 22 observed
   terminal events consumed 946,400 input tokens, including 404,992 cached tokens:
   541,408 uncached tokens, or 80.06% of the run's observed uncached total. At the
   first archival, cached input fell from 47,872 to 16,384 tokens and stayed at
   16,384 for the next several requests as further old results were replaced.
   This is consistent with changing earlier request content disrupting cached
   prefixes. It does not isolate all latency or cache effects: the runs had
   different trajectories and cache differences even before archival began.

This was repeated reading, not an observed endless chain of recalling recalled
pages. One recall response was itself subsequently archived, but none of the
16 successful recalls targeted that new artifact. The seven recovered objects
all came from earlier evidence reads or slices. No final review answer was
produced before the controller's deadline.

The earlier emphasis on retrieval overhead alone was incomplete. The test prompt
and truncation induced excessive reading; within-turn archival then removed
older reading chunks and repeatedly changed the request prefix. The transcript
supports that sequence, while the relative contribution of each factor would
require a controlled follow-up.

The controlled follow-up set `--keep-recent-tool-results 0`, retaining the
default two recent user turns and the same task, to isolate within-turn
archival. The existing
`long_turns_archive_consumed_results_but_keep_the_entire_latest_parallel_batch`
regression verifies that zero disables archival within the first long turn.
That experiment and a separate intake-reduction prototype are evaluated below.

### Retaining active turns

The controlled follow-up used the same immutable packets, four exhaustive
prompts, model, settings, release binary, and 240-second per-turn deadline.
Only the proxy's `--keep-recent-tool-results 0` setting changed. The default
two-user-turn retention remained in effect. The checkout was at `373bed0`;
there were no new production source changes.

**Task completion: PASS. Cost benefit: not demonstrated.** All four turns
completed, with all ten factual checks correct and no failed tools. The ledger
confirms zero archival during the first two turns; archival started in the third.
The first turn took 88.449 seconds, compared with 80.233 for passthrough and the
240-second timeout under default tools settings.

| Same exhaustive review workload | Passthrough | Tools, `--keep-recent-tool-results 0` |
| --- | ---: | ---: |
| Completed turns / factual checks correct | 4 / 10 of 10 | 4 / 10 of 10 |
| Model requests / observed terminal events | 24 / 24 | 23 / 23 |
| Request body bytes before Astral | 9,638,228 | 9,418,483 |
| Forwarded request body bytes | 9,638,228 | 7,669,087 |
| Archive replacements / saved bytes | 0 / 0 | 57 / 1,749,396 |
| Unique stored artifacts | 0 | 13 |
| Agent Astral recall calls | 0 | 0 |
| Input tokens including cache reads | 2,005,087 | 1,585,855 |
| Cached input tokens | 1,867,008 | 1,369,472 |
| Input tokens excluding cache reads | 138,079 | 216,383 |
| Output tokens | 3,330 | 3,621 |
| Sum of four turn wall times | 233.792 s | 212.112 s |
| Failed tools / non-200 model responses | 0 / 0 | 0 / 0 |

The run used 20.91% fewer total input tokens but **56.71% more uncached input**
than passthrough. Its elapsed time was 9.27% shorter, with a different agent
trajectory. One run per configuration cannot establish a general latency effect.
Retaining active turns avoided the observed timeout but did not establish lower
billing or elimination of cache disruption when older turns were archived.

All 13 artifact hashes validate. Ten artifacts exactly match local tool-output
records; three differ and contain harness truncation markers. No agent recall
occurred in this run, so this is an archival-and-task-completion result. The
earlier explicit recall qualification remains the retrieval evidence.

### Reducing output before it enters history

This experiment used a private MCP adapter, `intake_mcp.py`, over the same six
immutable evidence packets. It is a prototype of intake reduction, **not a new
Astral production feature**. It removes only runs of the exact known synthetic
padding phrase, replacing each with a stable notice and original character
offsets. All other text is retained. It does not generate summaries or decide
which unfamiliar log lines matter.

Both arms expose identical tool schemas:

- `evidence.read(key)` returns a view, original SHA-256, original byte count,
  and any explicitly omitted character ranges.
- `evidence.search(key, query)` searches the immutable original and returns up
  to six exact excerpts with character offsets.
- `evidence.read_range(key, offset, limit)` returns exact original text, up to
  8,000 Unicode characters, with `next_offset` for paging.

The raw arm returns the original view; the compact arm omits known padding.
Both use Astral **passthrough**, so previous tool outputs are not rewritten as
they age. The objective is a smaller, stable conversation from the first read.
The same full-history HTTP configuration, `gpt-6-astra` with low reasoning,
existing ChatGPT login, and 240-second per-turn deadline apply to both.

Both receive the same focused review task. It replaces the earlier requirement
to read every packet completely with “Answer the review questions using the
evidence you need” and states that routine synthetic padding is unnecessary.
The final ten scored fields are unchanged. Compare these two arms with each
other; the changed prompt prevents attributing differences from the exhaustive
review solely to intake reduction.

The first raw attempt failed tool selection after 25.348 seconds: the model
discovered only Astral tools, then used two packet names as archive handles.
Both calls failed. The transcript contains no attempt to discover evidence
tools, so it does not show that the evidence server was unavailable. The run is
preserved as `review-intake-raw-1`, with no final factual score. The corrected
prompt explicitly distinguishes evidence keys from archive handles and names
the evidence tools. **Both** matched arms use that correction, fresh sessions,
and fresh proxy state; execution order is raw followed by compact.

Local stdio MCP validation passed before inference: identical tool schemas,
stable repeated compact reads, exact search results, and complete paged recovery
of all six originals. Restoring omitted spans reconstructs each original hash
exactly. Source and report packets have no matching padding and remain intact.
Actual model-call audits separately compare returned MCP values with the
controller. These local records do not prove that every large result reached
the provider untruncated; harness truncation remains a limitation.

**Matched task and retrieval checks: PASS.** Both completed four turns and all
ten factual checks, with 14 upstream requests and 14 observed terminal events.
All responses were HTTP 200; no tools failed. Ledger usage matches the final
Codex cumulative usage exactly in each arm. The nine evidence returns per arm
match their controller values exactly: seven reads, one search, and one original
range retrieval. Neither agent called Astral retrieval.

| Focused review, stable history | Raw intake | Compact intake |
| --- | ---: | ---: |
| Completed turns / factual checks correct | 4 / 10 of 10 | 4 / 10 of 10 |
| Model requests / observed terminal events | 14 / 14 | 14 / 14 |
| Forwarded request body bytes | 3,699,979 | 3,043,516 |
| Astral archive replacements / saved bytes | 0 / 0 | 0 / 0 |
| Unique Astral artifacts / agent Astral recall calls | 0 / 0 | 0 / 0 |
| Successful original-range retrievals | 1 | 1 |
| Input tokens including cache reads | 809,432 | 687,405 |
| Cached input tokens | 708,608 | 605,440 |
| Input tokens excluding cache reads | 100,824 | 81,965 |
| Share of input reported cached | 87.54% | 88.08% |
| Output tokens | 2,481 | 2,398 |
| Sum of four turn wall times | 152.027 s | 135.886 s |
| Failed tools / non-200 model responses | 0 / 0 | 0 / 0 |

Compact intake reduced forwarded bytes by **17.74%**, total input tokens by
**15.08%**, and uncached input by **18.70%**. Elapsed time was **10.62%** shorter.
Both cached and uncached token counts fell, while the share cached remained
similar. These are task-level measurements from one fresh session per arm,
executed in a fixed order. They do not isolate model or service variability,
establish a general latency improvement, or measure billing amounts.

The result supports further work on **stable, smaller tool outputs from their
first appearance**, with exact originals available through targeted retrieval.
It does not establish a general-purpose summarizer: the omitted text was known
synthetic padding, and this test did not assess relevance selection on unfamiliar
logs. Production integration and qualification under ordinary Codex and Claude
history management remain **NOT TESTED**. Current ordinary harnesses still only
demonstrate routing through Astral; changing their output path would need its
own implementation and validation.

### HTTP accounting correction

The earlier missing rows were reproduced locally: an upstream SSE server emits
one terminal event but holds the stream open; the client consumes it and drops
the connection. Before the fix, the regression observed zero request rows where
one was expected. The correction records request sizes after upstream headers,
and terminal usage before yielding the terminal chunk. Both carry a request ID.
Clean EOF is still required for completion and projection commit.

The regression covers tools and passthrough modes, with either a terminal event
or only a partial text delta. It checks exact forwarded sizes, linked terminal
usage when present, absent completion rows, and absence of credentials or tool
text in the ledger. Scope is Responses HTTP; it does not add WebSocket accounting.
Do not sum these usage rows together with the duplicated final response rows.

Validation after this correction passed: the complete locked Rust suite,
**520 tests**; **18 Python tests**; `cargo fmt --all --check`; strict all-target
Clippy; Rust 1.85 all-target checking; and `cargo build --release --locked --bins`.
The failing-before and passing-after regression logs are retained locally.

### Follow-up evidence and reproduction

Private evidence and the exact controllers are retained outside the repository:

```text
/private/tmp/astral-benefit-20261001/
  prepare.py, evidence_mcp.py, trial.py, evaluate.py, audit_review.py
  evaluate-complete-only.py     # original scorer, retained for provenance
  packets.json, packet-manifest.json, packet-delivery.json
  evaluation.json, followup-validation.json, blocked-comparison.json
  controller-manifest.json, completed-controller-manifest.json
  comparison-derived.json
  trace_review.py, transcript-trace.json
  trial_turns.py, turns-variant.json
  trial_intake.py, trial_intake_v1.py, intake-variant.json
  intake_mcp.py, verify_intake.py, intake-verification.json, audit_intake.py
  stable-context-results.json, stable-context-manifest.json
  review-pass-1/
    command-*.json, receipt-*.json, answer-*.txt, turn-*.jsonl
    proxy.stderr, health.json, state/ledger.jsonl, audit.json
  review-tools-1/
    command-0.json, turn-0.jsonl, turn-0.stderr, proxy.stderr
    run-outcome.json, timeout-summary.json, audit.json
    state/ledger.jsonl, state/artifacts/
  review-tools-turns-1/
    command-*.json, receipt-*.json, answer-*.txt, turn-*.jsonl
    audit.json, turn-accounting.json, state/ledger.jsonl, state/artifacts/
  review-intake-raw-1/
    command-0.json, receipt-0.json, turn-0.jsonl, run-outcome.json
    state/ledger.jsonl
  review-intake-raw-2/, review-intake-compact-2/
    command-*.json, receipt-*.json, answer-*.txt, turn-*.jsonl
    audit.json, health.json, proxy.stderr, state/ledger.jsonl
  logs/
    accounting-before.log, affected.log, clippy.log, build.log
    full-test.log, python-test.log, msrv.log
```

`packet-manifest.json` records each immutable packet's SHA-256. Preserve those
packets for comparison: rerunning `prepare.py` after editing this report changes
the evidence. The saved per-arm `command-*.json` arrays are authoritative for
the runs. Raw transcripts and packet contents are not committed. The scorer now
includes timed-out arms instead of silently omitting runs without a final answer.

```sh
# Local scoring only; no inference or credential access:
python3 /private/tmp/astral-benefit-20261001/evaluate.py
python3 /private/tmp/astral-benefit-20261001/audit_review.py review-pass-1
python3 /private/tmp/astral-benefit-20261001/audit_review.py review-tools-1
python3 /private/tmp/astral-benefit-20261001/audit_review.py review-tools-turns-1
python3 /private/tmp/astral-benefit-20261001/trace_review.py
python3 /private/tmp/astral-benefit-20261001/verify_intake.py
python3 /private/tmp/astral-benefit-20261001/audit_intake.py review-intake-raw-2 raw
python3 /private/tmp/astral-benefit-20261001/audit_intake.py review-intake-compact-2 compact

# Fresh review sessions; these send packet contents to the Codex service.
python3 /private/tmp/astral-benefit-20261001/trial.py passthrough review-pass-2
python3 /private/tmp/astral-benefit-20261001/trial.py tools review-tools-2

# Retain active turns; only this retention setting differs from trial.py.
python3 /private/tmp/astral-benefit-20261001/trial_turns.py tools review-tools-turns-2

# Matched intake comparison: use the same revised prompt in both fresh arms.
python3 /private/tmp/astral-benefit-20261001/trial_intake.py passthrough review-intake-raw-3 raw
python3 /private/tmp/astral-benefit-20261001/trial_intake.py passthrough review-intake-compact-3 compact
```

The controller starts/stops only its own standalone proxy and saves each exact
CLI invocation. Each label must be new. No production source changes were needed
after the 520 Rust / 18 Python validation run. The intake experiment adds private
controller and MCP scripts; the repository change records their results and
reproduction commands. The unchanged release binary's SHA-256 is
`ec6519cd5645425b9940d2a62a438793c30203d2ae1f775451186fb9d0fe2094`.
All test-owned proxies and harnesses have stopped; port 18088 has no listener.

## Remaining limits

- Ordinary Codex and Claude Code did not demonstrate context reduction; their
  provider-managed histories intentionally pass through.
- Complete Codex forwarded-byte accounting is unavailable for the original runs
  and WebSocket traffic. The follow-up HTTP correction gives complete accounting
  for the new review requests. Tools-arm terminal usage is unavailable for the
  one response interrupted at its timeout; reported usage is a lower bound.
- The successful full-history configurations are explicit diagnostics. The
  initial recall qualification and subsequent synthetic review experiments do
  not establish general quality, latency, or billing benefits.
- The intake prototype preserved cached usage while reducing tokens in one
  matched pair. Its fixture-specific filter is not implemented in Astral's
  production proxy, and exact MCP returns do not prove untruncated provider input.
