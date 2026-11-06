use crate::sync::atomic::{
    AtomicBool, AtomicUsize,
    Ordering::{AcqRel, Acquire, Relaxed, Release},
};
use crate::{
    boxed::Box, ffi::CStr, io, num::NonZero, sync::Arc, thread::ThreadInit, time::Duration,
};
use kernel_api::model::{
    identity::{TaskId, TeamId, VirtAddr},
    unit::{JoinReply, UnitTarget},
    wait::Wait,
};
use kernel_client::call::{room, unit};

pub const DEFAULT_MIN_STACK_SIZE: usize = 256 * 1024;
// Kernel currently admits at most 1024 members. Include the initial task.
static TASKS: [AtomicUsize; 1024] = [const { AtomicUsize::new(0) }; 1024];
static REGISTRY: AtomicBool = AtomicBool::new(false);
static EXITING: AtomicUsize = AtomicUsize::new(0);
struct Registry;
impl Registry {
    fn lock() -> Self {
        while REGISTRY
            .compare_exchange(false, true, Acquire, Relaxed)
            .is_err()
        {
            yield_now();
        }
        Self
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        REGISTRY.store(false, Release);
    }
}
pub fn register_main() {
    TASKS[0].store(unit::self_id().get(), Release);
}
pub struct Thread {
    id: TaskId,
    done: Arc<AtomicBool>,
}
struct Start {
    init: Box<ThreadInit>,
    done: Arc<AtomicBool>,
}
impl Thread {
    pub unsafe fn new(stack: usize, init: Box<ThreadInit>) -> io::Result<Self> {
        let stack = stack
            .max(16384)
            .checked_next_multiple_of(4096)
            .filter(|size| *size <= isize::MAX as usize)
            .ok_or(io::const_error!(
                io::ErrorKind::InvalidInput,
                "invalid thread stack size"
            ))?;
        let done = Arc::new(AtomicBool::new(false));
        let start = Box::into_raw(Box::new(Start {
            init,
            done: done.clone(),
        }));
        let result = (|| {
            let _guard = Registry::lock();
            if EXITING.load(Acquire) != 0 {
                return Err(io::const_error!(
                    io::ErrorKind::Other,
                    "Kernel thread operation failed"
                ));
            }
            let slot = TASKS
                .iter()
                .skip(1)
                .find(|slot| slot.load(Relaxed) == 0)
                .ok_or(io::const_error!(
                    io::ErrorKind::OutOfMemory,
                    "thread registry full"
                ))?;
            let args = [start as usize];
            let id = unit::spawn(
                TeamId::new(0),
                __shuyra_thread_entry as *const () as usize,
                VirtAddr::new(args.as_ptr() as usize),
                1,
                stack,
            )
            .map_err(|_| {
                io::const_error!(io::ErrorKind::Other, "Kernel thread operation failed")
            })?;
            slot.store(id.get(), Release);
            if unit::embark(UnitTarget::Task(id)).is_err() {
                let _ = unit::slay(UnitTarget::Task(id));
                slot.store(0, Release);
                return Err(io::const_error!(
                    io::ErrorKind::Other,
                    "Kernel thread operation failed"
                ));
            }
            Ok(Self { id, done })
        })();
        if result.is_err() {
            unsafe {
                drop(Box::from_raw(start));
            }
        }
        result
    }
    pub fn join(self) {
        while !self.done.load(Acquire) {
            // A Kernel fault can terminate a task before Rust cleanup runs.
            // Never wait forever for a completion flag it can no longer set.
            if !matches!(
                unit::join(UnitTarget::Task(self.id), Wait::POLL, false),
                Ok(JoinReply::Pending)
            ) && !self.done.load(Acquire)
            {
                crate::sys::abort_internal();
            }
            yield_now();
        }
    }
}
core::arch::global_asm!(
    ".text",
    ".global __shuyra_thread_entry",
    "__shuyra_thread_entry:",
    ".option push",
    ".option norelax",
    "la gp, __global_pointer$",
    ".option pop",
    "tail __shuyra_thread_start",
);
unsafe extern "C" {
    fn __shuyra_thread_entry();
}
#[unsafe(no_mangle)]
unsafe extern "C" fn __shuyra_thread_start(args: *const usize, count: usize) -> ! {
    let tls = crate::sys::pal::tls::State::new();
    unsafe {
        tls.install();
    }
    if count != 1 {
        crate::sys::abort_internal();
    }
    let Start { init, done } = unsafe { *Box::from_raw((*args) as *mut Start) };
    let start = init.init();
    start();
    unsafe {
        crate::sys::pal::tls::cleanup();
    }
    // All user code, closure storage and TLS destructors have finished. Publish
    // with release; join acquires it before reading the upstream result packet.
    done.store(true, Release);
    drop(done);
    let id = unit::self_id().get();
    {
        let _guard = Registry::lock();
        if let Some(slot) = TASKS.iter().find(|slot| slot.load(Relaxed) == id) {
            slot.store(0, Release);
        }
    }
    room::reap(0, VirtAddr::new(0), 0)
}

// Serialize against publication, stop every other std-managed task, and wait
// until it can no longer execute. No allocation, std lock, or TLS is used here.
pub fn stop_other_threads() {
    let me = unit::self_id().get();
    if EXITING.compare_exchange(0, me, AcqRel, Acquire).is_err() {
        loop {
            yield_now();
        }
    }
    let _guard = Registry::lock();
    for slot in &TASKS {
        let id = slot.load(Acquire);
        if id == 0 || id == me {
            continue;
        }
        let target = UnitTarget::Task(TaskId::new(id));
        let _ = unit::slay(target);
        while matches!(
            unit::join(target, Wait::POLL, false),
            Ok(JoinReply::Pending)
        ) {
            yield_now();
        }
    }
}
pub fn available_parallelism() -> io::Result<NonZero<usize>> {
    Err(io::Error::UNKNOWN_THREAD_COUNT)
}
pub fn current_os_id() -> Option<u64> {
    Some(unit::self_id().get() as u64)
}
pub fn set_name(_: &CStr) {}
pub fn yield_now() {
    room::starve();
}
pub fn sleep(duration: Duration) {
    let start = crate::time::Instant::now();
    while start.elapsed() < duration {
        let remaining = duration.saturating_sub(start.elapsed());
        let nanos = u64::try_from(remaining.as_nanos()).unwrap_or(u64::MAX);
        let deadline = kernel_client::call::chrono::clock().saturating_add(nanos);
        if room::park_until(deadline).is_err() {
            yield_now();
        }
    }
}
