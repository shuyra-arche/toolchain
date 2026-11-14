//! One native-memory request sequence for the broker Task. Accepted work is
//! settled by its original identity; a delivery failure never causes a replay.
use kernel_api::abi::memory::{
    Action, Call, Failure, Outcome, Query, Request, RequestId, Snapshot, Value,
};
use kernel_client::{call::room, memory::Response};

pub(crate) struct Memory {
    next: u64,
    abort: fn(&'static str) -> !,
}

impl Memory {
    // Construct once, before this Task issues its first native memory request.
    // The sequence cannot be shared with another instance or reset at an ELF entry.
    pub(crate) const fn new(abort: fn(&'static str) -> !) -> Self {
        Self { next: 1, abort }
    }

    fn invoke(
        &self,
        action: Action,
        id: Option<RequestId>,
    ) -> Result<Response, kernel_client::memory::Error> {
        match Call::from_request(
            Request::new(id, action).unwrap_or_else(|| (self.abort)("invalid memory descriptor")),
        ) {
            Call::Flat(call) => kernel_client::memory::flat::invoke(call),
            Call::Nest(call) => kernel_client::memory::nest::invoke(call),
        }
    }

    pub(crate) fn inspect(&self, query: Query) -> Snapshot {
        let deadline = room::clock().saturating_add(5_000_000_000);
        loop {
            if room::clock() >= deadline {
                (self.abort)("memory observation timed out");
            }
            match self.invoke(Action::Inspect(query), None) {
                Ok(Response::Observation(value)) => return value.snapshot(),
                Err(kernel_client::memory::Error::Delivery(Failure::Busy)) => room::yield_(),
                _ => (self.abort)("native memory observation failed"),
            }
        }
    }

    pub(crate) fn mutate(&mut self, action: Action) -> Result<Value, Failure> {
        let id = RequestId::new(self.next)
            .unwrap_or_else(|| (self.abort)("memory request sequence exhausted"));
        self.next = self
            .next
            .checked_add(1)
            .unwrap_or_else(|| (self.abort)("memory request sequence exhausted"));
        let original = Request::new(Some(id), action)
            .unwrap_or_else(|| (self.abort)("invalid memory mutation"));
        let mut response = self.invoke(action, Some(id));
        let deadline = room::clock().saturating_add(5_000_000_000);
        loop {
            if let Ok(Response::Reply(reply)) = response {
                if !reply.matches(original) {
                    (self.abort)("uncorrelated memory result");
                }
                match reply.outcome() {
                    Outcome::Committed(value) => return Ok(value),
                    Outcome::Rejected(failure) => return Err(failure),
                    Outcome::Pending(_) => {}
                    // Failed may retain hardware obligations. Team termination
                    // leaves their reclamation with Kernel rather than reusing them.
                    Outcome::Failed { .. } | Outcome::Expired => {
                        (self.abort)("memory result unsettled")
                    }
                }
            }
            if room::clock() >= deadline {
                (self.abort)("memory result timed out");
            }
            room::yield_();
            response = self.invoke(
                Action::InspectResult {
                    domain: action.opcode().domain(),
                    request: id,
                },
                None,
            );
        }
    }
}
