# Interactive context picker verification

Date: 2026-09-11. Work: AST-sr3a6keeegxj. Base: `7818c90`.
Implemented and checked in an isolated `feat/context-picker` worktree.
Earlier receipts remain historical; the checks below were rerun for this change.

## Behavior and boundaries

Terminal `astral context list` opens searchable context and local work menus.
`astral project --pick` offers the same workflow with optional initial context,
work selection and literal Codex arguments. Plain, JSON and redirected list
output remain reports. The final review exposes the effective context, work,
branch, destination and route, with wrapped, paged details on smaller terminals.

Existing workers retain their recorded context and checkout. Browsing and
cancellation are read-only. The review uses existing project, binding, status and
launch-option validation. Confirmation restores the terminal, recomputes observed
context/Git/binding evidence, and refuses a changed review before invoking the
normal launcher. This comparison is not an atomic filesystem snapshot or ownership
reservation; the launcher's acquisition and validation remain authoritative.

Native history offers an explicitly named launch/resume action with `--proxy`.
No work-item creation or external issue adapter is included. Codex `--cd`/`-C`
may not resolve before a new managed worktree exists; omit it and let Astral use
the reviewed destination. Platform terminal behavior was exercised on macOS;
live provider inference and Windows terminal behavior were not tested here.

## Current checks

- All **497 Rust tests** passed, with zero failures or ignored tests, using the
  locked dependency graph and isolated Git signing settings.
- **14 real PTY integration tests** cover keyboard navigation, search, back and
  cancellation, plain/JSON/nonterminal behavior, incompatible options, changed
  review evidence, existing/busy bindings, native-route consent and an actual
  offline Codex stub resume. The stub checks restored canonical/echo settings
  and exact forwarded arguments, including shell metacharacters as literal data.
- Eight terminal unit tests cover bounds, Unicode/control escaping, search,
  clipping, navigation, small-screen confirmation and full-path review paging.
- All **18 Python tests** passed with the newly built release helper.
- Formatting, whitespace checks, strict all-target Clippy, Rust **1.85.0**
  all-target checking and all release binary builds passed.
- Seven direct release CLI controls passed. A release terminal walkthrough in
  the real repository reached context, work and launch review, then cancelled.
- The existing native bundle manifest and window still match their recorded
  SHA-256 hashes; neither was edited.

The terminal implementation uses Crossterm 0.29.0. Cargo added six packages and
enabled Mio's log dependency without changing existing dependency versions.

## Scanner review

The staged UBS JSON and verbose scans completed with no failed modules, exiting
1 for heuristic matches: **4 critical, 349 warning and 94 informational** findings
across six Rust files. This is not a clean scanner result.

All four critical locations were reviewed: two explicit PTY test failure panics,
`assume_init` immediately after successful `tcgetattr`, and a test child's
`finish()` method mistaken for non-cryptographic security-token generation.
Warnings include bounded menu allocations, serialization of known values,
deliberate terminal output and fixed fixture paths. No introduced defect was
confirmed; no suppression was added. UBS Cargo/audit phases were skipped;
the independent Cargo checks above ran against the complete checkout.

Private logs and release metadata are retained outside tracked source.
