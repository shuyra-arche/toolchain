//! Only the process broker holds these mapping controls and private sources.
use super::{address, room};
use kernel_api::{
    abi::call::cell::CellFail,
    address::{PAGE_SIZE, VirtAddr},
    model::{
        cell::{
            Allocation, CellAssociation, CellCreate, CellInfo, CellObject, CellPosition,
            CellSource, CellState, CreatedCell, MemoDestination, Placement, ReferenceRange,
        },
        identity::{PendId, TeamId},
    },
};
use kernel_client::call::cell;

const TEAM: TeamId = TeamId::new(0);
struct Header {
    memo: CellObject,
    count: usize,
}

pub(super) fn allocate(license: PendId, size: usize, align: usize) -> usize {
    let align = align.max(align_of::<Header>());
    let Some(prefix) = size_of::<Header>().checked_next_multiple_of(align) else {
        return 0;
    };
    let Some(bytes) = prefix
        .checked_add(size)
        .and_then(|n| n.checked_next_multiple_of(PAGE_SIZE))
        .filter(|&n| n <= isize::MAX as usize)
    else {
        return 0;
    };
    let count = bytes / PAGE_SIZE;
    let Some(allocation) = Allocation::new(bytes, PAGE_SIZE) else {
        return 0;
    };
    let Some(placement) = Placement::lowest(align.max(PAGE_SIZE)) else {
        return 0;
    };

    let Ok(CreatedCell::Slot(slot)) =
        cell::create(license, address(&CellCreate::Slot(allocation).to_bytes()))
    else {
        return 0;
    };
    let descriptor = CellCreate::Memo {
        source: CellSource::Slot(ReferenceRange {
            reference: slot,
            offset: 0,
            count,
        }),
        destination: MemoDestination::Internal {
            team: TEAM,
            placement,
            access: 6,
        },
    }
    .to_bytes();
    let (memo, base) = loop {
        match cell::create(PendId::NONE, address(&descriptor)) {
            Ok(CreatedCell::Memo { token, address }) => {
                break (CellObject::Memo(token), address);
            }
            Err(e) if e.source == CellFail::Busy => room::yield_(),
            _ => {
                delete(CellObject::Slot(slot));
                return 0;
            }
        }
    };
    for page in 0..count {
        if publish(memo, page, CellObject::Slot(slot)).is_err() {
            retire(memo, page + 1);
            delete(CellObject::Slot(slot));
            return 0;
        }
    }
    delete(CellObject::Slot(slot));
    let data = base.get() + prefix;
    unsafe {
        ((data - size_of::<Header>()) as *mut Header).write(Header { memo, count });
    }
    data
}

fn inspect(object: CellObject, page: usize) -> Result<(CellState, CellPosition), CellFail> {
    let mut pos = CellPosition {
        object,
        page,
        generation: 0,
    };
    let mut bytes = [0u8; CellInfo::WIDTH];
    loop {
        match cell::inspect(
            address(&pos.to_bytes()),
            VirtAddr::new(bytes.as_mut_ptr() as usize),
        ) {
            Ok(()) => {
                let info = CellInfo::from_bytes(&bytes).ok_or(CellFail::Denied)?;
                pos.generation = info.generation;
                return Ok((info.state, pos));
            }
            Err(e) if e.source == CellFail::Busy => room::yield_(),
            Err(e) => return Err(e.source),
        }
    }
}

fn publish(memo: CellObject, page: usize, slot: CellObject) -> Result<(), CellFail> {
    loop {
        let (state, target) = inspect(memo, page)?;
        let result = match state {
            CellState::Vacant => cell::insert(address(
                &CellAssociation {
                    source: CellPosition {
                        object: slot,
                        page,
                        generation: 0,
                    },
                    target,
                }
                .to_bytes(),
            )),
            CellState::Ready => cell::publish(address(&target.to_bytes())),
            CellState::Published => return Ok(()),
            CellState::Publishing | CellState::Retracting => {
                room::yield_();
                continue;
            }
            CellState::Failed => return Err(CellFail::Denied),
        };
        match result {
            Ok(()) => {}
            Err(e) if e.source == CellFail::Busy => room::yield_(),
            Err(e) => return Err(e.source),
        }
    }
}

fn delete(object: CellObject) {
    let descriptor = object.to_bytes();
    loop {
        match cell::delete(address(&descriptor)) {
            Ok(()) => return,
            Err(e) if e.source == CellFail::Busy => room::yield_(),
            Err(_) => super::super::abort_internal(),
        }
    }
}
fn retire(memo: CellObject, count: usize) {
    for page in 0..count {
        loop {
            match inspect(memo, page) {
                Ok((CellState::Published, pos)) => match cell::retract(address(&pos.to_bytes())) {
                    Ok(()) => break,
                    Err(e) if e.source == CellFail::Busy => room::yield_(),
                    Err(_) => super::super::abort_internal(),
                },
                Ok((CellState::Publishing | CellState::Retracting, _)) => room::yield_(),
                Ok(_) => break,
                Err(_) => super::super::abort_internal(),
            }
        }
    }
    delete(memo);
}
pub(super) fn free(data: usize) {
    // Copy the header before withdrawing the mapping that contains it.
    let Header { memo, count } = unsafe { ((data - size_of::<Header>()) as *const Header).read() };
    retire(memo, count);
}
