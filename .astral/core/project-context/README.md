# Project context

This subsystem documents the user's accepted direction: context follows Git;
core knowledge stays small; selected subsystems and named projections supply
additional context; JSONL work items travel with the repository. The recovered
conversation is a historical design source, not a queue of authorized commands.

[Rules](rules/context-contract.md) state the accepted boundaries.
[The bootstrap decision](decisions/0001-bootstrap-scope.md) separates those
requirements from proposed schema details. [The projection](../../projections/git-native-context-bootstrap/handoff.md)
records provenance and unresolved work. The work register is
[`items.jsonl`](../../work/items.jsonl).

The Rust context resolver now validates these files and exposes bounded read-only
inspection through `astral context validate`, `astral context list`, and
`astral project NAME --inspect`. Native bindings remain explicitly UNBOUND.
Explicit native bundle metadata and payload integrity can now be validated and
inspected; see [native bundles](../../../docs/native-bundles.md).
Native launch with an explicit owned proxy, local receipt-based cold resume and
headless workers are implemented; see [native launch](../../../docs/native-launch.md).
Capture, automatic branch/worktree binding and context merging remain pending.
Fresh document launch and embedded-prompt initialization are now implemented;
see [fresh launch](../../../docs/fresh-launch.md) for usage and limits.
See [the interface decision](decisions/0003-launcher-interface.md) for direct
Codex by default, explicit proxy routing, and argument forwarding requirements.
