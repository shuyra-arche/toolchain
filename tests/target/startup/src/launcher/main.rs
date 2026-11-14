#![no_std]
#![no_main]

//! Direct-Kernel test loader for a std ELF in a child Team.
use core::{fmt::Write, slice};
use kernel_api::{
    abi::{
        call::sign::SignReply,
        memory::{Action, Query, Snapshot, Source, Value},
    },
    address::{PAGE_SIZE, PageCount, VirtAddr},
    boot::{
        args,
        capsule::{Capsule, Region},
        initrd,
        ledger::{Call, Entries, Name},
    },
    model::{
        cell::{CellLevel, Placement},
        flat::SlotPosition,
        identity::{Mark, PendId, TaskId, TeamId},
        modifiable::Modifiable,
        nest::{MemoPosition, ReserveRequest},
        pend::{
            AccordRequest, AccordTarget, CreationRights, Distributable, Limits, NestRights,
            NestScope, Operations, PendInfo, PendRange, PendScope, ShareRights, Target, ViewTarget,
        },
        sign::SignSource,
        unit::{
            ControlId, CreatedUnit, ExitCause, ExitScope, UnitCreate, UnitInfo, UnitQuery,
            UnitState, UnitTarget,
        },
        wait::Wait,
    },
};
use kernel_client::call::{pend, room, share_view, sign, unit};

// Exercise the maintained request settlement path in the real launcher Tasks.
#[path = "../../../../../rust/library/std/src/sys/pal/vanllia/broker/memory.rs"]
mod memory;
use memory::Memory;

core::arch::global_asm!(
    ".section .text.entry,\"ax\"",
    ".global _start",
    "_start:",
    ".option push",
    ".option norelax",
    "la gp, __global_pointer$",
    ".option pop",
    "tail launch",
);

fn at<T>(value: &T) -> VirtAddr {
    VirtAddr::new(value as *const T as usize)
}

fn memory_failure(message: &'static str) -> ! {
    panic!("{message}")
}

fn pend_info(token: PendId) -> PendInfo {
    let mut bytes = [0; PendInfo::SIZE];
    pend::inspect(token, VirtAddr::new(bytes.as_mut_ptr() as usize)).unwrap();
    PendInfo::from_bytes(&bytes).unwrap()
}

fn reserve(
    memory: &mut Memory,
    space: PendId,
    placement: Placement,
    count: usize,
    access: u64,
) -> (MemoPosition, VirtAddr) {
    let Target::Nest(nest) = pend_info(space).target else {
        panic!("expected Nest")
    };
    let Snapshot::Nest(status) = memory.inspect(Query::Nest { token: space, nest }) else {
        panic!("expected Nest")
    };
    let Value::Reservation {
        token,
        reservation,
        address,
        ..
    } = memory
        .mutate(Action::Reserve {
            token: space,
            nest,
            expected: status.version,
            reservation: ReserveRequest {
                placement,
                count: PageCount::new(count).unwrap(),
                level: CellLevel::new(0).unwrap(),
                access,
            },
        })
        .unwrap()
    else {
        panic!("expected Reservation")
    };
    (
        MemoPosition::new(token, nest, reservation, 0).unwrap(),
        address,
    )
}

fn generation(memory: &Memory, memo: MemoPosition) -> kernel_api::model::nest::MemoGeneration {
    let Snapshot::Memo(status) = memory.inspect(Query::Memo(memo)) else {
        panic!("expected Memo")
    };
    status.generation
}

fn associate(memory: &mut Memory, first: MemoPosition, page: usize, source: Source) {
    let target = MemoPosition::new(first.token(), first.nest(), first.reservation(), page).unwrap();
    memory
        .mutate(Action::Insert {
            target,
            expected: generation(memory, target),
            source,
        })
        .unwrap();
    memory
        .mutate(Action::Publish {
            memo: target,
            expected: generation(memory, target),
            maintenance: None,
        })
        .unwrap();
}

fn stage(
    memory: &mut Memory,
    license: PendId,
    space: PendId,
    bytes: &[u8],
    pages: usize,
    modifiable: Modifiable,
) -> PendId {
    let Value::CreatedFlat { token, flat, .. } = memory
        .mutate(Action::CreateFlat {
            license,
            count: pages,
            level: CellLevel::new(0).unwrap(),
        })
        .unwrap()
    else {
        panic!("expected Flat")
    };
    let (memo, base) = reserve(
        memory,
        space,
        Placement::lowest(PAGE_SIZE).unwrap(),
        pages,
        6,
    );
    for page in 0..pages {
        let position = SlotPosition::new(token, flat, page).unwrap();
        let Snapshot::Slot(status) = memory.inspect(Query::Slot(position)) else {
            panic!("expected Slot")
        };
        let Value::Slot { version, .. } = memory
            .mutate(Action::Occupy {
                slot: position,
                expected: status.version,
            })
            .unwrap()
        else {
            panic!("expected Slot")
        };
        associate(
            memory,
            memo,
            page,
            Source::Flat {
                position,
                expected: version,
            },
        );
    }
    unsafe {
        slice::from_raw_parts_mut(base.get() as *mut u8, bytes.len()).copy_from_slice(bytes);
    }
    for page in 0..pages {
        let target =
            MemoPosition::new(memo.token(), memo.nest(), memo.reservation(), page).unwrap();
        memory
            .mutate(Action::Retract {
                memo: target,
                expected: generation(memory, target),
            })
            .unwrap();
        memory
            .mutate(Action::Remove {
                memo: target,
                expected: generation(memory, target),
            })
            .unwrap();
    }
    let Snapshot::Reservation(status) = memory.inspect(Query::Reservation {
        token: memo.token(),
        nest: memo.nest(),
        reservation: memo.reservation(),
    }) else {
        panic!("expected Reservation")
    };
    memory
        .mutate(Action::Release {
            token: memo.token(),
            nest: memo.nest(),
            reservation: memo.reservation(),
            expected: status.version,
        })
        .unwrap();
    let spec = AccordRequest {
        src: token,
        target: AccordTarget::Caller,
        mark: Mark::NONE,
        limits: Limits {
            operations: Operations::Share(ShareRights::ASSOCIATE),
            modifiable,
            distributable: Distributable::Yes,
            scope: PendScope::Pages(PendRange::Inherit),
        },
    }
    .to_bytes();
    // Keep the source Flat live through all child publications. Parent Task
    // teardown releases these fixture sources after the User Team has retired.
    share_view::derive(license, at(&spec)).unwrap()
}

fn map(memory: &mut Memory, token: PendId, space: PendId, region: Region) {
    let Target::View(ViewTarget::Share(view)) = pend_info(token).target else {
        panic!("expected Share")
    };
    let (memo, address) = reserve(
        memory,
        space,
        Placement::exact(VirtAddr::new(region.va), PAGE_SIZE).unwrap(),
        region.pages,
        region.flags,
    );
    assert_eq!(address.get(), region.va);
    for page in 0..region.pages {
        associate(
            memory,
            memo,
            page,
            Source::Share {
                token,
                view,
                page,
                count: 1,
            },
        );
    }
}

fn delegate(
    source: PendId,
    task: TaskId,
    operations: Operations,
    modifiable: Modifiable,
    scope: PendScope,
) {
    let request = AccordRequest {
        src: source,
        target: AccordTarget::Task(task),
        mark: Mark::NONE,
        limits: Limits {
            operations,
            modifiable,
            distributable: Distributable::No,
            scope,
        },
    }
    .to_bytes();
    pend::accord(at(&request)).unwrap();
}

#[unsafe(no_mangle)]
unsafe extern "C" fn launch(words: *const usize, count: usize) -> ! {
    let words = unsafe { slice::from_raw_parts(words, count) };
    let ledger =
        unsafe { slice::from_raw_parts(words[args::LEDGER] as *const u8, words[args::LEDGER_LEN]) };
    let entries = Entries::new(ledger).unwrap();
    let license = PendId::mint(entries.find(Name::Call(Call::Create)).unwrap().handle());
    let space = PendId::mint(entries.find(Name::Space).unwrap().handle());
    let mut memory = Memory::new(memory_failure);
    let bytes =
        unsafe { slice::from_raw_parts(words[args::VIEW] as *const u8, words[args::VIEW_LEN]) };
    let loader = initrd::entry_image(bytes).unwrap();
    let offset = loader.as_ptr().addr() - bytes.as_ptr().addr() + loader.len();
    let word = |at: usize| usize::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let initiator = TaskId::new(word(offset));
    let expected = (word(offset + 8) as u64, word(offset + 16) as u64);
    let program = &bytes[offset + 32..offset + 32 + word(offset + 24)];

    let spec = UnitCreate::Team {
        guard: ControlId::NONE,
        nest: None,
        member_exit: true,
    }
    .to_bytes();
    let CreatedUnit::Team {
        team,
        nest: child_space,
    } = unit::create(at(&spec)).unwrap()
    else {
        panic!("expected Team")
    };
    let image = Capsule::parse(program).unwrap();
    for index in 0..image.count {
        let region = image.region(index).unwrap();
        let data = &program[region.payload..region.payload + region.data_pages * PAGE_SIZE];
        let source = stage(
            &mut memory,
            license,
            space,
            data,
            region.pages,
            if region.flags & 4 != 0 {
                Modifiable::Yes
            } else {
                Modifiable::No
            },
        );
        map(&mut memory, source, child_space, region);
    }
    // The creator's Whole-Nest grant prepares the complete image. The child
    // starts directly at its ELF entry and has not consumed any RequestIds.
    let child = UnitCreate::Task {
        team: Some(team),
        entry: VirtAddr::new(image.entry),
        args: VirtAddr::new(0),
        count: 0,
        stack: VirtAddr::new(0),
    }
    .to_bytes();
    let CreatedUnit::Task(task) = unit::create(at(&child)).unwrap() else {
        panic!("expected Task")
    };
    delegate(
        license,
        task,
        Operations::None(CreationRights::FLAT | CreationRights::SIGN),
        Modifiable::No,
        PendScope::Object,
    );
    delegate(
        child_space,
        task,
        Operations::Nest(NestRights::INSPECT | NestRights::RESERVE),
        Modifiable::Yes,
        PendScope::Nest(NestScope::Whole),
    );
    let endpoint = sign::create(license).unwrap();
    let target = UnitTarget::Team(team);
    let source = SignSource::Join { target };
    sign::attach(endpoint, source, 0).unwrap();
    unit::resume(UnitTarget::Task(task)).unwrap();
    let deadline = room::clock().saturating_add(15_000_000_000);
    loop {
        let at = room::clock().saturating_add(5_000_000).min(deadline);
        let reply = sign::listen(endpoint, Wait::At(at)).unwrap();
        if reply == (SignReply::Source { source, fail: None }) {
            break;
        }
        if reply != SignReply::Pending || room::clock() >= deadline {
            let _ = unit::stop(target);
            panic!("Team did not exit: {reply:?}");
        }
        release_receipts(team, initiator);
    }
    for target in [target, UnitTarget::Task(initiator)] {
        let info = inspect(UnitQuery::Target(target)).unwrap();
        assert_eq!(info.state, UnitState::Exited);
        assert_eq!(info.result, Some(expected.0));
        assert_eq!(info.exit, Some((ExitCause::Reap, expected.1)));
    }
    sign::detach(endpoint, source).unwrap();
    loop {
        match unit::delete(target) {
            Ok(()) => break,
            Err(e) if e.source == kernel_api::abi::call::unit::UnitFail::Busy => {
                assert!(room::clock() < deadline);
                room::yield_();
            }
            Err(e) => panic!("Team retirement: {e:?}"),
        }
    }
    finish(0, "toolchain: std team observed")
}

fn inspect(query: UnitQuery) -> Option<UnitInfo> {
    let mut bytes = [0u8; UnitInfo::WIDTH];
    if !unit::inspect(query, VirtAddr::new(bytes.as_mut_ptr() as usize)).unwrap() {
        return None;
    }
    Some(UnitInfo::from_bytes(&bytes).unwrap())
}

fn release_receipts(team: TeamId, keep: TaskId) {
    // The controller and the broker each own a receipt. They release those
    // independently so completed workers do not fill the Team's member table.
    let mut after = TaskId::new(0);
    while let Some(info) = inspect(UnitQuery::Members { team, after }) {
        let UnitTarget::Task(task) = info.target else {
            panic!("expected Task")
        };
        after = task;
        if task != keep && info.reclaimed {
            unit::delete(info.target).unwrap();
        }
    }
}

fn finish(reason: u64, note: &str) -> ! {
    unit::exit(
        0,
        reason,
        VirtAddr::new(note.as_ptr() as usize),
        note.len(),
        ExitScope::Task,
    )
}

struct Note {
    bytes: [u8; 128],
    used: usize,
}
impl Write for Note {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.used + s.len();
        if end > self.bytes.len() {
            return Err(core::fmt::Error);
        }
        self.bytes[self.used..end].copy_from_slice(s.as_bytes());
        self.used = end;
        Ok(())
    }
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    let mut note = Note {
        bytes: [0; 128],
        used: 0,
    };
    let _ = write!(note, "launcher: {info}");
    finish(
        1,
        core::str::from_utf8(&note.bytes[..note.used]).unwrap_or("launcher: panic"),
    )
}
