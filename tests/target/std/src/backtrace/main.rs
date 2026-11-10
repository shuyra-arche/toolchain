use kernel_api::address::VirtAddr;
use kernel_client::call::{debug, room};
use std::{
    backtrace::{Backtrace, BacktraceStatus},
    hint::black_box,
    thread,
};

#[inline(never)]
#[unsafe(no_mangle)]
fn std_backtrace_leaf() -> Backtrace {
    let trace = Backtrace::force_capture();
    assert_eq!(trace.status(), BacktraceStatus::Captured);
    trace
}

#[inline(never)]
#[unsafe(no_mangle)]
fn std_backtrace_main_frame() -> Backtrace {
    black_box((std_backtrace_leaf(), 1)).0
}

#[inline(never)]
#[unsafe(no_mangle)]
fn std_backtrace_worker_frame() -> Backtrace {
    black_box((std_backtrace_leaf(), 2)).0
}

fn report(kind: &str, trace: Backtrace) {
    let text = format!("BEGIN std-backtrace {kind}\n{trace}END std-backtrace {kind}\n");
    let mut bytes = text.as_bytes();
    let deadline = room::clock().saturating_add(2_000_000_000);
    while !bytes.is_empty() {
        let size = bytes.len().min(256);
        let n = debug::put(VirtAddr::new(bytes.as_ptr() as usize), size).unwrap();
        if n == 0 {
            assert!(room::clock() < deadline, "debug console stalled");
            room::yield_();
        }
        bytes = &bytes[n..];
    }
}

fn main() {
    report("main", std_backtrace_main_frame());
    let worker = thread::Builder::new()
        .stack_size(64 * 1024 + 1)
        .spawn(std_backtrace_worker_frame)
        .unwrap();
    report("worker", worker.join().unwrap());
}
