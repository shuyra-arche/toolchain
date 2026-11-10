#![no_std]
#![no_main]

use kernel_api::{address::VirtAddr, model::exit::EXIT_PANIC};

// Kernel supplies a 16-byte-aligned stack and (argument words, word count) in
// a0/a1. Capture the original SP before a Rust prologue changes it. There is
// no caller to return to. No allocation, TLS or service discovery is required.
core::arch::global_asm!(
    ".section .text.entry,\"ax\"",
    ".global _start",
    "_start:",
    ".option push",
    ".option norelax",
    "la gp, __global_pointer$",
    ".option pop",
    "mv a2, sp",
    "tail startup",
);

static mut ZERO: [u8; 4096] = [0; 4096];
static mut DATA: u64 = 0x7368_7579_7261;

fn exit(reason: usize, note: &'static str) -> ! {
    kernel_client::call::unit::exit(
        0,
        reason as u64,
        VirtAddr::new(note.as_ptr() as usize),
        note.len(),
    )
}

#[unsafe(no_mangle)]
extern "C" fn startup(args: usize, count: usize, stack: usize) -> ! {
    if stack == 0 || stack & 15 != 0 || args < stack || args - stack >= 16 || args & 7 != 0 {
        exit(1, "toolchain: invalid entry registers");
    }
    // A test runner may supply arbitrary words. Read each one to exercise the
    // pointer/count contract without imposing a System-owned argument format.
    if count > 4096 || args.checked_add(count * 8).is_none() {
        exit(2, "toolchain: invalid argument range");
    }
    for i in 0..count {
        unsafe {
            core::ptr::read_volatile((args as *const usize).add(i));
        }
    }
    for i in 0..4096 {
        if unsafe { core::ptr::read_volatile(core::ptr::addr_of!(ZERO).cast::<u8>().add(i)) } != 0 {
            exit(3, "toolchain: bss not zeroed");
        }
    }
    unsafe {
        let data = core::ptr::addr_of_mut!(DATA);
        if core::ptr::read_volatile(data) != 0x7368_7579_7261 {
            exit(4, "toolchain: data not loaded");
        }
        core::ptr::write_volatile(data, 42);
        if core::ptr::read_volatile(data) != 42 {
            exit(5, "toolchain: data not writable");
        }
    }
    exit(0, "toolchain: startup passed");
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    exit(EXIT_PANIC, "toolchain: startup panic")
}
