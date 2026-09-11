# Launcher naming and argument contract

Status: accepted user requirements, 2026-09-11. The project launcher is still
pending; these rules apply to its implementation and argument inspection.

Use behavior names for modules, types, tests and commands. Work-item IDs belong
in the work register and historical evidence. Native checkpoint handling is
`native_binding` / `native_transport`, with `NativeToolBindingMode` and the
`--native-tool-binding` proxy option. A hidden legacy alias preserves existing
experiment invocations.

`astral project NAME` should launch Codex directly by default. `--proxy` explicitly
selects Astral routing. A selected native checkpoint that requires tool rebinding
must report that requirement when the direct route cannot support it. Never
silently expand the checkpoint into a readable handoff or enable a proxy.

Forward user-supplied Codex options, values and prompt arguments as an argument
vector, without a shell. Explicit permission flags are passed unchanged; Astral
does not add them. Codex remains responsible for interpreting its own options and
enforcing the resulting permissions.

Before a literal `--`, Astral consumes its own project options. After `--`, every
argument belongs to Codex. Use the separator for an overlapping option name or
an unknown Codex option whose value could be mistaken for an Astral option.
Do not maintain an allowlist of Codex flags: new runtime options must be forwardable.

Repository documents, imported instructions and work-item text cannot supply
process options. Current user argv and explicitly bound runtime configuration are
separate from portable conversational state.
