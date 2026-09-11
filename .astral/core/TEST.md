# Test

Run the repository's checks from its root:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked --bins
cargo +1.85.0 check --locked --all-targets
python3 -m unittest discover -s tests -p 'test_*.py'
```

Structural tests in `tests/native_binding.rs` cover native item preservation, declaration
placement, compaction controls, completion validation, inventories, and isolated
incremental state. `tests/native_transport.rs` uses real local HTTP/WS transports
with synthetic upstream events. Python tests cover native fixture capture and
independent sandbox-denial evidence. Synthetic passes do not prove provider support.

`tests/model_catalog.rs` exercises all catalog aliases through real local HTTP,
including exact queries, identity isolation, conditional/error responses, header
filtering, byte limits, deadlines and interrupted bodies. Catalog tests check
that no conversation state or inference ledger is created. CI also checks the
declared Rust 1.85 minimum; see [code health](../../docs/code-health.md) for the
dated live catalog check and module refactor verification.

`tests/project.rs` covers manifest graphs, bounded reads, confined paths, work-item
selection, stable fingerprints and CLI errors. Validate the repository's own
bootstrap with `./target/release/astral context validate` after building.

`tests/codex.rs` and `tests/launch_cli.rs` cover fresh staging, argument fidelity,
runtime errors, initialization and process exit behavior. Embedded initializer
templates are checked by `tests/project_init_prompt.rs`. The dated
[fresh launch receipt](../../docs/fresh-launch-verification.md) records actual
disposable Codex execution, document recall and read-only denial independently;
those checks do not establish native restoration or general repository discovery.

`tests/native_bundle.rs` validates exact native bytes, supported item shapes,
paired tool boundaries, bounded JSON, source metadata and format integrity.
`tests/native_project.rs` covers confined artifact references, metadata-only
inspection, scoped fingerprints and rejection of native/readable role overlap.
These synthetic bundle tests establish local structural behavior, not provider
decryptability, exporter completeness or successful destination import.
See the dated [bundle verification receipt](../../docs/native-bundle-verification.md).

Native staging, private launch receipts, managed proxy teardown and headless
worker launch now have focused process/transport coverage. The dated
[native launch receipt](../../docs/native-launch-verification.md) records current
local checks and disposable live execution, opaque recall, cold resume,
read-only denial and Astral worker dogfood. Those results do not qualify arbitrary
metadata, cross-account transfer or automatic worktree/save behavior.

`tests/workspace.rs`, `work_records.rs`, `git_context.rs`, and `launch_cli.rs`
cover bound worker creation/reuse, literal arguments, incomplete initialization,
locking, dirty-file preservation and record merging. `native_capture.rs` checks
the exact completed compaction boundary; `projection_save.rs` checks immutable
publication, private staging, safe work-file copying and failure preservation.
See the separate [worktree/handoff receipt](../../docs/worktree-handoff-verification.md)
for this feature's current checks and disposable live controls.

`tests/status.rs` and `tests/workspace_observation.rs` check local observation,
active owners versus abandoned staging, confined receipt/lock validation, context
drift, explicit blockers, pagination, terminal-safe rendering and no-write CLI
behavior. These tests do not establish remote thread availability or native recall.

`scripts/native_recovery_trial.py` runs explicit live-provider tests on disposable fixtures.
Its output directory, capsule, and native captures must stay private and outside
tracked source. It refuses to resume IDs absent from its own fixture index.
Use `cycle --turns 2 --capture --recall` for work, compaction, continued execution,
native capture, and separate recall. `--restricted` tests a prohibited fixture write
under read-only policy; it never escalates.

Require actual command/result events, exit status, and the expected working
directory. Recall requires an independently checked answer and absence from the
entire readable parent history before calling it opaque-only. Check encrypted
content hashes and stable native-window structure separately. Record every
failure; mark unavailable live cases NOT_TESTED. Request bytes and provider usage
are separate observations, neither an invoice-saving claim.

UBS is a supplemental local pre-commit scanner, not a runtime or CI dependency.
Use `ubs --staged --ci --no-cargo --format=json` for static staged-file checks;
run Cargo in the complete checkout as above. A staged snapshot can omit required
unchanged Rust modules. See [development checks](../../docs/development-checks.md)
for the repaired scanner and honest interpretation of findings.
