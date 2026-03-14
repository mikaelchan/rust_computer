//! Round-robin arbitration between the CPU and autonomous bus masters.

use std::{cell::RefCell, fmt, rc::Rc};

use crate::{
    BurstBus, BurstPhase, BurstRequest, BurstResponse, Bus, BusError, BusMaster, BusMasterRequest,
    BusMasterResponse, InterruptSet, TransactionBus, TransactionRequest, TransactionResponse,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArbiterStats {
    pub master_grants: u64,
    pub cpu_stall_cycles: u64,
}

struct MasterSlot {
    name: &'static str,
    master: Box<dyn BusMaster>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRequest {
    master_index: usize,
    request: BusMasterRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InFlightRequest {
    master_index: usize,
    request: BusMasterRequest,
    transaction_id: u64,
}

struct SharedBusMaster<M> {
    inner: Rc<RefCell<M>>,
}

impl<M> BusMaster for SharedBusMaster<M>
where
    M: BusMaster + 'static,
{
    fn name(&self) -> &'static str {
        self.inner.borrow().name()
    }

    fn request(&mut self) -> Option<BusMasterRequest> {
        self.inner.borrow_mut().request()
    }

    fn on_response(&mut self, response: Result<BusMasterResponse, BusError>) {
        self.inner.borrow_mut().on_response(response);
    }
}

/// A shared bus wrapper that grants lower-bus access to one master at a time.
pub struct ArbiterBus<B> {
    inner: B,
    masters: Vec<MasterSlot>,
    next_master_index: usize,
    pending_request: Option<PendingRequest>,
    in_flight_requests: Vec<InFlightRequest>,
    cpu_reserved_this_cycle: bool,
    stats: ArbiterStats,
}

impl<B> ArbiterBus<B>
where
    B: TransactionBus,
{
    #[must_use]
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            masters: Vec::new(),
            next_master_index: 0,
            pending_request: None,
            in_flight_requests: Vec::new(),
            cpu_reserved_this_cycle: false,
            stats: ArbiterStats::default(),
        }
    }

    pub fn add_master<M>(&mut self, master: M)
    where
        M: BusMaster + 'static,
    {
        let name = master.name();
        self.masters.push(MasterSlot {
            name,
            master: Box::new(master),
        });
    }

    pub fn add_shared_master<M>(&mut self, master: Rc<RefCell<M>>)
    where
        M: BusMaster + 'static,
    {
        let name = master.borrow().name();
        self.masters.push(MasterSlot {
            name,
            master: Box::new(SharedBusMaster { inner: master }),
        });
    }

    #[must_use]
    pub fn inner(&self) -> &B {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }

    #[must_use]
    pub const fn stats(&self) -> ArbiterStats {
        self.stats
    }

    fn request_to_transaction(request: &BusMasterRequest) -> TransactionRequest {
        match *request {
            BusMasterRequest::Load32 { addr } => TransactionRequest::load(addr, 4),
            BusMasterRequest::Store32 { addr, value } => {
                TransactionRequest::store(addr, 4, value.to_le_bytes())
            }
        }
    }

    fn request_addr(request: &BusMasterRequest) -> crate::Address {
        match *request {
            BusMasterRequest::Load32 { addr } | BusMasterRequest::Store32 { addr, .. } => addr,
        }
    }

    fn response_from_transaction(
        request: &BusMasterRequest,
        response: TransactionResponse,
    ) -> Result<BusMasterResponse, BusError> {
        match (request, response) {
            (BusMasterRequest::Load32 { .. }, TransactionResponse::Read { data, width: 4 }) => {
                Ok(BusMasterResponse::Load32(u32::from_le_bytes(data)))
            }
            (BusMasterRequest::Store32 { .. }, TransactionResponse::WriteComplete) => {
                Ok(BusMasterResponse::StoreComplete)
            }
            _ => Err(BusError::DeviceFault {
                addr: Self::request_addr(request),
                message: "arbiter observed mismatched master transaction response".to_string(),
            }),
        }
    }

    fn master_has_outstanding_request(&self, master_index: usize) -> bool {
        self.pending_request
            .as_ref()
            .is_some_and(|pending| pending.master_index == master_index)
            || self
                .in_flight_requests
                .iter()
                .any(|active| active.master_index == master_index)
    }

    fn try_submit_pending_request(&mut self) {
        let Some(pending) = self.pending_request.as_ref().cloned() else {
            return;
        };

        match self
            .inner
            .submit_transaction(Self::request_to_transaction(&pending.request))
        {
            Ok(transaction_id) => {
                self.stats.master_grants += 1;
                self.in_flight_requests.push(InFlightRequest {
                    master_index: pending.master_index,
                    request: pending.request,
                    transaction_id,
                });
                self.pending_request = None;
            }
            Err(BusError::Busy { .. }) => {}
            Err(error) => {
                self.masters[pending.master_index]
                    .master
                    .on_response(Err(error));
                self.pending_request = None;
            }
        }
    }

    fn poll_in_flight_requests(&mut self) {
        let mut index = 0;
        while index < self.in_flight_requests.len() {
            let active = &self.in_flight_requests[index];
            let Some(result) = self.inner.take_transaction_response(active.transaction_id) else {
                index += 1;
                continue;
            };

            let active = self.in_flight_requests.swap_remove(index);
            let response = result
                .and_then(|response| Self::response_from_transaction(&active.request, response));
            self.masters[active.master_index]
                .master
                .on_response(response);
        }
    }

    fn select_next_request(&mut self) -> Option<PendingRequest> {
        if self.masters.is_empty() {
            return None;
        }

        for offset in 0..self.masters.len() {
            let index = (self.next_master_index + offset) % self.masters.len();
            if self.master_has_outstanding_request(index) {
                continue;
            }
            if let Some(request) = self.masters[index].master.request() {
                self.next_master_index = (index + 1) % self.masters.len();
                return Some(PendingRequest {
                    master_index: index,
                    request,
                });
            }
        }

        None
    }

    fn ensure_cpu_slot(&mut self) -> Result<(), BusError> {
        if self.cpu_reserved_this_cycle {
            self.stats.cpu_stall_cycles += 1;
            return Err(BusError::Busy {
                remaining_cycles: 1,
            });
        }

        Ok(())
    }
}

impl<B> Bus for ArbiterBus<B>
where
    B: TransactionBus,
{
    fn reset(&mut self) {
        self.next_master_index = 0;
        self.pending_request = None;
        self.in_flight_requests.clear();
        self.cpu_reserved_this_cycle = false;
        self.stats = ArbiterStats::default();
        self.inner.reset();
    }

    fn fetch32(&mut self, addr: crate::Address) -> Result<u32, BusError> {
        self.ensure_cpu_slot()?;
        self.inner.fetch32(addr)
    }

    fn load8(&mut self, addr: crate::Address) -> Result<u8, BusError> {
        self.ensure_cpu_slot()?;
        self.inner.load8(addr)
    }

    fn store8(&mut self, addr: crate::Address, value: u8) -> Result<(), BusError> {
        self.ensure_cpu_slot()?;
        self.inner.store8(addr, value)
    }

    fn load16(&mut self, addr: crate::Address) -> Result<u16, BusError> {
        self.ensure_cpu_slot()?;
        self.inner.load16(addr)
    }

    fn load32(&mut self, addr: crate::Address) -> Result<u32, BusError> {
        self.ensure_cpu_slot()?;
        self.inner.load32(addr)
    }

    fn store16(&mut self, addr: crate::Address, value: u16) -> Result<(), BusError> {
        self.ensure_cpu_slot()?;
        self.inner.store16(addr, value)
    }

    fn store32(&mut self, addr: crate::Address, value: u32) -> Result<(), BusError> {
        self.ensure_cpu_slot()?;
        self.inner.store32(addr, value)
    }

    fn tick(&mut self) {
        self.inner.tick();
        self.cpu_reserved_this_cycle = false;
        self.poll_in_flight_requests();

        if self.pending_request.is_some() {
            self.cpu_reserved_this_cycle = true;
            self.try_submit_pending_request();
            return;
        }

        if let Some(pending) = self.select_next_request() {
            self.cpu_reserved_this_cycle = true;
            self.pending_request = Some(pending);
            self.try_submit_pending_request();
        }
    }

    fn is_busy(&self) -> bool {
        self.cpu_reserved_this_cycle
            || self.pending_request.is_some()
            || !self.in_flight_requests.is_empty()
            || self.inner.is_busy()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.pending_interrupts()
    }
}

impl<B> BurstBus for ArbiterBus<B>
where
    B: TransactionBus + BurstBus,
{
    fn submit_burst(&mut self, request: BurstRequest) -> Result<u64, BusError> {
        self.ensure_cpu_slot()?;
        self.inner.submit_burst(request)
    }

    fn burst_phase(&self, id: u64) -> Option<BurstPhase> {
        self.inner.burst_phase(id)
    }

    fn advance_burst(&mut self, id: u64) -> Option<BurstPhase> {
        self.inner.advance_burst(id)
    }

    fn take_burst_response(&mut self, id: u64) -> Option<Result<BurstResponse, BusError>> {
        self.inner.take_burst_response(id)
    }
}

impl<B> fmt::Debug for ArbiterBus<B>
where
    B: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let master_names: Vec<_> = self.masters.iter().map(|slot| slot.name).collect();
        f.debug_struct("ArbiterBus")
            .field("inner", &self.inner)
            .field("masters", &master_names)
            .field("next_master_index", &self.next_master_index)
            .field("pending_request", &self.pending_request)
            .field("in_flight_requests", &self.in_flight_requests)
            .field("cpu_reserved_this_cycle", &self.cpu_reserved_this_cycle)
            .field("stats", &self.stats)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    use crate::{
        Bus, BusMaster, BusMasterRequest, BusMasterResponse, TransactionBus, TransactionPhase,
        TransactionRequest, TransactionResponse,
    };

    use super::ArbiterBus;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ActiveTransaction {
        id: u64,
        request: TransactionRequest,
        phase: TransactionPhase,
    }

    #[derive(Debug)]
    struct TinyBus {
        data: [u8; 32],
        transaction_latency: u32,
        active_transaction: Option<ActiveTransaction>,
        next_transaction_id: u64,
    }

    impl Default for TinyBus {
        fn default() -> Self {
            Self {
                data: [0; 32],
                transaction_latency: 0,
                active_transaction: None,
                next_transaction_id: 0,
            }
        }
    }

    impl TinyBus {
        fn with_transaction_latency(transaction_latency: u32) -> Self {
            Self {
                transaction_latency,
                ..Self::default()
            }
        }

        fn execute_transaction(
            &mut self,
            request: TransactionRequest,
        ) -> Result<TransactionResponse, crate::BusError> {
            match request {
                TransactionRequest {
                    kind: crate::AccessKind::Load,
                    addr,
                    width: 4,
                    ..
                }
                | TransactionRequest {
                    kind: crate::AccessKind::Fetch,
                    addr,
                    width: 4,
                    ..
                } => Ok(TransactionResponse::Read {
                    data: [
                        self.data[addr as usize],
                        self.data[addr as usize + 1],
                        self.data[addr as usize + 2],
                        self.data[addr as usize + 3],
                    ],
                    width: 4,
                }),
                TransactionRequest {
                    kind: crate::AccessKind::Store,
                    addr,
                    width: 4,
                    write_data,
                } => {
                    self.data[addr as usize..addr as usize + 4].copy_from_slice(&write_data);
                    Ok(TransactionResponse::WriteComplete)
                }
                TransactionRequest { addr, .. } => Err(crate::BusError::DeviceFault {
                    addr,
                    message: "tiny bus only supports 32-bit transactions".to_string(),
                }),
            }
        }
    }

    impl Bus for TinyBus {
        fn load8(&mut self, addr: crate::Address) -> Result<u8, crate::BusError> {
            Ok(self.data[addr as usize])
        }

        fn store8(&mut self, addr: crate::Address, value: u8) -> Result<(), crate::BusError> {
            self.data[addr as usize] = value;
            Ok(())
        }

        fn reset(&mut self) {
            self.data = [0; 32];
            self.active_transaction = None;
            self.next_transaction_id = 0;
        }

        fn tick(&mut self) {
            let Some((request, phase)) = self
                .active_transaction
                .as_ref()
                .map(|active| (active.request, active.phase.clone()))
            else {
                return;
            };

            let next_phase = match phase {
                TransactionPhase::Accepted => {
                    if self.transaction_latency == 0 {
                        match self.execute_transaction(request) {
                            Ok(response) => TransactionPhase::Ready(response),
                            Err(error) => TransactionPhase::Failed(error),
                        }
                    } else {
                        TransactionPhase::InFlight {
                            remaining_cycles: self.transaction_latency,
                        }
                    }
                }
                TransactionPhase::InFlight { remaining_cycles } => {
                    if remaining_cycles > 1 {
                        TransactionPhase::InFlight {
                            remaining_cycles: remaining_cycles - 1,
                        }
                    } else {
                        match self.execute_transaction(request) {
                            Ok(response) => TransactionPhase::Ready(response),
                            Err(error) => TransactionPhase::Failed(error),
                        }
                    }
                }
                terminal => terminal,
            };

            if let Some(active) = self.active_transaction.as_mut() {
                active.phase = next_phase;
            }
        }

        fn is_busy(&self) -> bool {
            self.active_transaction.is_some()
        }
    }

    impl TransactionBus for TinyBus {
        fn submit_transaction(
            &mut self,
            request: TransactionRequest,
        ) -> Result<u64, crate::BusError> {
            if self.active_transaction.is_some() {
                return Err(crate::BusError::Busy {
                    remaining_cycles: 1,
                });
            }

            let id = self.next_transaction_id;
            self.next_transaction_id = self.next_transaction_id.wrapping_add(1);
            self.active_transaction = Some(ActiveTransaction {
                id,
                request,
                phase: TransactionPhase::Accepted,
            });
            Ok(id)
        }

        fn transaction_phase(&self, id: u64) -> Option<TransactionPhase> {
            self.active_transaction
                .as_ref()
                .filter(|active| active.id == id)
                .map(|active| active.phase.clone())
        }

        fn advance_transaction(&mut self, id: u64) -> Option<TransactionPhase> {
            if self
                .active_transaction
                .as_ref()
                .is_none_or(|active| active.id != id)
            {
                return None;
            }

            self.tick();
            self.transaction_phase(id)
        }

        fn take_transaction_response(
            &mut self,
            id: u64,
        ) -> Option<Result<TransactionResponse, crate::BusError>> {
            let active = self.active_transaction.as_ref()?;
            if active.id != id {
                return None;
            }

            match active.phase.clone() {
                TransactionPhase::Accepted | TransactionPhase::InFlight { .. } => None,
                TransactionPhase::Ready(response) => {
                    self.active_transaction = None;
                    Some(Ok(response))
                }
                TransactionPhase::Failed(error) => {
                    self.active_transaction = None;
                    Some(Err(error))
                }
            }
        }
    }

    #[derive(Debug, Default)]
    struct ScriptedMaster {
        name: &'static str,
        requests: VecDeque<BusMasterRequest>,
        completions: Vec<BusMasterResponse>,
    }

    impl ScriptedMaster {
        fn with_requests(
            name: &'static str,
            requests: impl IntoIterator<Item = BusMasterRequest>,
        ) -> Self {
            Self {
                name,
                requests: requests.into_iter().collect(),
                completions: Vec::new(),
            }
        }
    }

    impl BusMaster for ScriptedMaster {
        fn name(&self) -> &'static str {
            self.name
        }

        fn request(&mut self) -> Option<BusMasterRequest> {
            self.requests.pop_front()
        }

        fn on_response(&mut self, response: Result<BusMasterResponse, crate::BusError>) {
            self.completions
                .push(response.expect("scripted master request should succeed"));
        }
    }

    #[test]
    fn round_robin_grants_pending_masters() {
        let first = Rc::new(RefCell::new(ScriptedMaster::with_requests(
            "first",
            [BusMasterRequest::Store32 {
                addr: 0,
                value: 0x1122_3344,
            }],
        )));
        let second = Rc::new(RefCell::new(ScriptedMaster::with_requests(
            "second",
            [BusMasterRequest::Store32 {
                addr: 4,
                value: 0x5566_7788,
            }],
        )));

        let mut arbiter = ArbiterBus::new(TinyBus::default());
        arbiter.add_shared_master(Rc::clone(&first));
        arbiter.add_shared_master(Rc::clone(&second));

        arbiter.tick();
        arbiter.tick();
        arbiter.tick();

        assert_eq!(
            arbiter.load32(0).expect("first store should land"),
            0x1122_3344
        );
        assert_eq!(
            arbiter.load32(4).expect("second store should land"),
            0x5566_7788
        );
        assert_eq!(
            first.borrow().completions,
            vec![BusMasterResponse::StoreComplete]
        );
        assert_eq!(
            second.borrow().completions,
            vec![BusMasterResponse::StoreComplete]
        );
        assert_eq!(arbiter.stats().master_grants, 2);
    }

    #[test]
    fn cpu_access_stalls_when_master_claims_cycle() {
        let master = Rc::new(RefCell::new(ScriptedMaster::with_requests(
            "writer",
            [BusMasterRequest::Store32 {
                addr: 0,
                value: 0xdead_beef,
            }],
        )));
        let mut arbiter = ArbiterBus::new(TinyBus::default());
        arbiter.add_shared_master(Rc::clone(&master));

        arbiter.tick();

        let error = arbiter
            .load32(0)
            .expect_err("CPU should wait for a granted master cycle");
        assert_eq!(
            error,
            crate::BusError::Busy {
                remaining_cycles: 1
            }
        );

        arbiter.tick();
        assert_eq!(
            arbiter.load32(0).expect("CPU should proceed next cycle"),
            0xdead_beef
        );
        assert_eq!(arbiter.stats().cpu_stall_cycles, 1);
    }

    #[test]
    fn cpu_can_use_following_cycles_while_master_transaction_waits_in_fabric() {
        let master = Rc::new(RefCell::new(ScriptedMaster::with_requests(
            "writer",
            [BusMasterRequest::Store32 {
                addr: 0,
                value: 0xdead_beef,
            }],
        )));
        let mut arbiter = ArbiterBus::new(TinyBus::with_transaction_latency(2));
        arbiter.add_shared_master(Rc::clone(&master));

        arbiter.tick();
        assert_eq!(
            arbiter
                .load32(4)
                .expect_err("CPU should stall in the grant cycle"),
            crate::BusError::Busy {
                remaining_cycles: 1,
            }
        );

        arbiter.tick();
        assert_eq!(
            arbiter
                .load32(4)
                .expect("CPU should use cycles after submission"),
            0
        );
        assert_eq!(arbiter.stats().cpu_stall_cycles, 1);

        arbiter.tick();
        arbiter.tick();
        assert_eq!(
            arbiter.load32(0).expect("master store should complete"),
            0xdead_beef
        );
    }
}
