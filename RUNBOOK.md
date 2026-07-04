# Runbook

Day-to-day commands for working in the Riftgate tree. This file complements [`AGENTS.md`](AGENTS.md) (which governs *what* may change and *why*) by recording *how* you build, test, run, and benchmark the code.

If a command on this page contradicts something in `AGENTS.md` or an ADR, the ADR wins and this file is a bug.

## Lima VM setup (macOS)

All implementation, build, test, and benchmark work happens inside a Lima VM (Ubuntu 24.04 LTS)
on macOS. The VM definition is tracked at [`lima/riftgate.yaml`](lima/riftgate.yaml) and
provisioned Rust stable and all system packages automatically on first boot.

### One-time macOS setup

```bash
brew install lima
```

### Create and start the VM

```bash
# First run — downloads Ubuntu 24.04 and runs cloud-init provisioning (~5-10 min).
limactl start lima/riftgate.yaml
```

Subsequent starts are fast:

```bash
limactl start riftgate      # start a stopped VM
limactl stop riftgate       # graceful shutdown
limactl list                # status of all VMs
```

### Shell into the VM

```bash
limactl shell riftgate
```

The macOS home directory is mounted read-write at the same path inside the VM.
The repo is accessible without any copy step.

### VS Code Remote-SSH (one-time macOS setup)

```bash
limactl show-ssh --format config riftgate >> ~/.ssh/config
```

Then in VS Code: **Remote-SSH: Connect to Host** -> `lima-riftgate`.
Rust Analyzer, clippy, and cargo run against the Linux toolchain transparently.

### Bootstrap workspace config inside the VM

Run once after the first `limactl start`:

```bash
limactl shell riftgate
cd <path-to-repo>
cp config/workspace.local.env.example config/workspace.local.env
# Edit: set RIFTGATE_ENV_HOST_SHORT=lima-riftgate and RIFTGATE_ENV_TOOLCHAIN=stable.
# Leave RIFTGATE_CARGO_REGISTRY_* and RIFTGATE_INTERNAL_RUST_* commented out.
$EDITOR config/workspace.local.env
./scripts/cargow check --workspace --all-targets   # smoke-check the toolchain
```

Do not run `./scripts/render-cargo-config` for the Lima profile; crates.io is
reachable directly and no cargo proxy is needed.

## Prerequisites

- Rust toolchain, channel pinned by [`rust-toolchain.toml`](rust-toolchain.toml) (currently stable, MSRV 1.85). `rustup` will install it automatically on first `cargo` invocation.
- A POSIX-y OS for the data plane: Linux (epoll, optional io_uring) or macOS (kqueue). Windows is not a supported runtime target.
- For the eBPF work landing in `v0.3`+: Linux 5.15+, `clang`, `libelf-dev`. Not required for `v0.2`.

No system services are required to build or test. The examples directory uses Docker Compose for an end-to-end mock-backend loop; run `docker compose up -d` inside the Lima VM (install Docker via `sudo apt install docker.io` if not already present).

## Local environment profile (current)

Current default implementation environment:

- OS: Lima VM (Ubuntu 24.04 LTS) on macOS; VM definition at [`lima/riftgate.yaml`](lima/riftgate.yaml)
- Network policy: full internet access via macOS host (crates.io, rustup, apt direct)
- Package install path: `sudo apt` inside the VM (provisioned automatically by cloud-init on first start)
- Container runtime: Docker Engine installable via `sudo apt install docker.io` inside the VM; image pulls work with internet access

Concrete hostnames and local filesystem paths live only in the ignored local config file `config/workspace.local.env`.

Source of truth for the profile shape is [`AGENTS.md`](AGENTS.md) §11.5.

### Bootstrap workspace config

Run once inside the VM after the first `limactl start`:

```bash
limactl shell riftgate
cd <path-to-repo>
cp config/workspace.local.env.example config/workspace.local.env
# Set: RIFTGATE_ENV_HOST_SHORT=lima-riftgate  RIFTGATE_ENV_TOOLCHAIN=stable
# Leave RIFTGATE_CARGO_REGISTRY_* and RIFTGATE_INTERNAL_RUST_* commented out.
$EDITOR config/workspace.local.env
./scripts/cargow check --workspace --all-targets
```

Do not run `./scripts/render-cargo-config` for the Lima profile; no cargo proxy is needed.

### Rust toolchain

Rust stable is provisioned automatically by the Lima VM definition. Use `scripts/cargow` for all cargo invocations:

```bash
./scripts/cargow check -p riftgate-core
./scripts/cargow test --workspace --all-features
./scripts/cargow clippy --workspace --all-targets --all-features -- --deny warnings
```

If you need the explicit toolchain form:

```bash
source config/workspace.local.env
RUSTUP_TOOLCHAIN="$RIFTGATE_ENV_TOOLCHAIN" cargo check -p riftgate-core
```

### Nightly Rust (BPF source builds only)

Install inside the Lima VM when the `--build-from-source` BPF workflow is needed:

```bash
rustup toolchain install nightly --component rust-src
```

`scripts/build-bpf-objects` detects the active nightly toolchain and injects `-Z build-std=core` automatically via `scripts/cargow`.

### One-time apt baseline

Covered automatically by Lima cloud-init provisioning. To add a package ad-hoc:

```bash
sudo apt update && sudo apt install -y <package>
```

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
- `--features bpf` on the `riftgate` binary (forwards to `riftgate-obs/bpf`) — compiles the Linux-only Aya-backed `BpfSink` path. Runtime remains opt-in via `RIFTGATE_ENABLE_BPF=1`; when unset, startup logs `DisabledByEnv`.
- `--features gpu-nvml` on the `riftgate` binary (forwards to `riftgate-obs/gpu-nvml`) — enables the Linux-only NVML FFI GPU-pressure source escape hatch.

### v0.4 runtime GPU-pressure polling (env-gated)

The binary now supports a background GPU-pressure poll loop that updates the
live routing `BackendSignals` snapshot. Enable it with:

```bash
RIFTGATE_GPU_DCGM_ENDPOINT=http://127.0.0.1:9400/metrics \
  ./target/release/riftgate --config <path>
```

When unset, the poller stays disabled and routing uses static/default signals.

### v0.4 BPF object staging workflow (Linux)

Once your local Aya build pipeline emits object files, stage them into the
canonical contract path and run the staged-load harness:

```bash
./scripts/check-bpf-toolchain
./scripts/build-bpf-objects --mode placeholder
./scripts/build-bpf-objects --mode real --build-from-source --check-only
./scripts/build-bpf-objects --mode real --build-from-source
./scripts/build-bpf-objects --mode real --from <your-bpf-build-output-dir> --check-only
./scripts/build-bpf-objects --mode real --from <your-bpf-build-output-dir>
./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier -- --ignored aya_loads_staged_object_when_present
```

`build-bpf-objects --mode placeholder` writes `ARTIFACT_FORMAT=host-placeholder`.
`build-bpf-objects --mode real` delegates to `stage-bpf-objects --verify-elf`,
which enforces ELF machine `BPF` (EM_BPF) and writes `ARTIFACT_FORMAT=staged-elf`.
Use `--build-from-source` to compile in-repo Aya programs for
`bpfel-unknown-none` before staging.
Note: there is no `rust-std` component for `bpfel-unknown-none`; source builds
use `#![no_std]` and require nightly `-Z build-std=core` plus `rust-src`.
`build-bpf-objects` automatically adds `-Z build-std=core` when a nightly
toolchain is active via `scripts/cargow`.
Add `--check-only` to print per-slot ELF type/machine and validate inputs
without copying artifacts or changing `ARTIFACT_FORMAT`.

Required source filenames in the build-output directory:

- `cpu_sample.bpf.o`
- `syscall_stall.bpf.o`
- `tcp_retransmit.bpf.o`

Staged destination paths:

- `crates/riftgate-obs-bpf/obj/cpu_sample.bpf.o`
- `crates/riftgate-obs-bpf/obj/syscall_stall.bpf.o`
- `crates/riftgate-obs-bpf/obj/tcp_retransmit.bpf.o`

## Test

The full pre-commit verification, in the order CI runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --exclude riftgate-obs-bpf-programs -- --deny warnings
cargo test --workspace --all-features --exclude riftgate-obs-bpf-programs
RUSTDOCFLAGS='--deny warnings' cargo doc --workspace --no-deps --document-private-items
```

`riftgate-obs-bpf-programs` is excluded from clippy and test because it targets `bpfel-unknown-none`
(a no-std BPF target) which cannot be linted or tested on the host toolchain. This is a pre-existing
v0.4 constraint. The BPF program crate is still checked via `scripts/build-bpf-objects`.

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
RIFTGATE__listen_addr=127.0.0.1:9090 \
  ./target/release/riftgate --config examples/01-basic-openai-proxy/riftgate.toml

# Health and readiness probes.
curl -i http://127.0.0.1:8080/health    # liveness, always 200 while the process is up
curl -i http://127.0.0.1:8080/ready     # 200 in steady state, 503 while draining

# Graceful drain.
kill -TERM "$(pgrep -f target/release/riftgate)"
#   /ready flips to 503; in-flight requests are allowed to complete up to the
#   configured drain deadline; the process exits afterward.
```

For an end-to-end run against a mock OpenAI backend (requires Docker Engine inside the Lima VM; install with `sudo apt install docker.io` if not yet present):

```bash
cd examples/01-basic-openai-proxy
docker compose up -d              # mock upstream + OTel collector
./../../target/release/riftgate --config riftgate.toml &
curl -sS http://127.0.0.1:8080/v1/chat/completions \
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
scripts/stage-bpf-objects  stage Aya .bpf.o artifacts into canonical v0.4 paths
scripts/build-bpf-objects  build and emit canonical v0.4 .bpf.o artifacts from in-repo program sources
scripts/check-bpf-toolchain  preflight host/toolchain readiness for real bpfel source builds
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

## Public handoff (Linux workspace -> Mac repo)

When syncing this workspace back to a public GitHub working tree on macOS,
exclude local environment/proxy files and avoid destructive sync flags.

### Safety rules

- Do **not** sync `config/workspace.local.env`.
- Do **not** sync `.cargo/config.toml` (local proxy wiring).
- Do **not** sync `target/` build output.
- Do **not** use `--delete` in `rsync`; the Mac repo already has files and must not be pruned.

### Recommended rsync command

```bash
rsync -av \
  --exclude '.git/' \
  --exclude 'target/' \
  --exclude 'config/workspace.local.env' \
  --exclude '.cargo/config.toml' \
  /path/to/linux/riftgate/ \
  /path/to/mac/riftgate/
```

### Optional dry run first

```bash
rsync -avn \
  --exclude '.git/' \
  --exclude 'target/' \
  --exclude 'config/workspace.local.env' \
  --exclude '.cargo/config.toml' \
  /path/to/linux/riftgate/ \
  /path/to/mac/riftgate/
```

After sync, run local validation from the Mac repo root:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## When to update this file

Whenever a workflow command in this file becomes wrong (a flag changes, a directory moves, a new optional feature ships), update it in the same PR that introduced the change. If a new subsystem deserves a "how to run it" section, add one. This file lives next to the code and ages quickly without that discipline.
