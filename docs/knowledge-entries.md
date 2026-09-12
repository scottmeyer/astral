# Named knowledge entries

Source builds after v0.1.0 can give individual rules, decisions and observations
stable references. An entry identifies selected project prose and the exact code
files against which it was reviewed. For example:

```sh
astral context knowledge web
astral context show knowledge:web/session-revocation
astral context freshness knowledge:web/session-revocation
astral context review knowledge:web/session-revocation
```

`context knowledge` lists entries from a subsystem or projection and its selected
dependencies. Omit the selection for `project-context`. `context show` recalls
one explicit reference; it does not run inference. Human output is bounded and
sanitized for the terminal. Use `--json` for complete bounded text, kind, title,
source metadata and freshness. Entry references are not `project` launch selectors:
launch still selects a subsystem or projection with its normal full documents.

## Author an entry

Add an array-of-tables entry to a registered subsystem's `subsystem.toml`:

```toml
[[knowledge]]
id = "session-revocation"
kind = "decision"
title = "Session revocation"
document = "decisions/session-handling.md"
region = "session-revocation"
inputs = ["src/session.rs", "tests/session.rs"]
```

The document path is relative to the subsystem directory and must already appear
in that subsystem's `readme`, `rules` or `decisions`. Code inputs are exact paths
relative to the repository root. There is no implicit file discovery.

With `region`, the document must contain one matching pair of standalone lines:

```md
<!-- astral:begin session-revocation -->
Revoking a session invalidates its stored token before the next request.
<!-- astral:end session-revocation -->
```

Omit `region` to reference the entire document. Markers are literal lines: they
must have no surrounding whitespace. They count even inside a Markdown code
fence, so do not duplicate the same marker as a quoted example in its target
document. CRLF is accepted; returned text preserves its original line endings.
Marker lines themselves are excluded. This is byte-region selection, with no
Markdown heading, nesting or code-symbol interpretation.

Kinds are `rule`, `decision` and `observation`; they describe the entry and do not
change its authority. IDs are unique within each subsystem. The reference
`knowledge:web/session-revocation` stays stable when its document moves, provided
the subsystem and entry IDs stay the same and its declared document is updated.
Moving a document or changing the entry's declaration requires review. There are
no automatic rename aliases or references across repositories.

## Review scope and independence

Each entry starts `unreviewed`. After reviewing its prose against its declared code,
run `context review knowledge:SUBSYSTEM/ENTRY`. A terminal offers confirmation;
`--dry-run`, `--yes` and `--apply PLAN_SHA256` use the existing
[explicit review contract](context-freshness.md#review-and-acknowledge).
Acknowledgement writes that entry's `reviewed_fingerprint` in `subsystem.toml`.
Keep the code, document and manifest together when staging and committing.

The fingerprint covers the project ID, subsystem ID, normalized entry declaration,
referenced path and region, exact selected text, and declared code observations.
It excludes review digests, shared core and all other entries. Editing a different
region in the same document does not invalidate this entry. Editing a shared code
input does invalidate every entry that names it. Inputs remain whole files:
changes elsewhere in the same code file still require review.

`unchanged` means the observed fingerprint matches its recorded review; it does
not prove the prose true or tests passing. Changed bytes produce `needs_review`.
Missing, duplicate, reversed or empty regions produce `unavailable`, and recall
fails without substituting the full document. A missing code input also makes
freshness unavailable, but valid document text can still be recalled with that
status. An unreadable selected document invalidates normal project loading.

Whole-subsystem `[freshness]` can coexist with entries. Its broader baseline still
covers shared core, selected subsystem/dependency documents and all their declared
code inputs, including inputs declared only by knowledge entries. Acknowledging
the subsystem does not acknowledge its entries, and acknowledging an entry does
not change another entry's or subsystem's baseline. A pending review preview does
bind the entire target manifest: any intervening manifest edit, including a
different entry's acknowledgement, requires a new preview before applying.

Freshness reports, project inspection and launch metadata include selected entry
observations. Git lifecycle checks observe each entry separately in HEAD, index
and working files. Hooks remain advisory and never quote entry prose or record
reviews. Full selected documents still accompany normal launch/resume; narrow
recall does not silently trim the launcher prompt or erase old native history.

## Limits

There are at most 256 entries per project. An entry requires 1–64 code inputs;
all entries and subsystem declarations share the project-wide cap of 64 distinct
code files and existing byte/file/entry reader budgets. Reference components and
region IDs allow ASCII letters, digits, `_`, `-` and `.`, up to 128 bytes;
`.` and `..` are not valid IDs. Titles must be nonempty and at most 1,024 bytes.
Source documents must be UTF-8 and the
selected text must be nonempty. JSON output remains subject to the resolver's
output byte budget.

The same Unix confinement, regular-file policy, exact Git-object reads, stale
preview checks and publication limits as subsystem freshness apply. Native
payloads are untouched. Older binaries reject manifests that opt into these
fields; CLI operations and installed hooks need a compatible source build.

This slice provides explicit addressability and byte-change evidence. Automatic
claim extraction, function-aware dependencies, supersession, ranked retrieval,
semantic freshness and sustained recall-quality evaluations remain future work.
See the [verification record](knowledge-entries-verification.md) for this slice's
fixture and repository checks.
