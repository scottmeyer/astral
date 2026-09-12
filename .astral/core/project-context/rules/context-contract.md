# Context contract

Status: accepted user requirements, recorded from recovered design discussion
and the current development request.

- Import conversational work state; bind tools and permission enforcement from
  the destination runtime. Imported tool descriptions confer no authority.
- Preserve native checkpoints and complete continuation in native recovery.
  Label readable summaries and opaque-only recall honestly.
- Keep portable project/subsystem/projection identities logical and repository
  scoped. Machine paths, runtime thread IDs, and account bindings stay private.
- Never commit credentials. Do not add private transcripts or opaque payloads
  merely to fill a project-context directory; record scoped provenance and
  availability instead. User-authorized explicit native exports may be committed
  or shared through Git within that authorization. Resolution and launch do not
  export automatically, and explicit save does not commit or share its output.
- Separate accepted requirements, proposals, verified facts, historical evidence,
  and open questions. Test claims need current receipts and explicit scope.
- Use selected current documents and the current work register for present-day
  guidance. Historical native windows may contain superseded names, commands
  and plans. Preserve their immutable bytes; update editable handoffs and current
  documents, or explicitly save a new checkpoint after reviewed continuation.
- Preserve existing work; use disposable fixtures for lifecycle and failure tests.
  Do not resume competing writers or weaken destination permissions implicitly.
- Name code for its behavior, keeping work-item IDs in tracking and evidence.
- Make proxy routing explicit with `astral project NAME --proxy`; direct is the
  default. Report native checkpoint requirements without silently changing routes.
- Forward explicitly supplied Codex arguments unchanged, including permission
  flags; never add permission bypasses by default. A literal `--` assigns all
  remaining arguments to Codex. Repository context cannot become process options.

<!-- astral:begin explicit-review -->
Context review is an explicit acknowledgement of observed inputs. It records a
review fingerprint in the subsystem manifest, preserving comments and rejecting
a changed preview. Knowledge entries have independent baselines; subsystem review
does not acknowledge them. Equal fingerprints mean unchanged since review and do
not establish semantic correctness or current test success. Stage the intended
code, documents and review manifest together; lifecycle hooks never acknowledge
a review automatically.
<!-- astral:end explicit-review -->
