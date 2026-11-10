use std::{
    cell::Cell,
    sync::{
        Arc, Barrier, Condvar, Mutex, OnceLock, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
static DROPS: AtomicUsize = AtomicUsize::new(0);
static LATE_DROPS: AtomicUsize = AtomicUsize::new(0);
struct Local(usize);
impl Drop for Local {
    fn drop(&mut self) {
        assert!(LOCAL.try_with(|_| ()).is_err());
        assert!(thread::current().name().unwrap().starts_with("worker-"));
        LATE.with(|_| ());
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}
struct Late;
impl Drop for Late {
    fn drop(&mut self) {
        LATE_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}
thread_local! {
    static VALUE: Cell<usize> = const { Cell::new(0) };
    static LOCAL: Local = Local(VALUE.with(Cell::get));
    static LATE: Late = const { Late };
}
fn main() {
    const N: usize = 6;
    VALUE.set(999);
    let barrier = Arc::new(Barrier::new(N));
    let count = Arc::new(Mutex::new(0));
    let once = Arc::new(OnceLock::new());
    let rw = Arc::new(RwLock::new(0));
    let mut workers = Vec::new();
    for n in 0..N {
        let (barrier, count, once, rw) = (barrier.clone(), count.clone(), once.clone(), rw.clone());
        let from_parent = Box::new(n);
        workers.push(
            thread::Builder::new()
                .name(format!("worker-{n}"))
                .spawn(move || {
                    assert_eq!(VALUE.get(), 0);
                    VALUE.set(n + 1);
                    LOCAL.with(|v| assert_eq!(v.0, n + 1));
                    barrier.wait();
                    // Concurrent mappings/unmappings may transiently return Busy.
                    // They must not turn valid allocation/free into process abort.
                    for round in 0..32 {
                        let data = vec![n + round; 1024];
                        thread::yield_now();
                        assert!(data.iter().all(|value| *value == n + round));
                        drop(data);
                    }
                    for _ in 0..20 {
                        let mut guard = count.lock().unwrap();
                        thread::yield_now();
                        *guard += 1;
                        assert_eq!(*once.get_or_init(|| 7), 7);
                    }
                    *rw.write().unwrap() += 1;
                    assert!(*rw.read().unwrap() > 0);
                    thread::yield_now();
                    assert_eq!(VALUE.get(), n + 1);
                    assert_eq!(*from_parent, n);
                    drop(from_parent);
                    vec![n; 1000]
                })
                .unwrap(),
        );
    }
    for (n, worker) in workers.into_iter().enumerate() {
        assert!(worker.join().unwrap().iter().all(|v| *v == n));
    }
    assert_eq!(VALUE.get(), 999);
    assert_eq!(*count.lock().unwrap(), N * 20);
    assert_eq!(DROPS.load(Ordering::SeqCst), N);
    assert_eq!(LATE_DROPS.load(Ordering::SeqCst), N);
    // Both the JoinHandle and child heap survive the intermediate creator's exit.
    let nested = thread::spawn(|| thread::spawn(|| Box::new(vec![17; 2000])))
        .join()
        .unwrap();
    assert_eq!(nested.join().unwrap()[1999], 17);
    let mut borrowed = 0;
    thread::scope(|s| {
        s.spawn(|| borrowed = 42);
    });
    assert_eq!(borrowed, 42);
    // Detached worker's TLS cleanup and synchronization complete without join.
    let signal = Arc::new((Mutex::new(false), Condvar::new()));
    let copy = signal.clone();
    drop(thread::spawn(move || {
        let (m, c) = &*copy;
        *m.lock().unwrap() = true;
        c.notify_one();
    }));
    let (m, c) = &*signal;
    let guard = c.wait_while(m.lock().unwrap(), |done| !*done).unwrap();
    drop(guard);
    let start = Instant::now();
    let (guard, timeout) = c
        .wait_timeout(m.lock().unwrap(), Duration::from_millis(2))
        .unwrap();
    drop(guard);
    assert!(timeout.timed_out());
    assert!(start.elapsed() >= Duration::from_millis(2));
    let me = thread::current();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(2));
        me.unpark();
    })
    .join()
    .unwrap();
    thread::park(); // unpark-before-park token must be preserved.
    let start = Instant::now();
    thread::park_timeout(Duration::from_millis(2));
    assert!(start.elapsed() >= Duration::from_millis(2));
    // Rejection must release the closure without executing it.
    assert!(
        thread::Builder::new()
            .stack_size(usize::MAX)
            .spawn(|| panic!("ran failed spawn"))
            .is_err()
    );
    // Retained exit receipts must not turn the Team's member limit into a
    // lifetime cap on sequential thread creation.
    for _ in 0..1050 {
        thread::spawn(|| {}).join().unwrap();
    }
    // Main return must stop a detached worker which will never return itself.
    let ready = Arc::new(Barrier::new(2));
    let child_ready = ready.clone();
    drop(thread::spawn(move || {
        child_ready.wait();
        loop {
            thread::yield_now();
        }
    }));
    ready.wait();
}
