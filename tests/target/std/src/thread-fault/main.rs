fn main() {
    let worker = std::thread::spawn(|| unsafe {
        // A real Kernel-observed illegal instruction, outside Rust panic.
        core::arch::asm!(".word 0", options(noreturn));
    });
    let _ = worker.join();
    std::process::exit(91);
}
