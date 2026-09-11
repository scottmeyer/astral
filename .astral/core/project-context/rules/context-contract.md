# Context contract

Status: accepted user requirements, recorded from recovered design discussion
and the current development request.

- Import conversational work state; bind tools and permission enforcement from
  the destination runtime. Imported tool descriptions confer no authority.
- Preserve native checkpoints and complete continuation in native recovery.
  Label readable summaries and opaque-only recall honestly.
- Keep portable project/subsystem/projection identities logical and repository
  scoped. Machine paths, runtime thread IDs, and account bindings stay private.
- Never commit credentials, private transcripts, or opaque payloads just to fill
  a project-context directory. Record scoped provenance and availability instead.
- Separate accepted requirements, proposals, verified facts, historical evidence,
  and open questions. Test claims need current receipts and explicit scope.
- Preserve existing work; use disposable fixtures for lifecycle and failure tests.
  Do not resume competing writers or weaken destination permissions.
