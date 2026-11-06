//! Vanllia process entry and termination.
pub(crate) mod tls;
use kernel_api::model::{exit::EXIT_PANIC, identity::VirtAddr};

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
);

#[unsafe(no_mangle)]
extern "C" fn __shuyra_bad_entry() -> ! {
    abort_internal()
}

#[unsafe(no_mangle)]
extern "C" fn __shuyra_start() -> ! {
    let state = tls::State::new();
    unsafe {
        state.install();
    }
    crate::sys::thread::register_main();
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
    crate::sys::thread::stop_other_threads();
    kernel_client::call::room::reap(
        code as u32 as usize,
        VirtAddr::new(note.as_ptr() as usize),
        note.len(),
    )
}
pub fn abort_internal() -> ! {
    crate::sys::thread::stop_other_threads();
    let note = "toolchain: std abort";
    kernel_client::call::room::reap(
        EXIT_PANIC,
        VirtAddr::new(note.as_ptr() as usize),
        note.len(),
    )
}
