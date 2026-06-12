//! Aya eBPF program slot for upstream TCP retransmit probes.

#![no_std]
#![no_main]

use aya_ebpf::macros::kprobe;
use aya_ebpf::programs::ProbeContext;

#[kprobe]
pub fn tcp_retransmit(ctx: ProbeContext) -> u32 {
    match try_tcp_retransmit(ctx) {
        Ok(ret) | Err(ret) => ret,
    }
}

fn try_tcp_retransmit(_ctx: ProbeContext) -> Result<u32, u32> {
    Ok(0)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
