# riftgate-io-uring

Second concrete `AsyncIO` impl, backed by Linux `io_uring`. Opt-in: build
with `--features io-uring` on a Linux 5.10+ host. On every other target
(macOS, Windows, BSD, or Linux without the feature) the crate compiles to
an empty library so the workspace still builds.

- Trait surface: [`riftgate_core::io::AsyncIO`](../riftgate-core/src/io.rs).
- Decision: [ADR `0002`](../../docs/06-adrs/0002-start-on-epoll.md) — epoll
  is the default, io_uring is the v0.2+ opt-in.
- Design: [`docs/04-design/lld-io-runtime.md`](../../docs/04-design/lld-io-runtime.md).

This v0.2 ships the scaffold: feature gate, target gate, smoke construction
test, and the registration/poll plumbing wired against the `io-uring` crate.
The conformance suite under
[`crates/riftgate-io-epoll/tests/conformance.rs`](../riftgate-io-epoll/tests/conformance.rs)
will be lifted into a shared harness in v0.3 once both backends carry the
same edge-triggered drain contract end-to-end.
