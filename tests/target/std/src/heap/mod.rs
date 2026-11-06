//! Exercise the public System allocator through its shared small-object heap.
use std::alloc::{GlobalAlloc, Layout, System};

pub fn check() {
    const COUNT: usize = 128;
    let layout = Layout::from_size_align(32, 8).unwrap();
    let mut pointers = [std::ptr::null_mut::<u8>(); COUNT];
    unsafe {
        for (index, pointer) in pointers.iter_mut().enumerate() {
            *pointer = System.alloc(layout);
            assert!(!pointer.is_null());
            pointer.write_bytes(index as u8, layout.size());
        }
        // Small allocations must share pages, rather than consume one mapping
        // each. Do not prescribe bins, exact addresses or a particular reuse order.
        let shared_page = (0..COUNT).any(|a| {
            (a + 1..COUNT).any(|b| pointers[a].addr() / 4096 == pointers[b].addr() / 4096)
        });
        assert!(shared_page);
        for index in (0..COUNT).step_by(2) {
            System.dealloc(pointers[index], layout);
        }
        for index in (1..COUNT).step_by(2) {
            assert!(
                std::slice::from_raw_parts(pointers[index], layout.size())
                    .iter()
                    .all(|&value| value == index as u8)
            );
            let next = System.realloc(pointers[index], layout, 128);
            assert!(!next.is_null());
            assert!(
                std::slice::from_raw_parts(next, layout.size())
                    .iter()
                    .all(|&value| value == index as u8)
            );
            System.dealloc(next, Layout::from_size_align(128, 8).unwrap());
        }
        // Recycled storage must still satisfy alloc_zeroed and over-alignment.
        for alignment in [8, 64, 4096, 16384] {
            let layout = Layout::from_size_align(257, alignment).unwrap();
            let pointer = System.alloc_zeroed(layout);
            assert!(!pointer.is_null());
            assert_eq!(pointer.addr() % alignment, 0);
            assert!(
                std::slice::from_raw_parts(pointer, layout.size())
                    .iter()
                    .all(|&v| v == 0)
            );
            pointer.write_bytes(0xa5, layout.size());
            System.dealloc(pointer, layout);
        }
    }
}
