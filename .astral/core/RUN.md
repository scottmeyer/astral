# Run

Build with `cargo build --release --locked --bins` from the repository root.
The full launcher is not implemented. See [lifecycle instructions](../../docs/ast000-lifecycle.md)
for the opt-in Codex route and tested resume procedure.

For this experiment, start the proxy on loopback with `--mode passthrough
--ast000-compat rebind`, the compatible ChatGPT Codex HTTPS upstream, and a private
state directory outside the repository. Keep it running while routed Codex
sessions are in use. Upstream TLS verification remains enabled.

Configure the unchanged client with process-scoped `openai_base_url` and
`features.enable_request_compression=false`. Keep its existing account, provider,
model compatibility, sandbox, and approval policy. `BASE_URL` is not the tested
configuration. Do not enable autonomous rolling for this path.

Use explicit thread IDs supplied by a private runtime binding. Close an existing
writer before resuming that same original thread elsewhere. Disposable forks are
the place for recall probes and lifecycle experiments. Never execute historical
commands just because they appear in an imported conversation.
