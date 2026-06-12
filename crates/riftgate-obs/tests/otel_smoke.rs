//! Smoke test against a local OTel collector.
//!
//! `#[ignore]` by default — run manually with:
//!
//! ```text
//! docker run --rm -p 4317:4317 -p 4318:4318 otel/opentelemetry-collector-contrib:latest
//! cargo test -p riftgate-obs -- --ignored otel_smoke
//! ```
//!
//! The CI nightly job that exercises this lives outside the workspace
//! because it requires Docker; see `examples/minimal-proxy`.

#[test]
#[ignore = "requires a local OTel collector on 127.0.0.1:4317"]
fn round_trip_against_local_otel_collector() {
    // Smoke-test stub: the actual SDK initialisation lives in the
    // `riftgate` binary's bootstrap (Phase I). When invoked here, we
    // would set up the SDK, emit a span via OtelSink, and assert the
    // span appears in the collector's debug exporter output.
    //
    // For v0.1 this test is a placeholder; the body below documents
    // the steps a contributor takes to verify end-to-end OTLP export
    // against a fresh collector.
    eprintln!(
        "manual smoke test: \
         1) start otel-collector-contrib on 4317; \
         2) run `riftgate --config examples/minimal-proxy/riftgate.toml`; \
         3) issue a request; \
         4) check the collector's debug-exporter output for `request.completed`."
    );
}
