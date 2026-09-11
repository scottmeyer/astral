# Working state and compact tool observations

Astral now has an explicit host layer as well as the Responses proxy. The host
keeps exact artifacts and versioned workspace state. The proxy keeps the native
continuation state opaque. Neither needs to decode encrypted reasoning.

This is a runnable reference integration, not a transparent interceptor for
arbitrary Codex tools. `astral-state` is a local JSON-lines process. The Python
`Host` and `Session` classes connect it to Responses function tools and the local
proxy. Only the configured workspace files and check commands are exposed.

## Run a session

```sh
cargo build --release --locked --bins
cp -R examples/working-task /tmp/astral-example
export OPENAI_API_KEY='your-api-key'
python3 examples/agent.py \
  --workspace /tmp/astral-example \
  --profile /tmp/astral-example/profile.json \
  --state-dir /tmp/astral-example-state \
  --upstream https://api.openai.com/v1 --auth api-key \
  --model gpt-6-astra --cache-mode implicit \
  --prompt 'Fix total.py to use exact basis-point arithmetic and run tests.'
```

Run the same command with a new `--prompt` for another turn. Omit `--prompt`
to resume an interrupted turn. Gateway mode uses existing authorized routing;
it does not read CLI credentials. Choose a model and cache mode your endpoint
supports. The observed gateway rejects Platform's `prompt_cache_options` and
requires `--cache-mode provider-default`.

The profile is operator-owned JSON:

```json
{
  "files": [
    {"path": "total.py", "writable": true},
    {"path": "check_total.py", "writable": false},
    {"path": "requirements.json", "writable": false}
  ],
  "checks": {
    "tests": {"argv": ["python3", "-B", "check_total.py"], "timeout_seconds": 20}
  }
}
```

Changing the profile, workspace, model, provider scope, authentication mode or
API key requires a new session. The proxy separately isolates account, project
and credential lanes. When gateway identity changes invisibly to this host,
start a new session explicitly. Do not point an existing session at a different
account. The reference host is single-process and sequential; its state lock
rejects concurrent ownership.

## Read less without discarding the source

`read_file` returns a line window, a file generation and SHA-256, and the handle
of the complete original bytes. `write_file` requires that exact current hash
before replacing an explicitly writable file. It archives both versions. An
external edit causes a hash conflict rather than an overwrite based on stale
model context. Missing configured files use `expected_sha256: null`.

`check` runs a configured argv directly, without a shell. Its narrow adapter
recognizes this report contract:

```json
{
  "schema": "astral.check.v1",
  "passed": 12,
  "failed": 1,
  "failures": [{"test": "round_half_up", "actual": 0, "expected": 1}],
  "warnings": [],
  "cases": ["optional detailed observations"],
  "audit_token": "optional original-run identifier"
}
```

`passed` and `failed` count reported test groups; `failures` must have exactly
`failed` entries. A recognized compact observation preserves all failure and
warning entries and lists the original field names. Extra fields remain in
the raw artifact. The adapter never decides that an arbitrary line of terminal
text is irrelevant. Unrecognized, oversized or inconsistent summaries remain
unverified and include a bounded preview with artifact handles.

A passing check requires exit code zero, complete capture, zero failed groups,
and at least one passed group. Failed checks also include a preview. Stdout and
stderr are stored separately. All arms of the supplied experiment use the same
commands and schema; `--raw-observations` additionally exposes full captured
output to provide a control.

`check_history` retrieves prior check receipts and handles, including superseded
ones, in pages of 20. `search` returns literal byte offsets in a saved UTF-8
artifact. `recall` returns exact byte pages, with hexadecimal encoding when a
page splits UTF-8 or contains binary data. Every load verifies the SHA-256.
These tools do not rerun commands. The late-retrieval trial depends on this:
each actual test run creates a new audit token, and only the original artifact
can establish the old token.

## What is verified

The host rehashes every declared file before operations and after each check.
File changes increment generations. A check receipt records the tracked-input
digest, command fingerprint, exit status, capture status and artifact handles.
`working_state` computes `current` by comparing that receipt with the current
tracked revision and confirming no change was observed during execution.

This validity scope is deliberately explicit: **declared file contents,
generations and check configuration**. It does not fingerprint the interpreter,
environment, untracked dependencies, network or other external services. Track
all relevant source, test and requirement files; use a pinned environment for
reproducible verification. Changes that occur and revert entirely between
observations are not detected. A passing report is evidence from the configured
check, not proof that arbitrary program behavior is correct.

User turns are retained verbatim with their source and sequence. Later
corrections remain separate records; the model interprets which requirements
they supersede. The host does not claim to solve natural-language contradictions.
Agent `note` entries record declared obligations or conclusions, not verified
facts, and cannot mutate user constraint records through the provided tools.

## Immediate state updates, frozen prompt epochs

The append-only journal is authoritative. Tool results carry the state events
they produced, so model-visible updates append to the conversation. Astral mode
adds a canonical `HOST_WORKING_STATE` snapshot at the beginning and after each
native inline checkpoint. That snapshot's serialized bytes stay frozen within
the epoch, even while the host's internal state changes. It is an observation
in a user-role data message, not a new developer instruction. Tools and the
orientation text also stay stable.

The session persists a journal cursor, collecting unseen events through the
host's paged `changes` operation. Tool results carry these events even when an
independent host observer discovered the change earlier. New user turns append
a `HOST_WORKING_UPDATES` data message; file changes invalidate prior verification.
These updates do not rewrite the frozen snapshot or rely on rediscovery.

The native encrypted checkpoint remains the provider's continuation state.
All output items, order, assistant phases and encrypted fields are persisted
as returned. The Astral client keeps the complete original history; the proxy
maps it to its committed native projection plus new history. Native control
clients instead use the provider-documented inline pruning rule. Do not combine
these two ownership models in one session.

The host saves each completed response before executing its tools and saves
after each tool result. Provider call IDs become durable action IDs. An action
is journaled as started before execution; completion stores the exact response
as an immutable artifact and journals its handle. A retry with identical ID
and arguments returns the saved result, including after restart. Reusing that
ID with different arguments fails. If execution was interrupted without a
durable outcome, it fails with an uncertain-outcome error; inspect workspace
and state before issuing a new ID. It does not silently repeat a potentially
completed edit or check. This is not an exactly-once guarantee for arbitrary
external side effects.

Journal records are synced before becoming in-memory state. A torn final line
is treated as uncommitted on restart; complete malformed records fail closed.
Artifacts and session snapshots use file sync and atomic rename, plus directory
sync on Unix. An edit and its subsequent journal entry are not one filesystem
transaction: the started-action record and file refresh expose an uncertain
interruption rather than inventing a successful receipt.

## Roll only when the boundary and estimate allow it

`--inline-tool-boundaries` lets the inline proxy schedule provider compaction
after the last outstanding tool result, before the model's next read. It never
deletes or summarizes a partial call batch. The entire effective input is sent
to native inline compaction. This option does not change standalone compaction's
user-turn boundaries. It addresses the extra generation incurred by the older
user-boundary-only inline schedule.

The optional `--economic-roll-policy` accepts `x-ostk-roll-estimate`:

```json
{"remaining_calls":5,"saved_input_per_call":10,"checkpoint_cost":20,"lost_cache_cost":10,"recovery_cost":5}
```

All values must use one caller-chosen cost unit. The estimated net benefit is
`remaining_calls * saved_input_per_call - checkpoint_cost - lost_cache_cost - recovery_cost`.
An otherwise eligible roll is deferred when this is nonpositive, including a
forced attempt. Invalid estimates return 400; absent estimates retain the byte
policy. The header is stripped upstream. The same calculation is available as
the host's `roll_estimate` operation, but is not a model tool. The reference
session does not manufacture estimates or enable the economic gate by default.

This is a decision hook, not an autonomous price predictor. Measured cache reads,
expected remaining work and the cost of rebuilding and retrieving context need
to inform the caller's estimates. Elapsed time never proves cache expiry.

## Operational bounds

This host is intended for a cooperative local workspace. Configured checks run
with the host's OS permissions and may execute model-edited code. The profile
is an application-level scope, not an OS sandbox. Use your normal container or
execution sandbox for untrusted code. Symlink paths are rejected; this is not
protection against a hostile process racing filesystem operations.

Limits are 512 tracked files, 8 MiB per artifact/file/captured stream, 16 KiB per
retrieval page or accepted summary, a 32 MiB journal and check timeouts up to
600 seconds. Capture overflow is marked incomplete and cannot pass. Timeout
currently discards partial capture and records it as incomplete; it kills the
direct child, not an arbitrary tree of background processes. Use foreground
check commands. Non-UTF8 file reads archive bytes before returning an error;
the handle can be derived from the tracked SHA-256 for byte retrieval.

State and session files are private local data, excluded from git in the trial
directory. There is no encrypted-at-rest store, automatic artifact garbage
collection or journal rotation. Retain the state required for old handles;
delete the whole session when it is no longer needed. Model requests and tool
execution are sequential in this reference runner.

## Matched evaluation

```sh
python3 scripts/working_trial.py \
  --model gpt-6-astra --upstream https://api.openai.com/v1 --auth api-key \
  --cache-mode implicit --output .astral-trials/working
```

The three arms are native compaction with raw observations, native compaction
with compact observations, and Astral with those same compact observations
plus state updates and frozen epochs. They use identical tools, prompts and
tests. The task includes an actual code repair, a requirements change, current
verification, 400 independent unseen acceptance cases, a restart and retrieval
of a prior run's exact token. No minimum checkpoint count is required for a
benefit result: avoiding unnecessary compaction is allowed. Local protocol tests
and the separately recorded rollover probes cover checkpoint transitions.

Report adapter gains against the raw native arm; report the incremental state
and projection effect against the adapted native arm. Provider token counts
and elapsed time are observations, not verified invoice savings.
