//! Round-robin arbitration between the CPU and autonomous bus masters.

use std::{cell::RefCell, fmt, rc::Rc};

use crate::{Bus, BusError, BusMaster, BusMasterRequest, BusMasterResponse, InterruptSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArbiterStats {
    pub master_grants: u64,
    pub cpu_stall_cycles: u64,
}

struct MasterSlot {
    name: &'static str,
    master: Box<dyn BusMaster>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveRequest {
    master_index: usize,
    request: BusMasterRequest,
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
    active_request: Option<ActiveRequest>,
    cpu_reserved_this_cycle: bool,
    stats: ArbiterStats,
}

impl<B> ArbiterBus<B>
where
    B: Bus,
{
    #[must_use]
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            masters: Vec::new(),
            next_master_index: 0,
            active_request: None,
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

    fn dispatch_request(&mut self, active: ActiveRequest) {
        let result = match active.request {
            BusMasterRequest::Load32 { addr } => {
                self.inner.load32(addr).map(BusMasterResponse::Load32)
            }
            BusMasterRequest::Store32 { addr, value } => self
                .inner
                .store32(addr, value)
                .map(|()| BusMasterResponse::StoreComplete),
        };

        match result {
            Ok(response) => {
                self.masters[active.master_index]
                    .master
                    .on_response(Ok(response));
                self.active_request = None;
            }
            Err(BusError::Busy { .. }) => {}
            Err(error) => {
                self.masters[active.master_index]
                    .master
                    .on_response(Err(error));
                self.active_request = None;
            }
        }
    }

    fn select_next_request(&mut self) -> Option<ActiveRequest> {
        if self.masters.is_empty() {
            return None;
        }

        for offset in 0..self.masters.len() {
            let index = (self.next_master_index + offset) % self.masters.len();
            if let Some(request) = self.masters[index].master.request() {
                self.next_master_index = (index + 1) % self.masters.len();
                self.stats.master_grants += 1;
                return Some(ActiveRequest {
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
    B: Bus,
{
    fn reset(&mut self) {
        self.next_master_index = 0;
        self.active_request = None;
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

        if let Some(active) = self.active_request {
            self.cpu_reserved_this_cycle = true;
            self.dispatch_request(active);
            return;
        }

        if self.inner.is_busy() {
            return;
        }

        if let Some(active) = self.select_next_request() {
            self.cpu_reserved_this_cycle = true;
            self.active_request = Some(active);
            self.dispatch_request(active);
        }
    }

    fn is_busy(&self) -> bool {
        self.cpu_reserved_this_cycle || self.active_request.is_some() || self.inner.is_busy()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.pending_interrupts()
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
            .field("active_request", &self.active_request)
            .field("cpu_reserved_this_cycle", &self.cpu_reserved_this_cycle)
            .field("stats", &self.stats)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    use crate::{Bus, BusMaster, BusMasterRequest, BusMasterResponse};

    use super::ArbiterBus;

    #[derive(Debug, Default)]
    struct TinyBus {
        data: [u8; 32],
    }

    impl Bus for TinyBus {
        fn load8(&mut self, addr: crate::Address) -> Result<u8, crate::BusError> {
            Ok(self.data[addr as usize])
        }

        fn store8(&mut self, addr: crate::Address, value: u8) -> Result<(), crate::BusError> {
            self.data[addr as usize] = value;
            Ok(())
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
}
