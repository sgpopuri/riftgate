# riftgate-obs-bpf

v0.4 in-tree eBPF programs for Riftgate's observability plane. Compiled to
`bpfel-unknown-none` and loaded via Aya from `riftgate-obs`'s `BpfSink`.

Per [ADR 0024](../../docs/06-adrs/0024-ebpf-via-aya.md) and
[Options 014](../../docs/05-options/014-ebpf-integration.md).

## Implementation status (pass 1: scaffold)

- Crate manifest, lib.rs scaffold, and `BpfProgram` slot enumeration land
  today. `BACKEND_ENABLED` is `true` only on Linux with the `bpf` feature
  on; everywhere else the crate compiles to an empty library so the
  workspace graph builds without Aya / clang / LLVM in scope (same pattern
  as `riftgate-io-uring`).
- Canonical staged object contract is now fixed in code: per-program objects
  are expected under `crates/riftgate-obs-bpf/obj/<slot>.bpf.o` and exposed via
  `BpfProgram::staged_object_relpath()` for loader/verifier harnesses.
- Production Aya programs (CPU on/off-time sampling at 19 Hz, syscall
  stalls, TCP retransmits per upstream) and their generated skeletons land
  in a follow-on implementation PR within the combined `v0.3 + v0.4`
  implementation phase. Building them requires a Linux host with
  Aya's prerequisites; CI gates that path separately from the macOS /
  cross-platform default build.

Per ADR 0024, the runtime is *additionally* gated by the
`RIFTGATE_ENABLE_BPF=1` environment variable and requires `CAP_BPF` on the
host. The `bpf` Cargo feature is necessary but not sufficient.

## Staging object artifacts for loader tests

Two helper paths exist:

1. Build placeholder artifacts from in-repo program sources:

```bash
./scripts/build-bpf-objects --mode placeholder
```

Current state: this produces canonical *host-placeholder* artifacts plus
`crates/riftgate-obs-bpf/obj/ARTIFACT_FORMAT=host-placeholder`. The strict
Aya load assertion remains gated for real staged BPF objects.

2. Stage real prebuilt EM_BPF artifacts from an external build output directory:

After your local Aya pipeline builds object files, stage them into the
canonical contract path:

```bash
./scripts/check-bpf-toolchain
./scripts/build-bpf-objects --mode real --build-from-source --check-only
./scripts/build-bpf-objects --mode real --build-from-source
./scripts/build-bpf-objects --mode real --from <your-bpf-build-output-dir>
./scripts/build-bpf-objects --mode real --from <your-bpf-build-output-dir> --check-only
```

`--mode real` delegates to `scripts/stage-bpf-objects --verify-elf`. Staged
files must have ELF magic and machine type `BPF` (EM_BPF); host ELF binaries
are rejected.

`--build-from-source` additionally requires local support for the
`bpfel-unknown-none` target (core artifacts/toolchain installed in the active
environment).

Expected source filenames in `<your-bpf-build-output-dir>`:

- `cpu_sample.bpf.o`
- `syscall_stall.bpf.o`
- `tcp_retransmit.bpf.o`


The script copies them to:

- `crates/riftgate-obs-bpf/obj/cpu_sample.bpf.o`
- `crates/riftgate-obs-bpf/obj/syscall_stall.bpf.o`
- `crates/riftgate-obs-bpf/obj/tcp_retransmit.bpf.o`


and writes `crates/riftgate-obs-bpf/obj/ARTIFACT_FORMAT=staged-elf` so the
ignored Aya staged-load test runs in strict load mode (no placeholder skip).

Then run the ignored staged-load harness:

```bash
./scripts/cargow test -p riftgate-obs --features bpf --test bpf_verifier -- --ignored aya_loads_staged_object_when_present
```
