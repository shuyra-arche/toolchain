use crate::sys::pal::broker::{self, Request};
use crate::{boxed::Box, ffi::CStr, io, num::NonZero, thread::ThreadInit, time::Duration};
use kernel_api::{
    abi::call::room::RoomQuery,
    address::{PAGE_SIZE, VirtAddr},
    model::unit::{ExitScope, UnitQuery, UnitTarget},
};
use kernel_client::call::{room, unit};

pub const DEFAULT_MIN_STACK_SIZE: usize = 256 * 1024;

pub struct Thread {
    slot: usize,
}

impl Thread {
    pub unsafe fn new(stack: usize, init: Box<ThreadInit>) -> io::Result<Self> {
        let stack = stack
            .max(16384)
            .checked_next_multiple_of(PAGE_SIZE)
            .filter(|size| *size <= isize::MAX as usize)
            .ok_or(io::const_error!(
                io::ErrorKind::InvalidInput,
                "invalid thread stack size"
            ))?;
        let start = Box::into_raw(Box::new(init));
        let reply = broker::request(Request::Spawn {
            entry: __shuyra_thread_entry as *const () as usize,
            stack,
            start: start as usize,
        });
        if reply == 0 {
            unsafe {
                drop(Box::from_raw(start));
            }
            return Err(io::const_error!(
                io::ErrorKind::Other,
                "Kernel thread operation failed"
            ));
        }
        Ok(Self { slot: reply - 1 })
    }

    pub fn join(self) {
        broker::join(self.slot);
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        broker::release(self.slot);
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
    "li s0, 0",
    "tail __shuyra_thread_start",
);
unsafe extern "C" {
    fn __shuyra_thread_entry();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __shuyra_thread_start(args: *const usize, count: usize) -> ! {
    if count != 3 {
        crate::sys::abort_internal();
    }
    let stack = unsafe { *args.add(2) };
    let tls = crate::sys::pal::tls::State::new(broker::notification(), args, count, stack);
    unsafe {
        tls.install();
    }
    let init = unsafe { *Box::from_raw((*args) as *mut Box<ThreadInit>) };
    let slot = unsafe { *args.add(1) };
    init.init()();
    unsafe {
        crate::sys::pal::tls::cleanup();
    }
    broker::completed(slot);
    unit::exit(0, 0, VirtAddr::new(0), 0, ExitScope::Task)
}

pub fn available_parallelism() -> io::Result<NonZero<usize>> {
    room::inspect(RoomQuery::Ordinary)
        .ok()
        .and_then(NonZero::new)
        .ok_or(io::Error::UNKNOWN_THREAD_COUNT)
}

pub fn current_os_id() -> Option<u64> {
    let UnitTarget::Task(task) = broker::inspect(UnitQuery::Caller)?.target else {
        return None;
    };
    Some(task.get() as u64)
}

pub fn set_name(_: &CStr) {}

pub fn yield_now() {
    room::yield_();
}

pub fn sleep(duration: Duration) {
    if let kernel_api::model::wait::Wait::At(deadline) = room::deadline_after(duration) {
        while let Err(error) = room::sleep_until(deadline) {
            if error.source != kernel_api::abi::call::room::RoomFail::Busy {
                crate::sys::abort_internal();
            }
            room::yield_();
        }
    }
}
