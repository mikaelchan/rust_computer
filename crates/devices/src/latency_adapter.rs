use rvsim_system::{
    AccessKind, Address, AddressRange, Addressable, BusError, InterruptLine, InterruptSet,
};

/// Wrap any addressable device with a fixed access latency measured in cycles.
#[derive(Debug, Clone)]
pub struct LatencyAdapter<D> {
    inner: D,
    latency_cycles: u32,
}

impl<D> LatencyAdapter<D> {
    #[must_use]
    pub fn new(inner: D, latency_cycles: u32) -> Self {
        Self {
            inner,
            latency_cycles,
        }
    }

    #[must_use]
    pub fn inner(&self) -> &D {
        &self.inner
    }
}

impl<D> Addressable for LatencyAdapter<D>
where
    D: Addressable,
{
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn address_range(&self) -> AddressRange {
        self.inner.address_range()
    }

    fn reset(&mut self) {
        self.inner.reset();
    }

    fn tick(&mut self) {
        self.inner.tick();
    }

    fn access_latency(&self, _addr: Address, _kind: AccessKind, _width: usize) -> u32 {
        self.latency_cycles
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.pending_interrupts()
    }

    fn pending_interrupt(&self) -> Option<InterruptLine> {
        self.inner.pending_interrupt()
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        self.inner.load8(addr)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        self.inner.store8(addr, value)
    }
}
