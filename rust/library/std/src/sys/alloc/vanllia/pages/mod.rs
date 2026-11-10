//! Talc gets process-shared pages from the Task that owns their Kernel Memos.
use crate::alloc::{GlobalAlloc, Layout};
use crate::sys::pal::broker::{self, Request};

#[derive(Debug)]
pub(super) struct Pages;

unsafe impl GlobalAlloc for Pages {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        broker::request(Request::Allocate {
            size: layout.size(),
            align: layout.align(),
        }) as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        broker::request(Request::Free {
            address: ptr.addr(),
        });
    }
}
