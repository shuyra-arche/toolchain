//! Key-based TLS with one stack-owned table per Kernel task, addressed by tp.
use crate::{alloc::System, boxed::Box, cell::Cell, ptr};
pub type Key = usize;
pub struct LazyKey {
    dtor: Option<unsafe extern "C" fn(*mut u8)>,
}
impl LazyKey {
    pub const fn new(dtor: Option<unsafe extern "C" fn(*mut u8)>) -> Self {
        Self { dtor }
    }
    pub fn force(&'static self) -> Key {
        self as *const Self as usize
    }
}
struct Entry {
    key: Key,
    value: Cell<*mut u8>,
    next: *mut Entry,
}
pub struct State {
    head: Cell<*mut Entry>,
}
impl State {
    pub const fn new() -> Self {
        Self {
            head: Cell::new(ptr::null_mut()),
        }
    }
    // State must stay at this address until the task stops; never expose it to
    // another task. System allocation does not access TLS or user allocators.
    pub unsafe fn install(&self) {
        unsafe {
            core::arch::asm!("mv tp, {}", in(reg) self, options(nostack));
        }
    }
}
fn state() -> &'static State {
    let address: *const State;
    unsafe {
        core::arch::asm!("mv {}, tp", out(reg) address, options(nomem, nostack));
        &*address
    }
}
pub unsafe fn get(key: Key) -> *mut u8 {
    let mut entry = state().head.get();
    while !entry.is_null() {
        let item = unsafe { &*entry };
        if item.key == key {
            return item.value.get();
        }
        entry = item.next;
    }
    ptr::null_mut()
}
pub unsafe fn set(key: Key, value: *mut u8) {
    let mut entry = state().head.get();
    while !entry.is_null() {
        let item = unsafe { &*entry };
        if item.key == key {
            item.value.set(value);
            return;
        }
        entry = item.next;
    }
    if value.is_null() {
        return;
    }
    let item = Box::new_in(
        Entry {
            key,
            value: Cell::new(value),
            next: state().head.get(),
        },
        System,
    );
    state().head.set(Box::into_raw_with_allocator(item).0);
}
pub unsafe fn cleanup() {
    // Reentrant initialization may add keys. Restart from the head each round.
    // Like key-based system TLS implementations, bound destructor resurrection.
    for _ in 0..4 {
        let mut ran = false;
        let mut entry = state().head.get();
        while !entry.is_null() {
            let item = unsafe { &*entry };
            let key = unsafe { &*(item.key as *const LazyKey) };
            if let Some(dtor) = key.dtor {
                let value = item.value.replace(ptr::null_mut());
                if !value.is_null() {
                    ran = true;
                    unsafe {
                        dtor(value);
                    }
                }
            }
            entry = item.next;
        }
        if !ran {
            break;
        }
    }
    crate::rt::thread_cleanup();
    let mut entry = state().head.replace(ptr::null_mut());
    while !entry.is_null() {
        let item = unsafe { Box::from_raw_in(entry, System) };
        entry = item.next;
        drop(item);
    }
}
