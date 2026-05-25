# Runbook

Day-to-day commands for working in the Riftgate tree. This file complements [`AGENTS.md`](AGENTS.md) (which governs *what* may change and *why*) by recording *how* you build, test, run, and benchmark the code.

If a command on this page contradicts something in `AGENTS.md` or an ADR, the ADR wins and this file is a bug.

## Prerequisites

- Rust toolchain, channel pinned by [`rust-toolchain.toml`](rust-toolchain.toml) (currently stable, MSRV 1.85). `rustup` will install it automatically on first `cargo` invocation.
- A POSIX-y OS for the data plane: Linux (epoll, optional io_uring) or macOS (kqueue). Windows is not a supported runtime target.
- For the eBPF work landing in `v0.3`+: Linux 5.15+, `clang`, `libelf-dev`. Not required for `v0.2`.

No system services are required to build or test. The examples directory uses Docker Compose for an end-to-end mock-backend loop.

## Build

```bash
# Workspace check (fast; uses the dev profile, no codegen).
cargo check --workspace --all-targets

# Release build of the gateway binary.
cargo build --release -p riftgate

# Debug build of one crate (for iteration).
cargo build -p riftgate-core
```

The resulting binary lives at `target/release/riftgate`. There is no `cargo install` workflow today; distribution through `crates.io` is a `v1.0` decision per [Distribution](docs/02-mvp-roadmap.md#distribution-cratesio).

### Optional features

- `--features io-uring` (Linux only) on the `riftgate-io-uring` crate — enables the io_uring backend. The crate compiles to an empty surface on non-Linux targets.
- `--features per-core-scheduler` on the `riftgate` binary — opt into the custom `PerCoreScheduler` instead of the tokio multi-thread runtime. Default through `v0.2` is the tokio runtime; the per-core path is exercised by the `v0.2` scheduler tests.

## Test

The full pre-commit verification, in the order CI runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- --deny warnings
cargo test --workspace --all-features
RUSTDOCFLAGS='--deny warnings' cargo doc --workspace --no-deps --document-private-items
```

Primary CI parity checks all pass locally if the four commands above pass.

Additional CI jobs:

```bash
# Optional locally (CI always runs these).
cargo deny check --all-features
mdbook build
```

`cargo-deny` and `mdbook` are not required for normal development loops, but run them before
merging changes that touch dependency policy (`deny.toml`) or book/docs structure.

Targeted tests during iteration:

```bash
# One crate.
cargo test -p riftgate-core

# One module.
cargo test -p riftgate-core rate_limit::

# One function, with output captured.
cargo test -p riftgate-replay file_wal::shutdown_drains -- --nocapture

# Doc tests only.
cargo test --workspace --doc
```

### Fuzz

The HTTP/1.1 parser ships a `cargo-fuzz` target. Requires `cargo install cargo-fuzz` once.

```bash
cd crates/riftgate-parser/fuzz
cargo fuzz run http1 -- -max_total_time=60
```

## Run

The walking-skeleton binary takes a TOML config plus environment overrides. A self-contained dev loop lives under [`examples/01-basic-openai-proxy/`](examples/01-basic-openai-proxy/).

```bash
# Foreground.
./target/release/riftgate --config examples/01-basic-openai-proxy/riftgate.toml

# Override a single field at runtime (env wins over file per ADR 0012).
RIFTGATE__listen_addr=localhost:9090 \
  ./target/release/riftgate --config examples/01-basic-openai-proxy/riftgate.toml

# Health and readiness probes.
curl -i http://localhost:8080/health    # liveness, always 200 while the process is up
curl -i http://localhost:8080/ready     # 200 in steady state, 503 while draining

# Graceful drain.
kill -TERM "$(pgrep -f target/release/riftgate)"
#   /ready flips to 503; in-flight requests are allowed to complete up to the
#   configured drain deadline; the process exits afterward.
```

For an end-to-end run against a mock OpenAI backend:

```bash
cd examples/01-basic-openai-proxy
docker compose up -d              # mock upstream + OTel collector
./../../target/release/riftgate --config riftgate.toml &
curl -sS http://localhost:8080/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"gpt-4o-mini","stream":true,"messages":[{"role":"user","content":"hi"}]}'
```

The example README walks through the full traffic shape (FR-001..FR-008).

## Bench

Each crate that owns a hot path ships criterion benches under its own `benches/` directory:

```bash
# Run all benches in a crate.
cargo bench -p riftgate-core
cargo bench -p riftgate-parser
cargo bench -p riftgate-router

# Run one bench group (per crate's bench filenames).
cargo bench -p riftgate-core --bench timers
```

Reports land under `target/criterion/`. Per `AGENTS.md` §5, any number that ends up in a doc must be reproducible from `cargo bench` against a documented baseline.

## Layout cheat sheet

```text
crates/
  riftgate-core/        traits + in-core impls (allocator, timers, rate_limit, backpressure)
  riftgate-io-epoll/    AsyncIO impl via mio (epoll on Linux, kqueue on macOS)
  riftgate-io-uring/    AsyncIO impl via io_uring (Linux, scaffold landed v0.2)
  riftgate-parser/      Http1Parser + SseFramer
  riftgate-router/      RoundRobin + Weighted + CircuitBreakerArbiter
  riftgate-obs/         bounded MPSC bus + OtelSink + JsonStdoutSink + MultiSink
  riftgate-config/      TOML loader + env overlay + fail-loudly validator
  riftgate-replay/      FileWal (per-shard segment files, group-commit fdatasync)
  riftgate/             binary: bootstrap, accept loop, proxy, shutdown
docs/
  00..02-*              vision, requirements, roadmap, retrospectives
  03-architecture/      high-level + per-plane design
  04-design/            low-level designs (LLDs) per subsystem
  05-options/           Options docs (numbered, explore-3-to-5-candidates)
  06-adrs/              ADRs (Michael Nygard format)
examples/01-basic-openai-proxy/   end-to-end mock dev loop
benchmarks/             cross-crate benchmark harnesses (v0.3+)
```

## Troubleshooting

- **`/ready` returns 503 immediately on startup.** The config validator failed loudly (per ADR 0012). Re-run with `RUST_LOG=info` and inspect the structured log line that names the rejected field.
- **`riftgate_observability_dropped_total` is climbing.** The observability bus is full because the active sink is slower than the data plane. Inspect the OTel collector, raise `obs.bus_capacity` in config, or temporarily swap to `JsonStdoutSink`. The drop is intentional — the data plane never blocks on the bus.
- **No events arriving at the OTel collector.** Confirm collector endpoint and protocol in `riftgate.toml`; the `OtelSink` logs export failures at `warn` once per minute (rate-limited so a sustained outage does not flood stderr).
- **macOS, io_uring feature requested.** The `riftgate-io-uring` crate compiles to an empty surface on non-Linux targets. Build will succeed but the runtime backend is the epoll/kqueue impl.
- **`cargo bench` fails with `cannot find -lzstd` or similar.** Some criterion baselines pull system libs; install the relevant `-dev` package for your distro and re-run.
- **CI is red on `cargo fmt --check` only.** Run `cargo fmt` locally and amend.
- **CI is red on `clippy` only.** Reproduce with `cargo clippy --workspace --all-targets -- -D warnings`. We do not turn off lints; address the diagnostic.
- **CI is red on `cargo doc` only.** Reproduce with `RUSTDOCFLAGS='--deny warnings' cargo doc --workspace --no-deps --document-private-items`; most failures are broken or ambiguous intra-doc links.
- **CI is red on `mdbook` only.** Reproduce with `mdbook build`; this usually means a broken docs link or invalid SUMMARY entry.
- **CI is red on `cargo deny` only.** Reproduce with `cargo deny check --all-features`; install with `cargo install cargo-deny` if needed.
- **`cargo test` hangs in `riftgate-replay`.** The WAL flusher thread joins on shutdown; a deadlock here usually means a test forgot to drop the `FileWal` handle. Run with `RUST_BACKTRACE=1` and look for the parked thread holding `ShardState`.

## When to update this file

Whenever a workflow command in this file becomes wrong (a flag changes, a directory moves, a new optional feature ships), update it in the same PR that introduced the change. If a new subsystem deserves a "how to run it" section, add one. This file lives next to the code and ages quickly without that discipline.
