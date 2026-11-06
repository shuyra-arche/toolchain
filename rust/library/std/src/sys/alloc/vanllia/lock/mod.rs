//! Allocator lock: no allocation, TLS or std synchronization in this path.
use crate::sync::atomic::{
    AtomicBool,
    Ordering::{Acquire, Relaxed, Release},
};
use talc::lock_api::{GuardNoSend, RawMutex};

pub(super) struct Lock(AtomicBool);

// Acquire/release protects all heap state. Contenders remain runnable and yield
// through Kernel Client directly, including during TLS bootstrap/destruction.
unsafe impl RawMutex for Lock {
    const INIT: Self = Self(AtomicBool::new(false));
    type GuardMarker = GuardNoSend;
    fn lock(&self) {
        while !self.try_lock() {
            kernel_client::call::room::starve();
        }
    }
    fn try_lock(&self) -> bool {
        self.0
            .compare_exchange(false, true, Acquire, Relaxed)
            .is_ok()
    }
    unsafe fn unlock(&self) {
        self.0.store(false, Release);
    }
}
