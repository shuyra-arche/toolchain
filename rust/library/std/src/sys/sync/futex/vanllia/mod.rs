//! Cooperative polling until Kernel provides atomic compare-and-park.
//! Every wait rechecks the atomic after yielding; no notification can be lost.
use crate::sync::atomic::{AtomicU32, Ordering};
use crate::time::{Duration, Instant};
pub type Futex = AtomicU32;
pub type Primitive = u32;
pub type SmallFutex = AtomicU32;
pub type SmallPrimitive = u32;
pub fn futex_wait(word: &Futex, expected: u32, timeout: Option<Duration>) -> bool {
    let start = Instant::now();
    while word.load(Ordering::Relaxed) == expected {
        if timeout.is_some_and(|duration| start.elapsed() >= duration) {
            return false;
        }
        kernel_client::call::room::starve();
    }
    true
}
// Waiters remain runnable and observe the value directly. Report a possible
// waiter so upstream RwLock does not skip its writer handoff/fallback logic.
pub fn futex_wake(_: &Futex) -> bool {
    true
}
pub fn futex_wake_all(_: &Futex) {}
