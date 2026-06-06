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
- Production Aya programs (CPU on/off-time sampling at 19 Hz, syscall
  stalls, TCP retransmits per upstream) and their generated skeletons land
  in a follow-on implementation PR within the combined `v0.3 + v0.4`
  implementation phase. Building them requires a Linux host with
  Aya's prerequisites; CI gates that path separately from the macOS /
  cross-platform default build.

Per ADR 0024, the runtime is *additionally* gated by the
`RIFTGATE_ENABLE_BPF=1` environment variable and requires `CAP_BPF` on the
host. The `bpf` Cargo feature is necessary but not sufficient.
