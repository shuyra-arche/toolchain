//! Shared Talc heap for std::alloc::System, backed directly by Kernel mappings.
use crate::{
    alloc::{GlobalAlloc, Layout},
    ptr,
};
use talc::{TalcLock, source::GlobalAllocSource};
mod lock;
mod pages;

// Preserve the previous Talc heap granularity: grow by at least four pages.
// GlobalAllocSource returns completely free heaps to Pages; metadata may retain
// the first heap. No fixed arena, separate Runtime or task-local allocator exists.
const BLOCK_SIZE: usize = 4 * kernel_api::address::PAGE_SIZE;
static HEAP: TalcLock<lock::Lock, GlobalAllocSource<pages::Pages>> =
    TalcLock::new(GlobalAllocSource::with_block_size(pages::Pages, BLOCK_SIZE));

fn supported(layout: Layout) -> bool {
    // GlobalAllocSource adds alignment, heap metadata and block rounding before
    // constructing a Layout. Reject impossible requests before that arithmetic.
    // Two blocks conservatively cover its footer, chain pointer and rounding.
    layout
        .size()
        .checked_add(layout.align())
        .and_then(|n| n.checked_add(talc::min_first_heap_layout::<talc::DefaultBinning>().size()))
        .and_then(|n| n.checked_add(2 * BLOCK_SIZE))
        .is_some_and(|n| n <= isize::MAX as usize)
}

pub unsafe fn alloc(layout: Layout) -> *mut u8 {
    if !supported(layout) {
        return ptr::null_mut();
    }
    unsafe { HEAP.alloc(layout) }
}
pub unsafe fn alloc_zeroed(layout: Layout) -> *mut u8 {
    if !supported(layout) {
        return ptr::null_mut();
    }
    unsafe { HEAP.alloc_zeroed(layout) }
}
pub unsafe fn dealloc(ptr: *mut u8, layout: Layout) {
    unsafe { HEAP.dealloc(ptr, layout) }
}
pub unsafe fn realloc(ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
    let Ok(next) = Layout::from_size_align(size, layout.align()) else {
        return ptr::null_mut();
    };
    if !supported(next) {
        return ptr::null_mut();
    }
    unsafe { HEAP.realloc(ptr, layout, size) }
}
