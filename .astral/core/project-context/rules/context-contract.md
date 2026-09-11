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
- Preserve existing work; use disposable fixtures for lifecycle and failure tests.
  Do not resume competing writers or weaken destination permissions implicitly.
- Name code for its behavior, keeping work-item IDs in tracking and evidence.
- Make proxy routing explicit with `astral project NAME --proxy`; direct is the
  default. Report native checkpoint requirements without silently changing routes.
- Forward explicitly supplied Codex arguments unchanged, including permission
  flags; never add permission bypasses by default. A literal `--` assigns all
  remaining arguments to Codex. Repository context cannot become process options.
