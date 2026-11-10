#![no_std]
#![no_main]

//! Direct-Kernel test loader for a std ELF in a child Team.
use core::{fmt::Write, slice};
use kernel_api::{
    abi::call::sign::SignReply,
    address::{PAGE_SIZE, VirtAddr},
    boot::{
        args,
        capsule::{Capsule, Region},
        initrd,
        ledger::{Call, Entries, Name, Page},
    },
    model::{
        cell::{
            Allocation, CellCreate, CellHandoff, CellInfo, CellObject, CellPosition, CellSource,
            CreatedCell, MappingControl, MappingTarget, MemoDestination, Placement, ReferenceRange,
        },
        identity::{Mark, PendId, TaskId, TeamId},
        modifiable::Modifiable,
        pend::{
            AccordRequest, AccordTarget, CreationRights, Distributable, Limits, Operations,
            PendInfo, PendRange, ShareRights,
        },
        sign::SignSource,
        unit::{
            ControlId, ExitCause, ExitScope, ProgramKind, UnitCreate, UnitInfo, UnitQuery,
            UnitState, UnitTarget,
        },
        wait::Wait,
    },
};
use kernel_client::call::{cell, pend, room, share_view, sign, unit};

const OWN: TeamId = TeamId::new(0);

core::arch::global_asm!(
    ".section .text.entry,\"ax\"",
    ".global _start",
    "_start:",
    ".option push",
    ".option norelax",
    "la gp, __global_pointer$",
    ".option pop",
    "tail launch",
    ".text",
    ".global child_entry",
    "child_entry:",
    ".option push",
    ".option norelax",
    "la gp, __global_pointer$",
    ".option pop",
    "tail enter",
);
unsafe extern "C" {
    fn child_entry();
}

fn at<T>(value: &T) -> VirtAddr {
    VirtAddr::new(value as *const T as usize)
}

fn position(object: CellObject, page: usize) -> CellPosition {
    let mut pos = CellPosition {
        object,
        page,
        generation: 0,
    };
    let mut info = [0u8; CellInfo::WIDTH];
    cell::inspect(
        at(&pos.to_bytes()),
        VirtAddr::new(info.as_mut_ptr() as usize),
    )
    .unwrap();
    pos.generation = CellInfo::from_bytes(&info).unwrap().generation;
    pos
}

fn memo(
    source: CellSource,
    team: TeamId,
    placement: Placement,
    access: u64,
) -> (CellObject, VirtAddr) {
    let spec = CellCreate::Memo {
        source,
        destination: MemoDestination::Internal {
            team,
            placement,
            access,
            control: MappingControl::Caller,
        },
    }
    .to_bytes();
    let CreatedCell::Memo { mapping, address } = cell::create(PendId::NONE, at(&spec)).unwrap()
    else {
        panic!("expected Memo");
    };
    (
        CellObject::Memo {
            target: MappingTarget::Cpu(team),
            mapping,
        },
        address,
    )
}

fn publish(object: CellObject, pages: usize) {
    for page in 0..pages {
        cell::publish(at(&position(object, page).to_bytes())).unwrap();
    }
}

fn stage(license: PendId, bytes: &[u8], pages: usize, modifiable: Modifiable) -> PendId {
    let spec = CellCreate::Slot(Allocation::new(pages * PAGE_SIZE, PAGE_SIZE).unwrap()).to_bytes();
    let CreatedCell::Slot(slot) = cell::create(license, at(&spec)).unwrap() else {
        panic!("expected Slot");
    };
    let (object, address) = memo(
        CellSource::Slot(ReferenceRange {
            reference: slot,
            offset: 0,
            count: pages,
        }),
        OWN,
        Placement::lowest(PAGE_SIZE).unwrap(),
        6,
    );
    for page in 0..pages {
        cell::push(at(&CellHandoff {
            source: CellPosition {
                object: CellObject::Slot(slot),
                page,
                generation: 0,
            },
            target: position(object, page),
        }
        .to_bytes()))
        .unwrap();
    }
    publish(object, pages);
    unsafe {
        slice::from_raw_parts_mut(address.get() as *mut u8, bytes.len()).copy_from_slice(bytes);
    }
    for page in 0..pages {
        cell::retract(at(&position(object, page).to_bytes())).unwrap();
    }
    cell::delete(at(&object.to_bytes())).unwrap();
    let source = share_view::create(license, slot, modifiable).unwrap();
    cell::delete(at(&CellObject::Slot(slot).to_bytes())).unwrap();
    source
}

fn map(token: PendId, team: TeamId, region: Region) {
    let (object, address) = memo(
        CellSource::Pend {
            token,
            offset: 0,
            count: region.pages,
        },
        team,
        Placement::exact(VirtAddr::new(region.va), PAGE_SIZE).unwrap(),
        region.flags,
    );
    assert_eq!(address.get(), region.va);
    publish(object, region.pages);
}

fn delegate(source: PendId, task: TaskId, operations: Operations, modifiable: Modifiable) {
    let request = AccordRequest {
        src: source,
        target: AccordTarget::Task(task),
        mark: Mark::NONE,
        limits: Limits {
            operations,
            modifiable,
            distributable: Distributable::No,
            range: PendRange::Inherit,
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
    let initrd_token = PendId::mint(entries.find(Name::Page(Page::Initrd)).unwrap().handle());
    let (base, size) = share_view::open(initrd_token).unwrap();
    let bytes = unsafe { slice::from_raw_parts(base.get() as *const u8, size) };
    let loader = initrd::entry_image(bytes).unwrap();
    let offset = loader.as_ptr().addr() - bytes.as_ptr().addr() + loader.len();
    let word = |at: usize| usize::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let initiator = TaskId::new(word(offset));
    let expected = (word(offset + 8) as u64, word(offset + 16) as u64);
    let program = &bytes[offset + 32..offset + 32 + word(offset + 24)];

    let spec = UnitCreate::Team {
        kind: ProgramKind::User,
        guard: ControlId::NONE,
        nest: None,
        member_exit: true,
    }
    .to_bytes();
    let UnitTarget::Team(team) = unit::create(at(&spec)).unwrap() else {
        panic!("expected Team")
    };
    let mut writable = None;
    for (image, is_program) in [(loader, false), (program, true)] {
        let image_regions = Capsule::parse(image).unwrap();
        for index in 0..image_regions.count {
            let region = image_regions.region(index).unwrap();
            let writes = region.flags & 4 != 0;
            if writes && !is_program {
                continue;
            }
            let data = &image[region.payload..region.payload + region.data_pages * PAGE_SIZE];
            let source = stage(
                license,
                data,
                region.pages,
                if writes {
                    Modifiable::Yes
                } else {
                    Modifiable::No
                },
            );
            if writes {
                assert!(
                    writable.replace((source, region)).is_none(),
                    "one writable segment"
                );
            } else {
                map(source, team, region);
            }
        }
    }
    let (data, region) = writable.unwrap();
    let entry = Capsule::parse(program).unwrap().entry;
    let child_args = [entry, region.va, region.pages];
    let child = UnitCreate::Task {
        team: Some(team),
        entry: VirtAddr::new(child_entry as *const () as usize),
        args: at(&child_args),
        count: child_args.len() as u64,
        stack: VirtAddr::new(0),
    }
    .to_bytes();
    let UnitTarget::Task(task) = unit::create(at(&child)).unwrap() else {
        panic!("expected Task")
    };
    delegate(
        data,
        task,
        Operations::Share(ShareRights::ASSOCIATE),
        Modifiable::Yes,
    );
    delegate(
        license,
        task,
        Operations::None(CreationRights::FLAT | CreationRights::SIGN),
        Modifiable::No,
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

#[unsafe(no_mangle)]
unsafe extern "C" fn enter(words: *const usize, count: usize) -> ! {
    assert_eq!(count, 3);
    let words = unsafe { slice::from_raw_parts(words, count) };
    let mut after = PendId::NONE;
    let source = loop {
        let mut info = [0u8; PendInfo::SIZE];
        assert_eq!(
            pend::collect(after, VirtAddr::new(info.as_mut_ptr() as usize), 1).unwrap(),
            1
        );
        let info = PendInfo::from_bytes(&info).unwrap();
        if info.limits.operations == Operations::Share(ShareRights::ASSOCIATE) {
            break info.token;
        }
        after = info.token;
    };
    map(
        source,
        OWN,
        Region {
            va: words[1],
            pages: words[2],
            data_pages: 0,
            payload: 0,
            flags: 6,
        },
    );
    let start: extern "C" fn(usize, usize) -> ! = unsafe { core::mem::transmute(words[0]) };
    start(0, 0)
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
