//! Address-space-private Kernel conditional waits. The uncontended path stays
//! in userspace; comparison and queue insertion are atomic with respect to wake.
use crate::sync::atomic::{AtomicU32, Ordering};
use crate::time::{Duration, Instant};
use kernel_api::{
    abi::call::room::RoomFail,
    model::{identity::VirtAddr, wait::Wait},
};
use kernel_client::call::room;

pub type Futex = AtomicU32;
pub type Primitive = u32;
pub type SmallFutex = AtomicU32;
pub type SmallPrimitive = u32;

pub fn futex_wait(word: &Futex, expected: u32, timeout: Option<Duration>) -> bool {
    let start = Instant::now();
    loop {
        // Also faults the page in before the Kernel's resident-only atomic read.
        if word.load(Ordering::Relaxed) != expected {
            return true;
        }
        let wait = match timeout {
            None => Wait::Forever,
            Some(duration) => {
                let remaining = duration.saturating_sub(start.elapsed());
                if remaining.is_zero() {
                    return false;
                }
                let millis = remaining.as_nanos().div_ceil(1_000_000);
                Wait::AtMost(millis.min((usize::MAX - 1) as u128) as usize)
            }
        };
        match room::wait_value(VirtAddr::new(word.as_ptr() as usize), expected, wait) {
            Ok(result) => return result,
            // A concurrent mapping transaction can temporarily prevent the
            // atomic read. Retry only that transient condition, retaining the
            // original timeout; normal contention leaves the runnable queue.
            Err(error) if error.source == RoomFail::Busy => room::starve(),
            Err(_) => crate::sys::abort_internal(),
        }
    }
}

pub fn futex_wake(word: &Futex) -> bool {
    match room::wake_value(VirtAddr::new(word.as_ptr() as usize), false) {
        Ok(count) => count != 0,
        Err(_) => crate::sys::abort_internal(),
    }
}

pub fn futex_wake_all(word: &Futex) {
    if room::wake_value(VirtAddr::new(word.as_ptr() as usize), true).is_err() {
        crate::sys::abort_internal();
    }
}
