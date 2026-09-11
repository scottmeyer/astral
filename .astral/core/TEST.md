# Test

Run the repository's checks from its root:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked --bins
python3 -m unittest discover -s tests -p 'test_*.py'
```

Structural tests in `tests/native_binding.rs` cover native item preservation, declaration
placement, compaction controls, completion validation, inventories, and isolated
incremental state. `tests/native_transport.rs` uses real local HTTP/WS transports
with synthetic upstream events. Python tests cover native fixture capture and
independent sandbox-denial evidence. Synthetic passes do not prove provider support.

`tests/project.rs` covers manifest graphs, bounded reads, confined paths, work-item
selection, stable fingerprints and CLI errors. Validate the repository's own
bootstrap with `./target/release/astral context validate` after building.

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
