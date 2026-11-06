mod heap;
use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::process::{ExitCode, Termination};
use std::sync::atomic::{AtomicUsize, Ordering};

static DROPPED: AtomicUsize = AtomicUsize::new(0);
struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}
struct Completed;
impl Termination for Completed {
    fn report(self) -> ExitCode {
        // report runs after main's locals have been dropped, before std cleanup.
        assert_eq!(DROPPED.load(Ordering::Relaxed), 1);
        ExitCode::SUCCESS
    }
}

fn main() -> Completed {
    let _guard = Guard;
    heap::check();
    assert_eq!(std::thread::current().name(), Some("main"));
    assert_eq!(std::env::consts::OS, "vanllia");
    assert_eq!(std::env::args().len(), 0);
    let mut values = Vec::new();
    for n in 0..8192u64 {
        values.push(n);
    }
    assert_eq!(values.iter().sum::<u64>(), 8191 * 8192 / 2);
    values.truncate(32);
    values.shrink_to_fit();
    assert_eq!(values[31], 31);
    let text = String::from("Vanllia std");
    assert_eq!(*Box::new(text), "Vanllia std");
    unsafe {
        let layout = Layout::from_size_align(1000, 8192).unwrap();
        let p = System.alloc_zeroed(layout);
        assert!(!p.is_null());
        assert_eq!(p.addr() % layout.align(), 0);
        assert!(
            std::slice::from_raw_parts(p, layout.size())
                .iter()
                .all(|&b| b == 0)
        );
        p.write_bytes(0xa5, layout.size());
        // Legal Layout, but heap metadata + alignment exceeds isize::MAX:
        // fail without releasing or modifying the original allocation.
        let huge = (isize::MAX as usize) & !(layout.align() - 1);
        assert!(System.realloc(p, layout, huge).is_null());
        assert_eq!(p.read(), 0xa5);
        let p = System.realloc(p, layout, 17000);
        assert!(!p.is_null());
        assert_eq!(p.addr() % layout.align(), 0);
        assert!(
            std::slice::from_raw_parts(p, layout.size())
                .iter()
                .all(|&b| b == 0xa5)
        );
        System.dealloc(p, Layout::from_size_align(17000, layout.align()).unwrap());
        // This fits Layout and allocator arithmetic, but exceeds Kernel's
        // RV64 address-space window. Exercise Attach refusal and source rollback.
        let unmappable = Layout::from_size_align(1usize << 60, 4096).unwrap();
        for _ in 0..8 {
            assert!(System.alloc(unmappable).is_null());
        }
        // Repeated aligned allocations exercise heap reuse and backing reclamation.
        for _ in 0..256 {
            let p = System.alloc(layout);
            assert!(!p.is_null());
            p.write(42);
            System.dealloc(p, layout);
        }
    }
    assert_eq!(std::thread::spawn(|| 42).join().unwrap(), 42);
    assert_eq!(
        std::io::stdout()
            .write(b"unsupported\n")
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Unsupported
    );
    Completed
}
