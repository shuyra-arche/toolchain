//! Vanllia process entry and termination.
pub(crate) mod broker;
pub(crate) mod tls;
use kernel_api::{
    address::VirtAddr,
    model::{exit::EXIT_PANIC, unit::ExitScope},
};
use kernel_client::call::unit;

pub fn unsupported<T>() -> crate::io::Result<T> {
    Err(unsupported_err())
}
pub fn unsupported_err() -> crate::io::Error {
    crate::io::Error::UNSUPPORTED_PLATFORM
}

// Kernel argument words are not C strings. No service argument convention has
// been selected, so the Rust entry receives an empty argv, never a cast of a0.
core::arch::global_asm!(
    ".section .text.entry,\"ax\"",
    ".global _start",
    "_start:",
    ".option push",
    ".option norelax",
    "la gp, __global_pointer$",
    ".option pop",
    "andi t0, sp, 15",
    "bnez t0, __shuyra_bad_entry",
    "tail __shuyra_start",
    ".text",
    ".global __shuyra_main_entry",
    "__shuyra_main_entry:",
    ".option push",
    ".option norelax",
    "la gp, __global_pointer$",
    ".option pop",
    "tail __shuyra_main",
);

#[unsafe(no_mangle)]
extern "C" fn __shuyra_bad_entry() -> ! {
    abort_internal()
}

#[unsafe(no_mangle)]
extern "C" fn __shuyra_start() -> ! {
    unsafe extern "C" {
        fn __shuyra_main_entry();
    }
    broker::run(__shuyra_main_entry as *const () as usize)
}

#[unsafe(no_mangle)]
extern "C" fn __shuyra_main() -> ! {
    let state = tls::State::new(broker::notification());
    unsafe {
        state.install();
    }
    unsafe extern "C" {
        fn main(argc: isize, argv: *const *const u8) -> i32;
    }
    let code = unsafe { main(0, core::ptr::null()) };
    terminate(code, "toolchain: std main returned")
}

pub unsafe fn init(_argc: isize, _argv: *const *const u8, _sigpipe: u8) {}
pub unsafe fn cleanup() {
    unsafe {
        tls::cleanup();
    }
}

pub fn finish(code: i32) -> ! {
    terminate(code, "toolchain: std process exit")
}
fn terminate(code: i32, note: &str) -> ! {
    unit::exit(
        u64::from(code as u32),
        0,
        VirtAddr::new(note.as_ptr() as usize),
        note.len(),
        ExitScope::Team,
    )
}
pub fn abort_internal() -> ! {
    let note = "toolchain: std abort";
    unit::exit(
        0,
        EXIT_PANIC as u64,
        VirtAddr::new(note.as_ptr() as usize),
        note.len(),
        ExitScope::Team,
    )
}
