//! The process entry Task owns heap mappings and child exit receipts.
use crate::cell::UnsafeCell;
use crate::sync::atomic::{AtomicU32, Ordering::*};
use kernel_api::{
    abi::call::{
        room::RoomFail,
        sign::{SignFail, SignReply},
        unit::UnitFail,
    },
    address::VirtAddr,
    model::{
        identity::{Mark, PendId, TaskId},
        modifiable::Modifiable,
        pend::{
            AccordRequest, AccordTarget, CreationRights, Distributable, Limits, Operations,
            PendInfo, PendRange, PendStatus,
        },
        sign::{SignRights, SignSource},
        unit::{UnitCreate, UnitInfo, UnitQuery, UnitTarget},
        wait::Wait,
    },
};
use kernel_client::call::{pend, room, sign, unit};

mod pages;

const IDLE: u32 = 0;
const WRITING: u32 = 1;
const REQUESTED: u32 = 2;
const ANSWERED: u32 = 3;
const RUST_DONE: u32 = 1;
const KERNEL_DONE: u32 = 2;
const RELEASED: u32 = 4;

// The Kernel allows 1024 members, including the broker and the Rust main Task.
const WORKERS: usize = 1022;
static WORKER: [AtomicU32; WORKERS] = [const { AtomicU32::new(RELEASED) }; WORKERS];

#[derive(Clone, Copy)]
pub(crate) enum Request {
    Allocate {
        size: usize,
        align: usize,
    },
    Free {
        address: usize,
    },
    Spawn {
        entry: usize,
        stack: usize,
        start: usize,
    },
}

struct Mailbox {
    state: AtomicU32,
    request: UnsafeCell<Request>,
    answer: UnsafeCell<usize>,
}

// The claimant writes the request before REQUESTED. Only the broker writes
// the answer, before ANSWERED. The claimant releases the slot after reading it.
unsafe impl Sync for Mailbox {}
static MAILBOX: Mailbox = Mailbox {
    state: AtomicU32::new(IDLE),
    request: UnsafeCell::new(Request::Free { address: 0 }),
    answer: UnsafeCell::new(0),
};

pub(crate) fn request(request: Request) -> usize {
    while let Err(state) = MAILBOX
        .state
        .compare_exchange(IDLE, WRITING, Acquire, Relaxed)
    {
        wait(&MAILBOX.state, state);
    }
    unsafe { MAILBOX.request.get().write(request) };
    MAILBOX.state.store(REQUESTED, Release);
    match sign::trigger(super::tls::notification()) {
        Ok(()) => {}
        Err(error) if error.source == SignFail::Busy => {}
        Err(_) => super::abort_internal(),
    }
    loop {
        let state = MAILBOX.state.load(Acquire);
        if state == ANSWERED {
            break;
        }
        wait(&MAILBOX.state, state);
    }
    let answer = unsafe { MAILBOX.answer.get().read() };
    MAILBOX.state.store(IDLE, Release);
    wake(&MAILBOX.state);
    answer
}

fn wait(word: &AtomicU32, expected: u32) {
    match room::wait_value(address(word), expected, Wait::Infinite) {
        Ok(_) => {}
        Err(error) if error.source == RoomFail::Busy => room::yield_(),
        Err(_) => super::abort_internal(),
    }
}
fn wake(word: &AtomicU32) {
    loop {
        match room::wake_value(address(word), true) {
            Ok(_) => return,
            Err(error) if error.source == RoomFail::Busy => room::yield_(),
            Err(_) => super::abort_internal(),
        }
    }
}
pub(super) fn address<T>(value: &T) -> VirtAddr {
    VirtAddr::new(value as *const T as usize)
}
pub(crate) fn notification() -> PendId {
    find(|operations| operations == Operations::Sign(SignRights::TRIGGER))
}
fn find(matches: impl Fn(Operations) -> bool) -> PendId {
    let mut after = PendId::NONE;
    loop {
        let mut bytes = [0u8; PendInfo::SIZE];
        let count = pend::collect(after, VirtAddr::new(bytes.as_mut_ptr() as usize), 1)
            .unwrap_or_else(|_| super::abort_internal());
        if count == 0 {
            super::abort_internal();
        }
        let info = PendInfo::from_bytes(&bytes).unwrap_or_else(|| super::abort_internal());
        if info.status == PendStatus::Active && matches(info.limits.operations) {
            return info.token;
        }
        after = info.token;
    }
}
pub(crate) fn completed(slot: usize) {
    WORKER[slot].fetch_or(RUST_DONE, Release);
}
pub(crate) fn join(slot: usize) {
    loop {
        let state = WORKER[slot].load(Acquire);
        if state & KERNEL_DONE != 0 {
            if state & RUST_DONE == 0 {
                super::abort_internal();
            }
            return;
        }
        wait(&WORKER[slot], state);
    }
}
pub(crate) fn release(slot: usize) {
    WORKER[slot].fetch_or(RELEASED, Release);
}

pub(crate) fn inspect(query: UnitQuery) -> Option<UnitInfo> {
    let mut bytes = [0u8; UnitInfo::WIDTH];
    if !unit::inspect(query, VirtAddr::new(bytes.as_mut_ptr() as usize)).ok()? {
        return None;
    }
    UnitInfo::from_bytes(&bytes)
}

#[derive(Clone, Copy)]
enum Worker {
    Vacant,
    Running(TaskId),
    Retiring(TaskId),
}

fn spawn(endpoint: PendId, entry: usize, stack: usize, args: &[usize]) -> Worker {
    let descriptor = UnitCreate::Task {
        team: None,
        entry: VirtAddr::new(entry),
        args: VirtAddr::new(args.as_ptr() as usize),
        count: args.len() as u64,
        stack: VirtAddr::new(stack),
    }
    .to_bytes();
    let task = match unit::create(address(&descriptor)) {
        Ok(UnitTarget::Task(task)) => task,
        Ok(_) => super::abort_internal(),
        Err(_) => return Worker::Vacant,
    };
    let target = UnitTarget::Task(task);
    let grant = AccordRequest {
        src: endpoint,
        target: AccordTarget::Task(task),
        limits: Limits {
            operations: Operations::Sign(SignRights::TRIGGER),
            modifiable: Modifiable::No,
            distributable: Distributable::No,
            range: PendRange::Inherit,
        },
        mark: Mark::NONE,
    }
    .to_bytes();
    let source = SignSource::Join { target };
    let granted = pend::accord(address(&grant)).is_ok();
    let routed = granted && sign::attach(endpoint, source, 0).is_ok();
    if routed && unit::resume(target).is_ok() {
        return Worker::Running(task);
    }
    if routed && sign::detach(endpoint, source).is_err() {
        super::abort_internal();
    }
    if unit::stop(target).is_err() {
        super::abort_internal();
    }
    Worker::Retiring(task)
}

struct Broker {
    license: PendId,
    endpoint: PendId,
    main: TaskId,
    workers: [Worker; WORKERS],
}

impl Broker {
    fn command(&mut self, request: Request) -> usize {
        match request {
            Request::Allocate { size, align } => pages::allocate(self.license, size, align),
            Request::Free { address } => {
                pages::free(address);
                0
            }
            Request::Spawn {
                entry,
                stack,
                start,
            } => {
                let Some(slot) = self.workers.iter().enumerate().position(|(slot, worker)| {
                    matches!(worker, Worker::Vacant) && WORKER[slot].load(Acquire) & RELEASED != 0
                }) else {
                    return 0;
                };
                WORKER[slot].store(0, Release);
                let worker = spawn(self.endpoint, entry, stack, &[start, slot]);
                self.workers[slot] = worker;
                if matches!(worker, Worker::Running(_)) {
                    slot + 1
                } else {
                    release(slot);
                    0
                }
            }
        }
    }

    fn retire(&mut self) -> Wait {
        let mut pending = false;
        for worker in &mut self.workers {
            let Worker::Retiring(task) = *worker else {
                continue;
            };
            match unit::delete(UnitTarget::Task(task)) {
                Ok(()) => *worker = Worker::Vacant,
                Err(error) if error.source == UnitFail::Busy => pending = true,
                Err(_) => super::abort_internal(),
            }
        }
        if pending {
            Wait::At(room::clock().saturating_add(1_000_000))
        } else {
            Wait::Infinite
        }
    }

    fn observe_exit(&mut self, source: SignSource, task: TaskId) {
        // Main exits the whole Team. A main Task fault cannot leave its peers
        // and broker blocked indefinitely.
        if task == self.main {
            super::abort_internal();
        }
        let slot = self
            .workers
            .iter()
            .position(|worker| matches!(worker, Worker::Running(id) if *id == task))
            .unwrap_or_else(|| super::abort_internal());
        if inspect(UnitQuery::Target(UnitTarget::Task(task)))
            .and_then(|info| info.exit)
            .is_none()
            || sign::detach(self.endpoint, source).is_err()
        {
            super::abort_internal();
        }
        WORKER[slot].fetch_or(KERNEL_DONE, AcqRel);
        wake(&WORKER[slot]);
        self.workers[slot] = Worker::Retiring(task);
    }

    fn serve(&mut self) -> ! {
        loop {
            if MAILBOX.state.load(Acquire) == REQUESTED {
                let answer = self.command(unsafe { MAILBOX.request.get().read() });
                unsafe { MAILBOX.answer.get().write(answer) };
                MAILBOX.state.store(ANSWERED, Release);
                wake(&MAILBOX.state);
            }
            let wait = self.retire();
            match sign::listen(self.endpoint, wait) {
                Ok(SignReply::Triggered { sequence }) => {
                    if sign::consume(self.endpoint, sequence).is_err() {
                        super::abort_internal();
                    }
                }
                Ok(SignReply::Source {
                    source:
                        source @ SignSource::Join {
                            target: UnitTarget::Task(task),
                        },
                    fail: None,
                }) => self.observe_exit(source, task),
                Ok(SignReply::Pending) => {}
                _ => super::abort_internal(),
            }
        }
    }
}

pub(super) fn run(main: usize) -> ! {
    let license = find(|ops| {
        matches!(ops, Operations::None(rights)
        if rights.contains(CreationRights::FLAT | CreationRights::SIGN))
    });
    let endpoint = sign::create(license).unwrap_or_else(|_| super::abort_internal());
    let Worker::Running(main) = spawn(endpoint, main, 256 * 1024, &[]) else {
        super::abort_internal();
    };
    Broker {
        license,
        endpoint,
        main,
        workers: [Worker::Vacant; WORKERS],
    }
    .serve()
}
