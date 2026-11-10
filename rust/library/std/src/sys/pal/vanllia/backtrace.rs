//! Capture the current Rust Task's RV64 frame chain.

use core::{arch::asm, ffi::c_void, ptr};

const FRAME_LIMIT: usize = 128;
const RECORD_SIZE: usize = 2 * size_of::<usize>();

#[derive(Clone)]
pub(crate) struct Frame(usize);

impl Frame {
    pub(crate) fn ip(&self) -> *mut c_void {
        ptr::without_provenance_mut(self.0)
    }

    pub(crate) fn sp(&self) -> *mut c_void {
        ptr::null_mut()
    }

    pub(crate) fn symbol_address(&self) -> *mut c_void {
        self.ip()
    }

    pub(crate) fn module_base_address(&self) -> Option<*mut c_void> {
        None
    }
}

#[inline(never)]
pub(crate) unsafe fn trace(callback: &mut dyn FnMut(&crate::backtrace_rs::Frame) -> bool) {
    let Some(stack) = super::tls::stack() else {
        return;
    };
    let mut fp: usize;
    unsafe { asm!("mv {}, s0", out(reg) fp, options(nomem, nostack)) };

    for _ in 0..FRAME_LIMIT {
        if !fp.is_multiple_of(16) || fp < stack.start + RECORD_SIZE || fp > stack.end {
            break;
        }
        let parent: usize;
        let return_pc: usize;
        // RV64 s0 points just above the saved return PC and caller's s0.
        // A malformed chain can name uninitialized stack bytes. Read the
        // machine words directly so Rust never loads an uninitialized value.
        unsafe {
            asm!(
                "ld {parent}, -16({fp})",
                "ld {pc}, -8({fp})",
                fp = in(reg) fp,
                parent = out(reg) parent,
                pc = out(reg) return_pc,
                options(readonly, nostack),
            );
        }
        if return_pc == 0 || !return_pc.is_multiple_of(2) {
            break;
        }
        if !callback(&crate::backtrace_rs::Frame {
            inner: Frame(return_pc),
        }) || parent <= fp
        {
            break;
        }
        fp = parent;
    }
}
