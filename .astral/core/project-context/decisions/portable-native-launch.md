# Portable native launch scope

Status: launch plan and accepted user choices, 2026-09-11.

Explicitly exported native bundles will be tracked in Git with the project.
Launching one creates a new destination-local thread from its immutable native
window; the source thread is not resumed or modified by import. Runtime thread
IDs, active process ownership and executable capability bindings remain local.

Contexts that have never had a checkpoint start fresh from their selected
documents. If `.astral/` is absent, offer an explicit best-effort initialization
using a versioned prompt embedded in the binary to ask inference to scan the
repository and build the index. Validate the generated structure; distinguish
observations, suggestions and unverified checks. Missing referenced artifacts
still fail instead of taking the fresh path.

Implement five measurable milestones: fresh launch and initialization, versioned
native bundles, native launch, work-item/worktree launch, then capture and handoff.
The first usable native launch targets the existing checkout and the supported
Codex route. Direct is
the default; `--proxy` is explicit. Missing referenced checkpoints are errors.

Git can merge documents and work records under declared rules. Native checkpoint
bytes are immutable artifacts: retain competing histories and require an explicit
choice or a new reconciliation session, rather than concatenating their payloads.

See [the milestone plan](../../../../docs/launch-plan.md) for work IDs, acceptance
checks, expected command behavior and unproven portability boundaries.
