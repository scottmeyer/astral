# Fresh launch verification

Recorded 2026-09-11 on `feat/fresh-project-launch`, based on
`c41c36405288449be4811cf641a010c975ca1cd2`. These checks were run on the
implementation in this change; imported native-recovery results were not reused
as current verification. Scope and supported arguments are in
[fresh launch](fresh-launch.md).

## Local checks

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --all-targets --locked -- -D warnings` | PASS |
| `cargo test --locked` | PASS, 153 Rust tests |
| `cargo build --release --locked --bins` | PASS |
| `python3 -m unittest discover -s tests -p 'test_*.py'` | PASS, 18 tests |
| `git diff --check` | PASS |
| `astral context validate` after documentation/work-record updates | PASS, 1 subsystem, 1 projection, 21 work records |

The release `astral` binary used for the final live launch had SHA-256
`d43382317c96fc1120288b4c949acd201d4fe13dc9fdfe6069f47de9c5f30cac`.

Resolver tests cover selected source bytes and hashes, dependencies, work records,
UTF-8 and output bounds, and rejection of native or unknown projection kinds
through either direct or subsystem selection. App-server fixture tests cover
bounded messages, invalid/error replies, persistence and clean shutdown,
thread/workspace validation, option translation and unsupported settings.
CLI subprocess tests exercise initialization, generated-index validation,
literal resume arguments, refusal paths and child exit status. The embedded
prompt's templates are validated through the actual resolver.

## Live Codex controls

Installed Codex 0.154.0 on macOS, model `gpt-6-astra`, was exercised directly with
no Astral proxy. All sessions and repositories were disposable. Original recovery
sessions, portable native artifacts, authentication files and the running recovery
proxy were unchanged.

Two repositories containing a README and a small Python source file each ran
`astral init --non-interactive` with explicit workspace-write policy. Both Codex
tasks completed with exit 0, and Astral accepted a nine-file schema-v1 index with
zero work records. The original source-file hashes remained unchanged. These
checks establish successful best-effort indexing of small fixtures, not complete
or accurate architecture discovery in arbitrary repositories.

Each resulting context was then launched through `astral project` into the real
interactive Codex CLI with explicit read-only sandbox and approval policy
`never`. The final launch used the release binary identified above.

| Control | Result |
| --- | --- |
| Staging and interactive resume | PASS, new durable thread for each launch |
| Recall the supplied documented run command without reading files | PASS, `python3 hello.py` |
| Actually invoke terminal `pwd` | PASS, exit 0 and exact fixture working directory |
| Attempt a fixture write under read-only policy | PASS, actual exit 1 and operation-not-permitted result |
| Independently check denied target | PASS, file absent |
| Close interactive Codex with Ctrl-D | PASS, launcher exit 0 |

Command execution was checked from linked tool calls and results, independently
of the assistant's answer. The denied write appeared in a native custom-tool
result, so counting only `commandExecution` records would miss it. The production
adapter does not read or patch session files; the diagnostic audit read only its
own disposable session. A supplementary app-server `thread/read` audit timed out;
the final receipt instead uses linked execution records from that fixture's
rollout. An initial recall-audit assertion selected the wrong message
representation; inspecting assistant response items corrected the harness and
confirmed the recalled command. Neither diagnostic issue was a failed launch.

Private receipts, execution evidence, initialization events and scanner output
are retained outside the repository under
`/private/tmp/astral-fresh-launch-trial-20260911`. No private transcript, account
data, session identifier or native payload is included in this document.

## Supplemental scanner

UBS 5.4.2 completed `ubs --staged --ci --no-cargo --format=json` with no failed
modules. It scanned 12 Rust files and reported 12 critical, 773 warning and 270
informational heuristic matches; the findings produced exit 1. This is a completed,
reviewed scan, not a clean scanner pass. Cargo checks ran separately in the full
checkout.

The critical matches were five intentional test-only panics, an existing
`MaybeUninit::assume_init` guarded by successful `fstatat`, a literal configuration
argument comparison incorrectly classified as a secret comparison, and five
variable-executable matches. Production executable selection comes from the
current trusted process environment/default Codex command; test executables come
from Cargo's binary paths. Repository context cannot select the executable.
Warnings were predominantly test assertions/unwraps, indexing, and bounded
allocation/path patterns. Review found no introduced production defect in these
matches; findings were not suppressed.

## Limits

This milestone establishes fresh document launch and initialization on the tested
runtime. It does not verify native restoration, managed proxy launch, automatic
worktree/session reuse, Git reconciliation, cross-account portability, other
platforms, abnormal host termination, or resource savings. Regular app-server
shutdown and interactive exit were exercised; full cancellation/signal lifecycle
qualification remains separate. Supported staging settings are explicitly bounded
as described in the launch guide. Historical native evidence remains historical.
