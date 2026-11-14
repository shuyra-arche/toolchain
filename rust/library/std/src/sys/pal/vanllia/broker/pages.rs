//! The broker retains Flat holding and Reservation authority until every
//! publication is retracted, every association removed and release has settled.
use super::{address, memory::Memory, room};
use kernel_api::{
    abi::memory::{Action, Failure, Query, Snapshot, Source, Value},
    address::{PAGE_SIZE, PageCount},
    model::{
        cell::{CellLevel, Placement},
        flat::SlotPosition,
        identity::PendId,
        nest::{MemoPhase, MemoPosition, NestId},
        pend::{PendInfo, Target},
    },
};
use kernel_client::call::pend;

pub(super) struct Pages {
    memory: Memory,
    license: PendId,
    space: PendId,
    nest: NestId,
}
struct Header {
    flat: SlotPosition,
    memo: MemoPosition,
    count: usize,
}

impl Pages {
    pub(super) fn new(license: PendId, space: PendId) -> Self {
        let mut bytes = [0; PendInfo::SIZE];
        pend::inspect(
            space,
            kernel_api::address::VirtAddr::new(bytes.as_mut_ptr() as usize),
        )
        .unwrap_or_else(|_| super::super::abort_internal());
        let Some(PendInfo {
            target: Target::Nest(nest),
            ..
        }) = PendInfo::from_bytes(&bytes)
        else {
            super::super::abort_internal();
        };
        Self {
            memory: Memory::new(super::super::abort_memory),
            license,
            space,
            nest,
        }
    }

    // Only rejected operations may get a new identity. Busy/Stale snapshots
    // are refreshed; accepted operations settle inside Memory::mutate.
    fn change(&mut self, action: impl Fn(&Memory) -> Action) -> Result<Value, Failure> {
        let deadline = room::clock().saturating_add(5_000_000_000);
        loop {
            match self.memory.mutate(action(&self.memory)) {
                Err(Failure::Busy | Failure::Stale) if room::clock() < deadline => room::yield_(),
                result => return result,
            }
        }
    }

    pub(super) fn allocate(&mut self, size: usize, align: usize) -> usize {
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
        let Some(placement) = Placement::lowest(align.max(PAGE_SIZE)) else {
            return 0;
        };
        let license = self.license;
        let Ok(Value::CreatedFlat { token, flat, .. }) = self.change(|_| Action::CreateFlat {
            license,
            count,
            level: CellLevel::new(0).unwrap(),
        }) else {
            return 0;
        };
        let flat = SlotPosition::new(token, flat, 0).unwrap();
        let space = self.space;
        let nest = self.nest;
        let result = self.change(|memory| {
            let Snapshot::Nest(status) = memory.inspect(Query::Nest { token: space, nest }) else {
                super::super::abort_internal();
            };
            Action::Reserve {
                token: space,
                nest,
                expected: status.version,
                reservation: kernel_api::model::nest::ReserveRequest {
                    placement,
                    count: PageCount::new(count).unwrap(),
                    level: CellLevel::new(0).unwrap(),
                    access: 6,
                },
            }
        });
        let Ok(Value::Reservation {
            token,
            reservation,
            address: base,
            ..
        }) = result
        else {
            self.delete(flat);
            return 0;
        };
        let memo = MemoPosition::new(token, nest, reservation, 0).unwrap();
        for page in 0..count {
            let slot = SlotPosition::new(flat.token(), flat.flat(), page).unwrap();
            let target = MemoPosition::new(token, nest, reservation, page).unwrap();
            let occupied = self.change(|memory| {
                let Snapshot::Slot(status) = memory.inspect(Query::Slot(slot)) else {
                    super::super::abort_internal()
                };
                Action::Occupy {
                    slot,
                    expected: status.version,
                }
            });
            let inserted = occupied.and_then(|_| {
                self.change(|memory| {
                    let Snapshot::Slot(source) = memory.inspect(Query::Slot(slot)) else {
                        super::super::abort_internal()
                    };
                    let Snapshot::Memo(status) = memory.inspect(Query::Memo(target)) else {
                        super::super::abort_internal()
                    };
                    Action::Insert {
                        target,
                        expected: status.generation,
                        source: Source::Flat {
                            position: slot,
                            expected: source.version,
                        },
                    }
                })
            });
            let published = inserted.and_then(|_| {
                self.change(|memory| {
                    let Snapshot::Memo(status) = memory.inspect(Query::Memo(target)) else {
                        super::super::abort_internal()
                    };
                    Action::Publish {
                        memo: target,
                        expected: status.generation,
                        maintenance: None,
                    }
                })
            });
            if published.is_err() {
                self.retire(memo, count);
                self.delete(flat);
                return 0;
            }
        }
        let data = base.get() + prefix;
        unsafe {
            ((data - size_of::<Header>()) as *mut Header).write(Header { flat, memo, count });
        }
        data
    }

    fn delete(&mut self, flat: SlotPosition) {
        self.change(|memory| {
            let Snapshot::Flat(status) = memory.inspect(Query::Flat {
                token: flat.token(),
                flat: flat.flat(),
            }) else {
                super::super::abort_internal()
            };
            Action::DeleteFlat {
                token: flat.token(),
                flat: flat.flat(),
                expected: status.version,
            }
        })
        .unwrap_or_else(|_| super::super::abort_internal());
    }

    fn retire(&mut self, first: MemoPosition, count: usize) {
        for page in 0..count {
            let memo =
                MemoPosition::new(first.token(), first.nest(), first.reservation(), page).unwrap();
            let Snapshot::Memo(status) = self.memory.inspect(Query::Memo(memo)) else {
                super::super::abort_internal()
            };
            if status.phase == MemoPhase::Published {
                // Poll the exact accepted Retract, never reconstruct it from
                // an intermediate generation or infer retirement from a wake.
                self.change(|memory| {
                    let Snapshot::Memo(status) = memory.inspect(Query::Memo(memo)) else {
                        super::super::abort_internal()
                    };
                    Action::Retract {
                        memo,
                        expected: status.generation,
                    }
                })
                .unwrap_or_else(|_| super::super::abort_internal());
            }
            let Snapshot::Memo(status) = self.memory.inspect(Query::Memo(memo)) else {
                super::super::abort_internal()
            };
            if status.phase == MemoPhase::Associated {
                self.change(|memory| {
                    let Snapshot::Memo(status) = memory.inspect(Query::Memo(memo)) else {
                        super::super::abort_internal()
                    };
                    Action::Remove {
                        memo,
                        expected: status.generation,
                    }
                })
                .unwrap_or_else(|_| super::super::abort_internal());
            } else if status.phase != MemoPhase::Empty {
                super::super::abort_internal();
            }
        }
        self.change(|memory| {
            let Snapshot::Reservation(status) = memory.inspect(Query::Reservation {
                token: first.token(),
                nest: first.nest(),
                reservation: first.reservation(),
            }) else {
                super::super::abort_internal()
            };
            Action::Release {
                token: first.token(),
                nest: first.nest(),
                reservation: first.reservation(),
                expected: status.version,
            }
        })
        .unwrap_or_else(|_| super::super::abort_internal());
    }

    pub(super) fn free(&mut self, data: usize) {
        // Copy every retained authority before withdrawing its containing mapping.
        let Header { flat, memo, count } =
            unsafe { ((data - size_of::<Header>()) as *const Header).read() };
        self.retire(memo, count);
        self.delete(flat);
    }
}
