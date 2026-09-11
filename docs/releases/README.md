# Publishing a release

Keep the Cargo package version, release tag and `docs/releases/vVERSION.md` in
agreement. Build and test on a clean branch before promoting it to `main`.

The CI matrix independently runs formatting, strict Clippy, Rust tests, release
builds and Python tests on Linux x86_64, both macOS architectures and Windows
x86_64. A separate job checks Rust 1.85. Platform exclusions in the source still
apply; green Windows CI does not mean Unix-only project operations are supported.

Each successful platform job runs `scripts/package_release.py` to check the
native target, smoke-test the three executables and package only explicit public
files. A clean checkout is required. Archives contain a commit/compiler manifest,
per-file hashes, release notes and license. The artifact also includes an archive
checksum. Build output is kept in `target/`; downloadable archives go in `dist/`.

After all jobs pass for the exact commit and the notes are reviewed, create and
push an annotated version tag. For example:

```sh
git tag -a v0.1.0 -m 'Astral v0.1.0'
git push origin v0.1.0
```

The tag-triggered release workflow reruns the same CI matrix. Only after every
job succeeds does it collect that run's four artifacts, verify tag/version and
archive checksums, and upload them with `SHA256SUMS` to a draft GitHub release.
It then publishes the release. Never reuse or move a published version tag.

Download the published assets and verify `SHA256SUMS` and the embedded source
commit. Extract the archive for your platform into a fresh directory and run its
`astral --version` and `astral --help`. Publishing does not replace a developer's
installed binary or alter any Git/Codex hooks or runtime configuration.

Published verification receipts: [v0.1.0](v0.1.0-verification.md).
