//! Linux-only eBPF verifier/loader harness for v0.4.
//!
//! Per ADR 0024, this target is feature-gated and grows into concrete
//! verifier-acceptance assertions as Aya program objects land.

#[cfg(all(target_os = "linux", feature = "bpf"))]
fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[cfg(all(target_os = "linux", feature = "bpf"))]
fn staged_object_path(program: riftgate_obs_bpf::BpfProgram) -> std::path::PathBuf {
    repo_root().join(program.staged_object_relpath())
}

#[cfg(all(target_os = "linux", feature = "bpf"))]
fn staged_artifact_format_marker() -> std::path::PathBuf {
    repo_root().join("crates/riftgate-obs-bpf/obj/ARTIFACT_FORMAT")
}

#[cfg(all(target_os = "linux", feature = "bpf"))]
#[test]
fn bpf_loader_rejects_invalid_object_bytes() {
    // This is a minimal real Aya loader-path assertion that does not require
    // CAP_BPF or probe attachment. It verifies that the userspace loader is
    // wired and returns a typed error on invalid object input.
    let bad_obj = [0_u8; 16];
    let load_result = aya::Ebpf::load(&bad_obj);

    assert!(load_result.is_err());
}

#[cfg(all(target_os = "linux", feature = "bpf"))]
#[test]
fn bpf_program_slots_stay_stable() {
    let backend_enabled = std::hint::black_box(riftgate_obs_bpf::BACKEND_ENABLED);
    assert!(backend_enabled);

    let slots = [
        riftgate_obs_bpf::BpfProgram::CpuSample.as_str(),
        riftgate_obs_bpf::BpfProgram::SyscallStall.as_str(),
        riftgate_obs_bpf::BpfProgram::TcpRetransmit.as_str(),
    ];

    assert_eq!(slots, ["cpu_sample", "syscall_stall", "tcp_retransmit"]);
}

#[cfg(all(target_os = "linux", feature = "bpf"))]
#[test]
fn staged_object_paths_follow_contract() {
    let expected = [
        "crates/riftgate-obs-bpf/obj/cpu_sample.bpf.o",
        "crates/riftgate-obs-bpf/obj/syscall_stall.bpf.o",
        "crates/riftgate-obs-bpf/obj/tcp_retransmit.bpf.o",
    ];
    let actual = [
        riftgate_obs_bpf::BpfProgram::CpuSample.staged_object_relpath(),
        riftgate_obs_bpf::BpfProgram::SyscallStall.staged_object_relpath(),
        riftgate_obs_bpf::BpfProgram::TcpRetransmit.staged_object_relpath(),
    ];

    assert_eq!(actual, expected);
}

#[cfg(all(target_os = "linux", feature = "bpf"))]
#[test]
#[ignore = "requires staged BPF object artifacts under crates/riftgate-obs-bpf/obj"]
fn aya_loads_staged_object_when_present() {
    let slots = [
        riftgate_obs_bpf::BpfProgram::CpuSample,
        riftgate_obs_bpf::BpfProgram::SyscallStall,
        riftgate_obs_bpf::BpfProgram::TcpRetransmit,
    ];
    for slot in slots {
        let path = staged_object_path(slot);
        if !path.exists() {
            eprintln!(
                "skipping staged-object load test; missing {}",
                path.display()
            );
            return;
        }

        let marker = staged_artifact_format_marker();
        let is_host_placeholder = std::fs::read_to_string(&marker)
            .map(|s| s.trim() == "host-placeholder")
            .unwrap_or(false);
        if is_host_placeholder {
            eprintln!(
                "skipping staged-object load test; artifacts are host placeholders ({})",
                marker.display()
            );
            return;
        }

        let bytes = std::fs::read(&path).expect("read staged BPF object bytes");
        let load_result = aya::Ebpf::load(&bytes);
        assert!(
            load_result.is_ok(),
            "Aya failed to load staged object {}: {load_result:?}",
            path.display()
        );
    }
}
