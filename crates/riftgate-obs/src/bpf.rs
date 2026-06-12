//! eBPF observability sink scaffold.
//!
//! Per ADR 0024, the sink is feature-gated at compile time and gated again at
//! runtime by `RIFTGATE_ENABLE_BPF=1`. This scaffold lands the sink shape and
//! non-blocking `ObservabilitySink` implementation; Aya program loading follows
//! when the `riftgate-obs-bpf` objects exist.

use riftgate_core::obs::{ObservabilityEvent, ObservabilitySink};
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};

/// Runtime environment variable that opts into loading BPF programs.
pub const RIFTGATE_ENABLE_BPF_ENV: &str = "RIFTGATE_ENABLE_BPF";

/// Runtime state of [`BpfSink`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BpfRuntimeState {
    /// The crate was compiled without the `bpf` feature or on a non-Linux host.
    CompiledOut,
    /// BPF support is compiled in, but `RIFTGATE_ENABLE_BPF=1` is absent.
    DisabledByEnv,
    /// BPF support is compiled in and runtime-enabled.
    Loaded {
        /// Stable program slot names that will be loaded by the Aya follow-on.
        programs: Vec<&'static str>,
    },
}

/// Non-blocking eBPF sink scaffold.
#[derive(Debug)]
pub struct BpfSink {
    state: BpfRuntimeState,
    published_total: AtomicU64,
    profile_events_total: AtomicU64,
}

impl BpfSink {
    /// Construct from the process environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::with_runtime_enabled(env::var(RIFTGATE_ENABLE_BPF_ENV).as_deref() == Ok("1"))
    }

    /// Construct with an explicit runtime gate, useful for tests and startup code.
    #[must_use]
    pub fn with_runtime_enabled(runtime_enabled: bool) -> Self {
        Self {
            state: runtime_state(runtime_enabled),
            published_total: AtomicU64::new(0),
            profile_events_total: AtomicU64::new(0),
        }
    }

    /// Current BPF runtime state.
    #[must_use]
    pub fn state(&self) -> &BpfRuntimeState {
        &self.state
    }

    /// `true` when BPF support is compiled in and runtime-enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        matches!(self.state, BpfRuntimeState::Loaded { .. })
    }

    /// Total events observed by this sink.
    #[must_use]
    pub fn published_total(&self) -> u64 {
        self.published_total.load(Ordering::Relaxed)
    }

    /// Total profile events observed by this sink.
    #[must_use]
    pub fn profile_events_total(&self) -> u64 {
        self.profile_events_total.load(Ordering::Relaxed)
    }
}

impl ObservabilitySink for BpfSink {
    fn publish(&self, event: ObservabilityEvent) {
        self.published_total.fetch_add(1, Ordering::Relaxed);
        if matches!(event, ObservabilityEvent::Profile { .. }) {
            self.profile_events_total.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(all(target_os = "linux", feature = "bpf"))]
fn runtime_state(runtime_enabled: bool) -> BpfRuntimeState {
    if !runtime_enabled {
        return BpfRuntimeState::DisabledByEnv;
    }

    BpfRuntimeState::Loaded {
        programs: vec![
            riftgate_obs_bpf::BpfProgram::CpuSample.as_str(),
            riftgate_obs_bpf::BpfProgram::SyscallStall.as_str(),
            riftgate_obs_bpf::BpfProgram::TcpRetransmit.as_str(),
        ],
    }
}

#[cfg(not(all(target_os = "linux", feature = "bpf")))]
fn runtime_state(_runtime_enabled: bool) -> BpfRuntimeState {
    BpfRuntimeState::CompiledOut
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::RequestId;
    use riftgate_core::obs::{Attributes, Labels, ProfileKind, ProfileSample};

    #[test]
    fn disabled_by_default_when_runtime_gate_is_off() {
        let sink = BpfSink::with_runtime_enabled(false);
        assert!(!sink.is_enabled());
        #[cfg(all(target_os = "linux", feature = "bpf"))]
        assert_eq!(sink.state(), &BpfRuntimeState::DisabledByEnv);
        #[cfg(not(all(target_os = "linux", feature = "bpf")))]
        assert_eq!(sink.state(), &BpfRuntimeState::CompiledOut);
    }

    #[test]
    fn publish_is_non_blocking_and_counts_profile_events() {
        let sink = BpfSink::with_runtime_enabled(false);
        sink.publish(ObservabilityEvent::Profile {
            kind: ProfileKind::OnCpu,
            samples: vec![ProfileSample {
                stack: vec!["worker-0".to_string()],
                weight: 1,
            }],
        });

        assert_eq!(sink.published_total(), 1);
        assert_eq!(sink.profile_events_total(), 1);
    }

    #[test]
    fn publish_counts_non_profile_events_without_profile_increment() {
        let sink = BpfSink::with_runtime_enabled(false);
        sink.publish(ObservabilityEvent::Counter {
            name: "riftgate_test_counter_total",
            value: 1,
            labels: Labels::new(),
        });
        sink.publish(ObservabilityEvent::SpanStart {
            request_id: RequestId(42),
            name: "request.received",
            attributes: Attributes::new(),
        });

        assert_eq!(sink.published_total(), 2);
        assert_eq!(sink.profile_events_total(), 0);
    }

    #[cfg(all(target_os = "linux", feature = "bpf"))]
    #[test]
    fn runtime_enabled_lists_program_slots() {
        let sink = BpfSink::with_runtime_enabled(true);
        assert!(sink.is_enabled());
        match sink.state() {
            BpfRuntimeState::Loaded { programs } => {
                assert_eq!(programs, &["cpu_sample", "syscall_stall", "tcp_retransmit"]);
            }
            other => panic!("expected loaded state, got {other:?}"),
        }
    }
}
