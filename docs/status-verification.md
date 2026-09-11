# Status and diagnosis verification

Date: 2026-09-11. Scope: local macOS implementation based on `517a66a`; initial
status/guide commit `dd73933`. Earlier lifecycle receipts are historical and were
not counted as new native recovery checks in this pass.

## Local checks and review

The full **324-test Rust suite**, **18 Python tests**, strict all-target Clippy,
formatting, locked release builds of all binaries, and the Rust **1.85.0**
all-target check passed. Coverage includes 9 new status tests, 10 worker
observation integration tests and one atomic receipt-replacement unit test.
After the full pass, the human display was refined to show work progress
separately from binding state; all 9 status tests and strict Clippy passed again.
No observation or launcher behavior changed in that display refinement.

The tests cover absent and valid bindings, nondefault selectors and managed
roots, active ownership, abandoned preparation, missing/corrupt/private state,
symlinks and hardlinks, changed Git associations, orphan planned artifacts,
selected context changes, native selection mismatch, missing runtime metadata,
unknown comparison baselines, pagination, formatted JSON, human control-character
escaping, and unchanged files/index/receipts under observation. The existing
worktree and launch regression suites also passed.

Independent review identified three issues before the full pass: no baseline
was being treated as changed context; native-required metadata missing its model
or provider still received a continuation suggestion; and Unicode controls could
reach the human argv display. They were fixed with regression coverage. An early
work-record creation helper misread the CLI response envelope after successfully
creating its first record; inspection recovered that record and no duplicate was
created. Those intermediate outcomes are retained in private evidence.

Current usage guides were reconciled with implemented behavior; dated verification
receipts remain unchanged. Validation found 36 work records, one native artifact
and a valid project graph. All 80 local links in the changed Markdown resolved.
`AST-009`, `AST-003` and `AST-004` now reflect their completed, explicitly scoped
milestones. Status/diagnosis is complete; the larger finish/resume milestone
remains in progress with recovery, completion and hook work still open.

## Live Astral worker observation

A real review worker was launched through `astral project --work` from the
committed implementation, using direct Codex 0.154.0, read-only sandbox policy,
never approval, and a bounded source-review task. No proxy or native import was
needed for this fresh document context. Its work ID is `AST-63b4m1bn3vpk`.

| Observation | Result |
| --- | --- |
| Before launch | Work record exists; no binding; `unbound` |
| During initial staging | Existing ownership lock is busy; `owned`, with no continuation suggestion |
| During the running review | `owned`, recorded thread ID present, no diagnostics, no continuation suggestion |
| After worker exit | Launch exit 0; `recorded`, ownership `available`, selected context matches the staged digest, continuation argv present |
| Doctor after exit | Exit 0; exact private receipt bytes unchanged |

The worker independently reviewed `dd73933` and reported no concrete defects
across status, observation, CLI/tests, binding/context logic and the workflow plan.
It did not edit files, run tests/builds, inspect private sessions or authentication,
use network tools, or delegate. Parent tests and observation receipts supply the
verification above; the reviewer's prose is not execution evidence.

These controls establish local observation against an actual owned worker. They
do not establish remote thread existence from status alone, native recall,
cross-account portability, repair, hook behavior or branch completion. Status
does not consult the runtime, infer code freshness, or certify historical tests.
The inventoried IDs come from the current checkout's work register; orphan
private bindings and receipt-only launches remain outside this inventory.

## Scanner and evidence

The initial required staged static UBS scan completed its Rust module across
8 files with **0 critical, 293 warning and 119 informational** matches, exit 0.
This is not a warning-free scan. Reviewed matches concern test assertions and
fixture parsing, existing guarded Git-output indices, a test-only path join,
bounded report allocations and already-reviewed workspace operations. No new
production defect was confirmed; no detector suppression was added.

The final display/receipt follow-up scan covered 2 Rust files: **0 critical,
135 warning and 32 informational** matches, exit 0. The same test assertions and
bounded report-rendering patterns were reviewed; this was also not warning-free.

The scan used `--no-cargo` because staged snapshots omit unchanged crate modules.
Independent Cargo checks ran in the complete checkout. UBS dependency-audit
phases, remote CI jobs, other operating systems and live native lifecycle tests
were not run in this pass. Logs, event streams and local binding identifiers remain
in a private evidence directory outside Git; no transcript or credential is
included in this receipt.
