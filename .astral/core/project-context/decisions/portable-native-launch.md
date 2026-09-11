# Portable native launch scope

Status: accepted and implemented within the compatibility limits below, 2026-09-11.

Explicitly exported native bundles are tracked in Git with the project.
Launching one creates a new destination-local thread from its immutable native
window; the source thread is not resumed or modified by import. Runtime thread
IDs, active process ownership and executable capability bindings remain local.

Contexts that have never had a checkpoint start fresh from their selected
documents. If `.astral/` is absent, offer an explicit best-effort initialization
using a versioned prompt embedded in the binary to ask inference to scan the
repository and build the index. Validate the generated structure; distinguish
observations, suggestions and unverified checks. Missing referenced artifacts
still fail instead of taking the fresh path.

The five launch milestones are implemented: fresh launch/initialization,
versioned native bundles, native launch, work-item/worktree launch and explicit
capture/handoff. Status, recovery, branch completion and advisory hook integration
were added afterward. Direct is the default for fresh launch; `--proxy` is
explicit. Missing referenced checkpoints are errors.

All bound workers currently require Codex 0.154.0. Native import and save use
the same-account OpenAI / `gpt-6-astra` route with managed tool rebinding. Once a
worker has been saved natively, resume it with `--proxy`, including workers
originally launched from documents. Cross-account and broader runtime support
remain unqualified.

Git can merge documents and work records under declared rules. Native checkpoint
bytes are immutable artifacts: retain competing histories and require an explicit
choice or a new reconciliation session, rather than concatenating their payloads.

See [the milestone plan](../../../../docs/launch-plan.md) for work IDs, acceptance
checks, expected command behavior and unproven portability boundaries.
