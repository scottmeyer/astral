# Named knowledge verification — 2026-09-11

`AST-qfsf83fb1kf0` extends the subsystem freshness slice with named knowledge
entries. This pass was developed on `feat/knowledge-entries` from
`e6708c6af95616857d4f0a5f42315d882150976a` (`feat/context-freshness`). These are
recorded results; later checkouts require their own relevant checks.

Local macOS verification passed: 523 locked Rust tests, formatting, strict
all-target Clippy, the locked release build of all binaries, the Rust 1.85
all-target check and 18 Python tests. After final scope-text and output-truncation
changes, Clippy, release build and MSRV checks passed again, along with 53 focused
tests (33 CLI unit tests, 11 subsystem freshness tests and 9 knowledge tests).

The new fixtures cover stable references, region-only recall, code and region
edits affecting only their declared entries, document moves, CRLF, whole-document
entries, missing code, invalid UTF-8 and missing/duplicate/reversed/empty markers.
They check bounded declarations and output, invalid paths, exact Git index/HEAD
and working evidence, comment-preserving review writes in both TOML array forms,
stale previews and independent entry/subsystem acknowledgements. Existing
confinement and publication tests remain in the full suite. Tests use disposable
local files, Git repositories and processes; no live provider is involved.

The repository index validates and all four declared selections inspect with the
new release build. Both authored entries recall their exact marked text; a manual
terminal acknowledgement exercised the named review scope. Their fingerprints and
the broader subsystem baseline were explicitly reviewed and recorded. Named-entry
inputs expand that broader scope from nine to twelve distinct code files; this
does not claim complete Astral source coverage. Changed documentation's local link
targets exist. Native bundles were not edited.

Staged UBS completed with Cargo execution disabled: 12 Rust files, zero critical
findings, 264 warnings and 168 informational findings. This is not a warning-free
scan. Reviewed categories include fixture assertions/unwraps, validated map/range
indexing, and allocations/clones under existing source, entry and output bounds.
Cargo checks were run separately; the scanner's skipped dependency-audit phases
are not claimed as passing.

The supported contract is [explicit references and byte-change evidence](knowledge-entries.md).
No automatic claim extraction, function-aware tracking, supersession, semantic
staleness, ranked retrieval, prompt-size savings or long-horizon recall benefit
was established. Shared core remains outside an individual entry's review
baseline. Normal launch still includes full selected documents and native
history remains historical. The installed CLI and prior release were not replaced
as part of this slice; manifests with these fields need the compatible source build.
