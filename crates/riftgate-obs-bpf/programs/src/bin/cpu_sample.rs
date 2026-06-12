//! Aya eBPF program slot for lightweight CPU sampling trigger points.

#![no_std]
#![no_main]

use aya_ebpf::macros::tracepoint;
use aya_ebpf::programs::TracePointContext;

#[tracepoint]
pub fn cpu_sample(ctx: TracePointContext) -> u32 {
    match try_cpu_sample(ctx) {
        Ok(ret) | Err(ret) => ret,
    }
}

fn try_cpu_sample(_ctx: TracePointContext) -> Result<u32, u32> {
    Ok(0)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
