//! Kernel mappings backing Talc heaps. This layer never calls System or TLS.
//! Each allocation retains its source and mapping until deallocation. This is
//! usable across tasks: SpaceMembers controls the mapping, and temporary
//! task-local acquisition handles are released before returning the allocation.
use crate::{
    alloc::{GlobalAlloc, Layout},
    ptr,
};

#[derive(Debug)]
pub(super) struct Pages;

// Each allocation owns an independent mapping; deallocation consumes its view.
unsafe impl GlobalAlloc for Pages {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { dealloc(ptr, layout) }
    }
}
use kernel_api::abi::call::memory::MemoryFail;
use kernel_api::model::{
    identity::{PieToken, TeamId, VirtAddr},
    memory::{
        AcquireSource, Allocation, PAGE_SIZE, Placement, ReferenceRange, ViewControl, ViewId,
        ViewSelection,
    },
};
use kernel_client::call::memory;

#[derive(Clone, Copy)]
struct Header {
    view: ViewId,
}

fn cleanup_failure() -> ! {
    crate::sys::abort_internal()
}

pub unsafe fn alloc(layout: Layout) -> *mut u8 {
    let align = layout.align().max(align_of::<Header>());
    let Some(prefix) = size_of::<Header>().checked_next_multiple_of(align) else {
        return ptr::null_mut();
    };
    let Some(size) = prefix
        .checked_add(layout.size())
        .and_then(|n| n.checked_next_multiple_of(PAGE_SIZE))
    else {
        return ptr::null_mut();
    };
    if size > isize::MAX as usize {
        return ptr::null_mut();
    }
    let Some(allocation) = Allocation::new(size, PAGE_SIZE) else {
        return ptr::null_mut();
    };
    let Some(placement) = Placement::lowest(align.max(PAGE_SIZE)) else {
        return ptr::null_mut();
    };
    let Ok(reference) = memory::acquire(AcquireSource::Allocation(allocation), PieToken::NONE)
    else {
        return ptr::null_mut();
    };
    // All validation precedes acquisition. These word arrays are the Kernel
    // API's codecs, not the in-memory layout of its Rust descriptor structs.
    let range = ReferenceRange {
        reference,
        offset: 0,
        count: size / PAGE_SIZE,
    }
    .encode();
    let placement = placement.encode();
    let attached = loop {
        let result = memory::attach(
            VirtAddr::new(range.as_ptr() as usize),
            TeamId::new(0),
            VirtAddr::new(placement.as_ptr() as usize),
            6,
            ViewControl::SpaceMembers,
        );
        if matches!(&result, Err(error) if matches!(error.source, MemoryFail::Busy)) {
            kernel_client::call::room::starve();
            continue;
        }
        break result;
    };
    let (view, address) = match attached {
        Ok(value) => value,
        Err(_) => {
            if memory::release(reference).is_err() {
                cleanup_failure()
            }
            return ptr::null_mut();
        }
    };
    if memory::release(reference).is_err() {
        cleanup_failure()
    }
    let data = unsafe { (address.get() as *mut u8).add(prefix) };
    unsafe {
        data.sub(size_of::<Header>())
            .cast::<Header>()
            .write(Header { view });
    }
    data
}

pub unsafe fn dealloc(data: *mut u8, _layout: Layout) {
    let header = unsafe { data.sub(size_of::<Header>()).cast::<Header>().read() };
    // Copy metadata first: Detach makes the header itself inaccessible.
    // Concurrent invalidation can refuse a mapping operation with Busy. Retry
    // the same view identity; never read its potentially unmapped header again.
    loop {
        match memory::detach(TeamId::new(0), header.view, ViewSelection::Whole) {
            Ok(()) => break,
            Err(error) if matches!(error.source, MemoryFail::Busy) => {
                kernel_client::call::room::starve();
            }
            Err(_) => cleanup_failure(),
        }
    }
}
