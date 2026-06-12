//! Aya eBPF program slot for syscall stall tracepoint hooks.

#![no_std]
#![no_main]

use aya_ebpf::macros::tracepoint;
use aya_ebpf::programs::TracePointContext;

#[tracepoint]
pub fn syscall_stall(ctx: TracePointContext) -> u32 {
    match try_syscall_stall(ctx) {
        Ok(ret) | Err(ret) => ret,
    }
}

fn try_syscall_stall(_ctx: TracePointContext) -> Result<u32, u32> {
    Ok(0)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
