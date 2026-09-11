# Native bundle contract, version 1

Astral can validate a Git-trackable native bundle and inspect its metadata. A
bundle preserves a complete native item window and its continuation as data.
The separate [native launcher](native-launch.md) stages compatible bundles and
binds a destination runtime through an owned proxy. Explicit
[save and handoff](worktree-handoff.md#save-and-hand-off) captures a supported,
completed compaction from a stopped bound worker. The dated
[bundle-reader verification receipt](native-bundle-verification.md) records that
earlier milestone's checks and limitations; it is not a current execution receipt.

## Layout and identity

A bundle consists of `manifest.json` and a sibling `window.json`. The latter is a
JSON array containing the native items in their original order. Astral retains
the exact payload bytes and each raw item, including unknown fields on supported
items, encrypted strings and number spellings. It does not decode the checkpoint,
extract only the encrypted field, or replace the window with a readable summary.

The bundle's identity is the SHA-256 of its exact manifest bytes. Its projection
pins that digest; the manifest pins the payload's exact digest, length and item
count. Formatting changes therefore change identity. Treat published bundles as
immutable: a new capture creates a new bundle and advances a projection reference.
`astral save` writes a new immutable bundle and advances the named projection
after validation. It does not commit or push the export. Garbage collection is
not implemented; earlier bundles remain available.

Example layout:

```text
.astral/projections/concurrency-investigation/
  projection.toml
  handoff.md
  bundles/
    saved-context/
      manifest.json
      window.json
```

The existing projection schema gains one optional table. The digest below is a
placeholder, not a valid artifact reference:

```toml
schema_version = 1
id = "concurrency-investigation"
kind = "native-checkpoint"
subsystems = ["web"]
handoff = "handoff.md"
native_payload_in_repository = true
sources = []

[native_bundle]
manifest = "bundles/saved-context/manifest.json"
sha256 = "<64 lowercase hexadecimal characters>"
```

The manifest path is relative to that projection's directory. Absolute paths,
parent traversal, symlink components and nonregular files are rejected. The
payload filename is fixed to `window.json` in the manifest's directory. A readable
projection cannot attach a native bundle while claiming to be fresh context.
Existing readable projections remain valid without the new table.
Declared bundle manifests/payloads and exact copies cannot also serve as readable
documents, including through an unrelated fresh-linked subsystem. Such role
conflicts fail validation before context assembly.

## Manifest

All fields shown below are required. Placeholders illustrate their types; they
must be replaced with measured hashes/counts and actual provenance:

```json
{
  "schema_version": 1,
  "format": "astral-codex-native",
  "payload": {
    "file": "window.json",
    "sha256": "<payload SHA-256>",
    "bytes": 1234,
    "item_count": 3
  },
  "compatibility": {
    "runtime": "codex",
    "runtime_version": "0.154.0",
    "protocol": "openai-responses-lite",
    "provider": "openai",
    "model": "gpt-6-astra",
    "requires_tool_rebinding": true,
    "identity_scope": "same-account"
  },
  "source": {
    "project_id": "example-project",
    "revision": null,
    "dirty": true,
    "selection_sha256": "<selected input fingerprint>",
    "history_sha256": "<exporter input snapshot SHA-256>"
  },
  "capture": {
    "boundary": "completed-turn",
    "history_complete": true,
    "last_checkpoint_index": 1
  },
  "parents": []
}
```

`source.revision` is a full lowercase Git object ID (40 or 64 hexadecimal
characters), or null when unavailable. `dirty` records the source checkout's
state; a revision alone does not describe uncommitted work. `selection_sha256`
identifies the observed selected project inputs. `history_sha256` identifies the
exact exporter input snapshot bytes used to reconstruct the complete window;
that source snapshot need not be distributed with the bundle. These are source
claims, not validation against the current checkout or remote Git repository.
The source project ID must match the destination project's logical ID.

`capture.boundary` is `completed-turn` or `completed-compaction`.
`last_checkpoint_index` is the zero-based index of the final native compaction
item. The capture must attest `history_complete=true`. Parent entries are unique
manifest digests, at most 32, with no self-reference. Parents describe provenance;
they need not be present locally, and their hashes do not prove Git ancestry or
that two opaque histories have been reconciled.

Version 1 deliberately qualifies only the listed Codex 0.154.0 / OpenAI Responses
Lite / `gpt-6-astra` metadata combination. Other combinations are explicit errors.
`same-account` records the limited portability claim; it contains no account ID
or credential and does not establish that the current destination is that
account. The native launcher checks effective model/route/workspace compatibility;
same-account provenance remains an operator responsibility and cross-account
portability is unproven. Artifact validation never implies import readiness.
In particular, the Codex importer can enrich or intentionally ignore host-owned
metadata. Exact preservation in this bundle reader does not establish an exact
round trip through that importer. The launch gate rejects fields that the
supported importer discards and numeric values that typed import would round.

## Structural validation and trust

The parser rejects duplicate JSON object keys, malformed items, empty or missing
native checkpoints, invalid digests/counts, unsupported item types and unresolved
tool boundaries. Function, custom-tool and client tool-search calls require
matching outputs; duplicate IDs, orphan outputs and unresolved calls at any
checkpoint or at the end are errors. Supported native fields remain unchanged.
Legacy local-shell items are not supported by this format revision.

Supported kinds are `compaction`, `message`, `agent_message`, `reasoning`, function
and custom-tool calls/outputs, client/server tool-search calls/outputs, completed
web-search calls and completed image-generation calls. Inter-agent text/encrypted
content is retained; inspection exposes its message count, not its body or peer
identities. Image/audio input content must use self-contained `data:` URLs;
Astral never fetches media while validating. Native output strings are historical
results, not instructions to execute. A matched call/output proves structural
closure, not tool success; optional call status strings are preserved.

Runtime `additional_tools` declarations, request/reset controls and external item
references are rejected. They cannot grant capabilities or resolve missing
history. Known source messages with system/developer roles are preserved because
legitimate complete Codex windows can contain them; inspection reports those
roles. They remain historical data. This reader does not install them as trusted
instructions, execute tools, run bundled scripts, or import permissions. The
destination must supply its own trusted instructions, tools, policy and routing.

Checksums detect mismatches against the declared bytes, including ordinary
truncation. Matching hashes and closed tool pairs cannot prove an exporter did
not omit a tail and then recompute its manifest. Completion and completeness are
exporter attestations. The implemented saver requests a new compaction, verifies
its matching successful completion and exports the exact replacement window from
the pinned Codex 0.154.0 rollout format. It supports bounded standalone histories;
referenced/forked histories, rollback reconstruction, unknown metadata and
concurrent or incomplete boundaries fail explicitly. It does not claim a general
exporter for every history shape. The experimental trial helper is not production
capture code. See [save limits and failure behavior](worktree-handoff.md#save-and-hand-off).

The strict manifest schema does not accept arbitrary importer code, executable
paths, destination thread IDs, account credentials or permission settings.
Unknown fields within a supported native item are preserved; unknown manifest
fields are rejected. Opaque payloads can still encode sensitive conversational
content. Only user-authorized explicit exports should be committed or shared
through Git. Resolving, inspecting and launching context do not export it, and
save itself does not commit or share it.

## Inspect and resolve

```sh
astral context validate
astral project concurrency-investigation --inspect
astral project web --inspect
```

Validation checks every declared local bundle, just as it checks all declared
documents. A missing/corrupt declared bundle fails validation even when a different
context is selected. Inspection returns `native_artifacts` for the explicitly
selected projection and linked projections in the selected dependency closure.
Each contains confined source handles, hashes, counts and manifest metadata,
with `availability="validated"` and `runtime_binding="unbound"`. Raw native items
and encrypted content are never printed. Empty arrays describe readable-only
selections. Selected artifact hashes contribute to the selection fingerprint.

An explicit native projection selects that bundle for launch while its linked
subsystems supply current documents. A subsystem selection with distinct native
histories in its dependency closure fails rather than combining them. Inspection
still reports the selected artifacts without binding a runtime.

Native launch requires explicit `--proxy` on the supported Codex route. Missing references
and unavailable files have separate errors; none triggers fresh-context fallback.
The current readable bootstrap remains a fresh context and contains no native
bundle. This feature introduces no proxy requirement for existing fresh launches.

Default budgets are 64 KiB per native manifest, 8 MiB per payload, 4,096 payload
items, JSON depth 64, 131,072 JSON value nodes, and the existing 16 MiB aggregate
project input limit. Other declared files
remain limited to 1 MiB, with 2,048 observed files, 4,096 discovery entries and
2 MiB inspection output. Budget failures are explicit; byte counts are not token
counts. File observations are cached consistently within a load, not an atomic
snapshot of a concurrently changing repository. Confined filesystem loading
currently requires Unix; the data format is independent of machine paths.

See the [milestone plan](launch-plan.md) for the implemented launch/save flow and
[finish and resume](finish-resume.md) for the next workflow milestone.
