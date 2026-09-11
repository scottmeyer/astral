# Development checks and UBS

Astral's CI runs formatting, strict Clippy, the locked Rust tests, a release build,
and the Python tests. UBS is not a runtime, build or CI dependency. The local
pre-commit UBS requirement comes from the operator's global `AGENTS.md`.
It is an additional heuristic review aid; findings need source-level triage.

## Repaired installation

On 2026-09-11, the installed UBS 5.0.6 launcher had fixed module hashes but fetched
modules from mutable `master`. Its checksum rejection correctly stopped execution
of a module that did not match the expected version.

The installation was upgraded to [UBS 5.4.2](https://github.com/Dicklesworthstone/ultimate_bug_scanner/releases/tag/v5.4.2),
whose [fetch implementation](https://github.com/Dicklesworthstone/ultimate_bug_scanner/blob/ae6a8d77d2318c4f208b73d2ea501ca5cf6fb94f/ubs#L32-L49)
pins module/helper URLs to the release tag. The signed release manifest and its
trusted-comment signature verified against the maintainer's
[published key](https://github.com/Dicklesworthstone/ultimate_bug_scanner/blob/ae6a8d77d2318c4f208b73d2ea501ca5cf6fb94f/docs/security.md#L52).
The key was fetched over HTTPS from that pinned documentation and matched the
release notes; this was not independent out-of-band key authentication.

The launcher, 12 language modules and 349 helper assets matched the release's
hashes before and after installation. No checksum bypass was enabled. The prior
launcher/cache remains under `~/.local/share/ubs-backups/20260911-v5.0.6`.
The installed `ubs doctor --format=json` returned status `ok`, with zero failures
and zero warnings. The downloaded upstream installer was not executed.

Release commit: `ae6a8d77d2318c4f208b73d2ea501ca5cf6fb94f`.

- Launcher SHA-256: `a8fc9672e4dcc479b295fbfd30d61797c6f1aaa35e2f352300f9aa5ef1bc8154`.
- Rust module SHA-256: `c2826bd764d57f6dc668f5d0e63de2e38aee6b19ecca264a260a9c8f58b11cc3`.

## Staged Rust scans

The unmodified `ubs --staged --ci --format=json` invocation was partial: it copied
only staged files into a temporary tree, then tried Cargo against missing,
unchanged Rust modules. This is a staged-snapshot limitation, not evidence that
the real checkout fails to build.

For staged scans, use the supported static-only option and run the normal Cargo
checks in the real checkout:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked --bins
python3 -m unittest discover -s tests -p 'test_*.py'
ubs --staged --ci --no-cargo --format=json
```

`--no-cargo` skips UBS's duplicate Cargo phases, including optional dependency
audits; it does not disable release integrity checks or static detectors. Do not
report those skipped phases as UBS passes. Separate CI checks above do not include
`cargo audit`, `cargo deny`, `cargo udeps` or `cargo outdated`.

The current staged static scan completed both language modules across 13 files,
with no failed modules. It exited 1 with 2 critical, 284 warning and 93 informational
matches, so this is **not a clean scan**. Both critical matches were reviewed:

- The header-injection detector flagged an outgoing Authorization header built
  from the local API-key environment variable in a trial runner. It is not an
  HTTP response header derived from an incoming request.
- The secret-equality detector flagged comparison of a nonsecret fixture input
  hash against its expected hash. It is not an authentication-secret comparison.

The Python warnings also include intentional exact-int checks that reject booleans,
exceptions propagated to trial failure handling, a response later closed by
`with response`, and process handles owned and stopped by the caller. Counts
include repeated heuristic matches and unchanged lines in whole staged files;
they are not counts of confirmed defects. Reports remain private outside Git.

The detailed Rust static scan reported zero critical, 270 warning and 55
informational matches. Most warnings concern test assertions and unwrap/expect
calls. Review found no introduced production defect: the new context-name expect
is guarded by a successful peek, existing JSON operations have guaranteed input
types, and the SIGTERM handler moved unchanged. No detector suppressions were
added to obtain these results.
