use kernel_api::{
    abi::call::room::RoomFail,
    model::{identity::VirtAddr, wait::Wait},
};
use kernel_client::call::room;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU32, AtomicUsize, Ordering::*},
    },
    thread,
    time::{Duration, Instant},
};

fn address(word: &AtomicU32) -> VirtAddr {
    VirtAddr::new(word.as_ptr() as usize)
}
fn wait(word: &AtomicU32, expected: u32, timeout: Wait) -> bool {
    // The raw ABI requires an already resident atomic word.
    word.load(Relaxed);
    loop {
        match room::wait_value(address(word), expected, timeout) {
            Ok(result) => return result,
            Err(error) if error.source == RoomFail::Busy => thread::yield_now(),
            Err(error) => panic!("conditional wait failed: {error:?}"),
        }
    }
}
fn wake(word: &AtomicU32, all: bool) -> usize {
    room::wake_value(address(word), all).unwrap()
}
fn raw_contract() {
    let word = Arc::new(AtomicU32::new(17));
    assert!(wait(&word, 18, Wait::Forever));
    assert!(!wait(&word, 17, Wait::POLL));
    assert_eq!(wake(&word, true), 0);
    assert_eq!(
        room::wait_value(VirtAddr::new(1), 0, Wait::POLL)
            .unwrap_err()
            .source,
        RoomFail::Invalid
    );
    assert_eq!(
        room::wake_value(VirtAddr::new(usize::MAX), true)
            .unwrap_err()
            .source,
        RoomFail::Invalid
    );
    assert_eq!(
        room::wait_value(VirtAddr::new(0), 0, Wait::POLL)
            .unwrap_err()
            .source,
        RoomFail::Denied
    );
    // Event beacons and value waits at the same numeric key are disjoint.
    assert!(!room::wake(address(&word).get()));
    assert!(!wait(&word, 17, Wait::AtMost(2)));
    room::wait(address(&word).get(), Wait::POLL).unwrap();
    // A wake without waiters must not leave a token for the next value wait.
    for _ in 0..8 {
        assert_eq!(wake(&word, false), 0);
        let start = Instant::now();
        assert!(!wait(&word, 17, Wait::AtMost(2)));
        assert!(start.elapsed() >= Duration::from_millis(2));
        assert_eq!(wake(&word, true), 0);
    }
    // An unchanged value still returns on notification. The nonzero wake count
    // proves the child entered a Kernel queue instead of cooperative polling.
    for timeout in [Wait::Forever, Wait::AtMost(usize::MAX - 1)] {
        let child_word = word.clone();
        let child = thread::spawn(move || assert!(wait(&child_word, 17, timeout)));
        let start = Instant::now();
        loop {
            if wake(&word, false) == 1 {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            thread::yield_now();
        }
        child.join().unwrap();
        assert_eq!(wake(&word, false), 0);
    }
    // One wake removes at most one waiter; broadcast drains the remaining queue.
    let mut children = Vec::new();
    let entered = Arc::new(AtomicUsize::new(0));
    for _ in 0..4 {
        let (word, entered) = (word.clone(), entered.clone());
        children.push(thread::spawn(move || {
            entered.fetch_add(1, Release);
            assert!(wait(&word, 17, Wait::Forever));
        }));
    }
    while entered.load(Acquire) != 4 {
        thread::yield_now();
    }
    // Let the last child reach the syscall. Completion does not depend on it:
    // the broadcast loop also covers wake racing that last queue insertion.
    thread::sleep(Duration::from_millis(5));
    assert_eq!(wake(&word, false), 1);
    let broadcast = wake(&word, true);
    assert!((2..=3).contains(&broadcast));
    let mut remaining = 3 - broadcast;
    let start = Instant::now();
    while remaining != 0 {
        let count = wake(&word, true);
        assert!(count <= remaining);
        remaining -= count;
        assert!(start.elapsed() < Duration::from_secs(3));
        thread::yield_now();
    }
    for child in children {
        child.join().unwrap();
    }
    assert_eq!(wake(&word, true), 0);
}
fn lost_wake_races() {
    let sent = Arc::new(AtomicU32::new(0));
    let received = Arc::new(AtomicU32::new(0));
    let (s, r) = (sent.clone(), received.clone());
    let child = thread::spawn(move || {
        for n in 1..=256 {
            while s.load(Acquire) != n {
                wait(&s, n - 1, Wait::Forever);
            }
            r.store(n, Release);
            wake(&r, false);
        }
    });
    for n in 1..=256 {
        sent.store(n, Release);
        if n % 2 == 0 {
            thread::yield_now();
        }
        wake(&sent, false);
        while received.load(Acquire) != n {
            wait(&received, n - 1, Wait::Forever);
        }
    }
    child.join().unwrap();
}
fn std_broadcast_and_park() {
    let state = Arc::new((Mutex::new((0, false)), Condvar::new()));
    let mut children = Vec::new();
    for _ in 0..4 {
        let state = state.clone();
        children.push(thread::spawn(move || {
            let (lock, changed) = &*state;
            let mut guard = lock.lock().unwrap();
            guard.0 += 1;
            while !guard.1 {
                guard = changed.wait(guard).unwrap();
            }
        }));
    }
    let (lock, changed) = &*state;
    loop {
        let mut guard = lock.lock().unwrap();
        if guard.0 == 4 {
            guard.1 = true;
            changed.notify_all();
            break;
        }
        drop(guard);
        thread::yield_now();
    }
    for child in children {
        child.join().unwrap();
    }
    let me = thread::current();
    let ready = Arc::new(AtomicU32::new(0));
    let copy = ready.clone();
    let child = thread::spawn(move || {
        thread::sleep(Duration::from_millis(5));
        copy.store(1, Release);
        me.unpark();
    });
    while ready.load(Acquire) == 0 {
        thread::park();
    }
    child.join().unwrap();
    // Leave a detached worker blocked; main-return cleanup must remove it from
    // the wait queue and terminate the process without scheduling it again.
    let ready = Arc::new(AtomicU32::new(0));
    let copy = ready.clone();
    thread::spawn(move || {
        copy.store(1, Release);
        loop {
            thread::park();
        }
    });
    while ready.load(Acquire) == 0 {
        thread::yield_now();
    }
    thread::sleep(Duration::from_millis(2));
}
fn main() {
    raw_contract();
    lost_wake_races();
    std_broadcast_and_park();
}
