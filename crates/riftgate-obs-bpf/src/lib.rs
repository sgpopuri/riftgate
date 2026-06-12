//! # riftgate-obs-bpf
//!
//! v0.4 eBPF programs for Riftgate's observability plane, compiled to
//! `bpfel-unknown-none` and loaded via Aya from `riftgate-obs`'s `BpfSink`.
//!
//! Per [ADR 0024](../../../docs/06-adrs/0024-ebpf-via-aya.md) the runtime
//! pulls Aya pure-Rust BPF programs covering:
//!
//! - CPU on / off-time sampling at 19 Hz (kernel `perf` cadence).
//! - Syscall stalls (latency outliers via tracepoint instrumentation).
//! - TCP retransmits per upstream backend (via kprobe).
//!
//! This crate follows the same "empty library on every non-Linux target /
//! feature-off" pattern that `riftgate-io-uring` already uses for the
//! `io_uring` path:
//!
//! - On `cfg(all(target_os = "linux", feature = "bpf"))`, the
//!   [`BACKEND_ENABLED`] descriptor is `true` and the Aya programs (when
//!   they land in the follow-on implementation PR) are loadable.
//! - Everywhere else, the crate compiles to an empty library so the
//!   workspace builds without an Aya / clang / LLVM toolchain in scope.
//!
//! The production Aya programs and their generated skeletons are deferred
//! to a follow-on implementation PR; this scaffold lands the crate manifest and
//! the public descriptor so the workspace graph is complete today.

#![doc(html_root_url = "https://docs.rs/riftgate-obs-bpf/0.1.0-dev")]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

/// Canonical relative location (from the repository root) where staged
/// compiled BPF object artifacts live for loader and verifier harnesses.
///
/// The v0.4 follow-on Aya implementation emits one object file per program
/// slot into this directory.
pub const STAGED_OBJECT_DIR: &str = "crates/riftgate-obs-bpf/obj";

/// Build-time descriptor — useful for runtime introspection and bench
/// harnesses that want to know whether the BPF backend is compiled in.
///
/// `true` only when **all** of the following are true:
/// - `cfg(target_os = "linux")`
/// - the `bpf` Cargo feature is enabled on this crate
pub const BACKEND_ENABLED: bool = cfg!(all(target_os = "linux", feature = "bpf"));

/// Symbolic name for each BPF program slot. Stable for observability and
/// for the `BpfSink` -> `riftgate-obs-bpf` wiring.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum BpfProgram {
    /// CPU on/off-time sampling at 19 Hz.
    CpuSample,
    /// Syscall-stall outlier tracepoint.
    SyscallStall,
    /// Per-upstream TCP retransmit kprobe.
    TcpRetransmit,
}

impl BpfProgram {
    /// Wire-format name. Stable for observability and runbook references.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CpuSample => "cpu_sample",
            Self::SyscallStall => "syscall_stall",
            Self::TcpRetransmit => "tcp_retransmit",
        }
    }

    /// Canonical staged object file path relative to the repository root.
    ///
    /// Example: `crates/riftgate-obs-bpf/obj/cpu_sample.bpf.o`
    #[must_use]
    pub fn staged_object_relpath(self) -> String {
        format!("{}/{}.bpf.o", STAGED_OBJECT_DIR, self.as_str())
    }
}
