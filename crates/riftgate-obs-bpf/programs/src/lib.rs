//! Host-safe entry surface for the Aya eBPF program-source crate.
//!
//! The actual eBPF binaries are feature-gated and compile only when
//! `bpf-programs` is enabled for the `bpfel-unknown-none` target.

// Required for the bpfel-unknown-none target; no-op on host targets since
// the lib has no code that touches std.
#![no_std]
