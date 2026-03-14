//! Address-decoded bus implementation for memory-mapped devices.

use core::fmt;
use std::{cell::RefCell, rc::Rc};

use crate::bus::{
    AccessKind, Address, AddressRange, Addressable, Bus, BusError, InterruptSet, TransactionPhase,
    TransactionRequest, TransactionResponse,
};

struct DeviceSlot {
    range: AddressRange,
    name: &'static str,
    device: Box<dyn Addressable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTransaction {
    id: u64,
    device_index: usize,
    request: TransactionRequest,
    phase: TransactionPhase,
}

struct SharedAddressable<D> {
    inner: Rc<RefCell<D>>,
}

impl<D> Addressable for SharedAddressable<D>
where
    D: Addressable + 'static,
{
    fn name(&self) -> &'static str {
        self.inner.borrow().name()
    }

    fn address_range(&self) -> AddressRange {
        self.inner.borrow().address_range()
    }

    fn reset(&mut self) {
        self.inner.borrow_mut().reset();
    }

    fn tick(&mut self) {
        self.inner.borrow_mut().tick();
    }

    fn access_latency(&self, addr: Address, kind: AccessKind, width: usize) -> u32 {
        self.inner.borrow().access_latency(addr, kind, width)
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.borrow().pending_interrupts()
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        self.inner.borrow_mut().load8(addr)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        self.inner.borrow_mut().store8(addr, value)
    }
}

/// A simple bus that routes accesses to the first mapped device that contains the address.
#[derive(Default)]
pub struct MemoryMap {
    devices: Vec<DeviceSlot>,
    active_transaction: Option<ActiveTransaction>,
    next_transaction_id: u64,
}

impl MemoryMap {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Map a device into the physical address space, rejecting overlapping regions.
    pub fn map_device<D>(&mut self, device: D) -> Result<(), BusError>
    where
        D: Addressable + 'static,
    {
        let range = device.address_range();
        let name = device.name();
        self.ensure_range_available(range, name)?;
        self.devices.push(DeviceSlot {
            range,
            name,
            device: Box::new(device),
        });
        Ok(())
    }

    pub fn map_shared_device<D>(&mut self, device: Rc<RefCell<D>>) -> Result<(), BusError>
    where
        D: Addressable + 'static,
    {
        let borrowed = device.borrow();
        let range = borrowed.address_range();
        let name = borrowed.name();
        drop(borrowed);
        self.ensure_range_available(range, name)?;
        self.devices.push(DeviceSlot {
            range,
            name,
            device: Box::new(SharedAddressable { inner: device }),
        });
        Ok(())
    }

    fn ensure_range_available(
        &self,
        range: AddressRange,
        name: &'static str,
    ) -> Result<(), BusError> {
        for slot in &self.devices {
            if slot.range.overlaps(range) {
                return Err(BusError::DeviceFault {
                    addr: range.start,
                    message: format!("device {name} overlaps with {}", slot.name),
                });
            }
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.active_transaction = None;
        self.next_transaction_id = 0;
        for slot in &mut self.devices {
            slot.device.reset();
        }
    }

    fn find_device_index(&self, addr: Address) -> Option<usize> {
        self.devices
            .iter()
            .position(|slot| slot.range.contains(addr))
    }

    fn load_bytes<const N: usize>(
        &mut self,
        index: usize,
        addr: Address,
    ) -> Result<[u8; N], BusError> {
        let mut bytes = [0; N];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = self.devices[index].device.load8(addr + offset as u64)?;
        }
        Ok(bytes)
    }

    fn store_bytes<const N: usize>(
        &mut self,
        index: usize,
        addr: Address,
        bytes: [u8; N],
    ) -> Result<(), BusError> {
        for (offset, byte) in bytes.into_iter().enumerate() {
            self.devices[index]
                .device
                .store8(addr + offset as u64, byte)?;
        }
        Ok(())
    }

    fn execute_request(
        &mut self,
        device_index: usize,
        request: TransactionRequest,
    ) -> Result<TransactionResponse, BusError> {
        match (request.kind, request.width) {
            (AccessKind::Fetch, 4) | (AccessKind::Load, 4) => Ok(TransactionResponse::Read {
                data: self.load_bytes::<4>(device_index, request.addr)?,
                width: 4,
            }),
            (AccessKind::Load, 2) => Ok(TransactionResponse::Read {
                data: {
                    let mut bytes = [0; 4];
                    bytes[..2].copy_from_slice(&self.load_bytes::<2>(device_index, request.addr)?);
                    bytes
                },
                width: 2,
            }),
            (AccessKind::Load, 1) => Ok(TransactionResponse::Read {
                data: {
                    let mut bytes = [0; 4];
                    bytes[0] = self.devices[device_index].device.load8(request.addr)?;
                    bytes
                },
                width: 1,
            }),
            (AccessKind::Store, 4) => {
                self.store_bytes(device_index, request.addr, request.write_data)?;
                Ok(TransactionResponse::WriteComplete)
            }
            (AccessKind::Store, 2) => {
                self.store_bytes(
                    device_index,
                    request.addr,
                    [request.write_data[0], request.write_data[1]],
                )?;
                Ok(TransactionResponse::WriteComplete)
            }
            (AccessKind::Store, 1) => {
                self.devices[device_index]
                    .device
                    .store8(request.addr, request.write_data[0])?;
                Ok(TransactionResponse::WriteComplete)
            }
            _ => Err(BusError::DeviceFault {
                addr: request.addr,
                message: format!(
                    "unsupported transaction {:?} width {}",
                    request.kind, request.width
                ),
            }),
        }
    }

    fn active_remaining_cycles(&self) -> u32 {
        match self
            .active_transaction
            .as_ref()
            .map(|transaction| &transaction.phase)
        {
            Some(TransactionPhase::InFlight { remaining_cycles }) => *remaining_cycles,
            Some(TransactionPhase::Accepted)
            | Some(TransactionPhase::Ready(_))
            | Some(TransactionPhase::Failed(_)) => 1,
            None => 0,
        }
    }

    fn advance_active_transaction(&mut self) {
        let Some((device_index, request, phase)) = self
            .active_transaction
            .as_ref()
            .map(|active| (active.device_index, active.request, active.phase.clone()))
        else {
            return;
        };

        let next_phase = match phase {
            TransactionPhase::Accepted => {
                let latency = self.devices[device_index].device.access_latency(
                    request.addr,
                    request.kind,
                    request.width,
                );
                if latency == 0 {
                    match self.execute_request(device_index, request) {
                        Ok(response) => TransactionPhase::Ready(response),
                        Err(error) => TransactionPhase::Failed(error),
                    }
                } else {
                    TransactionPhase::InFlight {
                        remaining_cycles: latency,
                    }
                }
            }
            TransactionPhase::InFlight { remaining_cycles } => {
                if remaining_cycles > 1 {
                    TransactionPhase::InFlight {
                        remaining_cycles: remaining_cycles - 1,
                    }
                } else {
                    match self.execute_request(device_index, request) {
                        Ok(response) => TransactionPhase::Ready(response),
                        Err(error) => TransactionPhase::Failed(error),
                    }
                }
            }
            ready_or_failed => ready_or_failed,
        };

        if let Some(active) = &mut self.active_transaction {
            active.phase = next_phase;
        }
    }

    fn finish_active_request(&mut self) -> Result<TransactionResponse, BusError> {
        let phase = self
            .active_transaction
            .as_ref()
            .map(|active| active.phase.clone())
            .expect("finish_active_request should only be called with an active transaction");

        match phase {
            TransactionPhase::Accepted => Err(BusError::Busy {
                remaining_cycles: 1,
            }),
            TransactionPhase::InFlight { remaining_cycles } => {
                Err(BusError::Busy { remaining_cycles })
            }
            TransactionPhase::Ready(response) => {
                self.active_transaction = None;
                Ok(response)
            }
            TransactionPhase::Failed(error) => {
                self.active_transaction = None;
                Err(error)
            }
        }
    }

    fn perform_request(
        &mut self,
        request: TransactionRequest,
    ) -> Result<TransactionResponse, BusError> {
        if let Some(active) = &self.active_transaction {
            if active.request != request {
                return Err(BusError::Busy {
                    remaining_cycles: self.active_remaining_cycles(),
                });
            }
        } else {
            self.submit_transaction(request)?;
        }

        if matches!(
            self.active_transaction.as_ref().map(|active| &active.phase),
            Some(TransactionPhase::Accepted)
        ) {
            self.advance_active_transaction();
        }

        self.finish_active_request()
    }

    /// Submit a single transaction to the memory map and leave it in the `Accepted` phase.
    pub fn submit_transaction(&mut self, request: TransactionRequest) -> Result<u64, BusError> {
        if self.active_transaction.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: self.active_remaining_cycles(),
            });
        }

        let device_index = self
            .find_device_index(request.addr)
            .ok_or(BusError::UnmappedAddress { addr: request.addr })?;
        let id = self.next_transaction_id;
        self.next_transaction_id = self.next_transaction_id.wrapping_add(1);
        self.active_transaction = Some(ActiveTransaction {
            id,
            device_index,
            request,
            phase: TransactionPhase::Accepted,
        });
        Ok(id)
    }

    /// Inspect the current phase of the outstanding transaction, if the IDs match.
    #[must_use]
    pub fn transaction_phase(&self, id: u64) -> Option<TransactionPhase> {
        self.active_transaction
            .as_ref()
            .filter(|active| active.id == id)
            .map(|active| active.phase.clone())
    }

    /// Advance the single outstanding transaction by one protocol step.
    pub fn advance_transaction(&mut self, id: u64) -> Option<TransactionPhase> {
        if self
            .active_transaction
            .as_ref()
            .is_none_or(|active| active.id != id)
        {
            return None;
        }

        self.advance_active_transaction();
        self.transaction_phase(id)
    }

    /// Consume a completed transaction response, or a terminal error, if available.
    pub fn take_transaction_response(
        &mut self,
        id: u64,
    ) -> Option<Result<TransactionResponse, BusError>> {
        if self
            .active_transaction
            .as_ref()
            .is_none_or(|active| active.id != id)
        {
            return None;
        }

        match self.finish_active_request() {
            Ok(response) => Some(Ok(response)),
            Err(BusError::Busy { .. }) => None,
            Err(error) => Some(Err(error)),
        }
    }
}

impl Bus for MemoryMap {
    fn reset(&mut self) {
        MemoryMap::reset(self);
    }

    fn fetch32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        match self.perform_request(TransactionRequest::fetch32(addr))? {
            TransactionResponse::Read { data, width: 4 } => Ok(u32::from_le_bytes(data)),
            _ => unreachable!("fetch32 should complete with a 32-bit read response"),
        }
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        match self.perform_request(TransactionRequest::load(addr, 1))? {
            TransactionResponse::Read { data, width: 1 } => Ok(data[0]),
            _ => unreachable!("load8 should complete with an 8-bit read response"),
        }
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        self.perform_request(TransactionRequest::store(addr, 1, [value, 0, 0, 0]))?;
        Ok(())
    }

    fn load16(&mut self, addr: Address) -> Result<u16, BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        match self.perform_request(TransactionRequest::load(addr, 2))? {
            TransactionResponse::Read { data, width: 2 } => {
                Ok(u16::from_le_bytes([data[0], data[1]]))
            }
            _ => unreachable!("load16 should complete with a 16-bit read response"),
        }
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        match self.perform_request(TransactionRequest::load(addr, 4))? {
            TransactionResponse::Read { data, width: 4 } => Ok(u32::from_le_bytes(data)),
            _ => unreachable!("load32 should complete with a 32-bit read response"),
        }
    }

    fn store16(&mut self, addr: Address, value: u16) -> Result<(), BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        let mut bytes = [0; 4];
        bytes[..2].copy_from_slice(&value.to_le_bytes());
        self.perform_request(TransactionRequest::store(addr, 2, bytes))?;
        Ok(())
    }

    fn store32(&mut self, addr: Address, value: u32) -> Result<(), BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        self.perform_request(TransactionRequest::store(addr, 4, value.to_le_bytes()))?;
        Ok(())
    }

    fn tick(&mut self) {
        self.advance_active_transaction();
        for slot in &mut self.devices {
            slot.device.tick();
        }
    }

    fn is_busy(&self) -> bool {
        self.active_transaction.is_some()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.devices
            .iter()
            .fold(InterruptSet::empty(), |interrupts, slot| {
                interrupts.union(slot.device.pending_interrupts())
            })
    }
}

impl fmt::Debug for MemoryMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("MemoryMap");
        let devices: Vec<_> = self
            .devices
            .iter()
            .map(|slot| (slot.name, slot.range.start, slot.range.size))
            .collect();
        debug
            .field("active_transaction", &self.active_transaction)
            .field("next_transaction_id", &self.next_transaction_id)
            .field("devices", &devices)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::MemoryMap;
    use crate::{
        AccessKind, AddressRange, Addressable, Bus, BusError, InterruptLine, InterruptSet,
        TransactionPhase, TransactionRequest, TransactionResponse,
    };

    #[derive(Debug)]
    struct CounterDevice {
        range: AddressRange,
        value: u8,
        interrupts: InterruptSet,
        latency_cycles: u32,
    }

    impl Addressable for CounterDevice {
        fn name(&self) -> &'static str {
            "counter"
        }

        fn address_range(&self) -> AddressRange {
            self.range
        }

        fn load8(&mut self, _addr: u64) -> Result<u8, BusError> {
            Ok(self.value)
        }

        fn store8(&mut self, _addr: u64, value: u8) -> Result<(), BusError> {
            self.value = value;
            Ok(())
        }

        fn pending_interrupts(&self) -> InterruptSet {
            self.interrupts
        }

        fn access_latency(&self, _addr: u64, _kind: AccessKind, _width: usize) -> u32 {
            self.latency_cycles
        }
    }

    #[test]
    fn routes_to_mapped_device() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x1000, 4),
            value: 0,
            interrupts: InterruptSet::empty(),
            latency_cycles: 0,
        })
        .expect("device should map");

        map.store8(0x1000, 7).expect("write should succeed");
        assert_eq!(map.load8(0x1000).expect("read should succeed"), 7);
    }

    #[test]
    fn aggregates_interrupts_across_devices() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x1000, 4),
            value: 0,
            interrupts: InterruptSet::from(InterruptLine::MachineTimer),
            latency_cycles: 0,
        })
        .expect("timer-like device should map");
        map.map_device(CounterDevice {
            range: AddressRange::new(0x2000, 4),
            value: 0,
            interrupts: InterruptSet::from(InterruptLine::MachineExternal),
            latency_cycles: 0,
        })
        .expect("external device should map");

        let interrupts = map.pending_interrupts();
        assert!(interrupts.contains(InterruptLine::MachineTimer));
        assert!(interrupts.contains(InterruptLine::MachineExternal));
        assert_eq!(
            interrupts.highest_priority(),
            Some(InterruptLine::MachineExternal)
        );
    }

    #[test]
    fn delays_access_until_busy_cycles_elapse() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x1000, 4),
            value: 9,
            interrupts: InterruptSet::empty(),
            latency_cycles: 2,
        })
        .expect("device should map");

        let error = map.load8(0x1000).expect_err("first access should stall");
        assert_eq!(
            error,
            BusError::Busy {
                remaining_cycles: 2
            }
        );
        assert!(map.is_busy());

        map.tick();
        let error = map
            .load8(0x1000)
            .expect_err("second access should still stall");
        assert_eq!(
            error,
            BusError::Busy {
                remaining_cycles: 1
            }
        );

        map.tick();
        assert_eq!(map.load8(0x1000).expect("access should complete"), 9);
        assert!(!map.is_busy());
    }

    #[test]
    fn explicit_transaction_progresses_from_accepted_to_ready() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x1000, 4),
            value: 0x5a,
            interrupts: InterruptSet::empty(),
            latency_cycles: 2,
        })
        .expect("device should map");

        let id = map
            .submit_transaction(TransactionRequest::load(0x1000, 1))
            .expect("transaction should submit");
        assert_eq!(map.transaction_phase(id), Some(TransactionPhase::Accepted));

        assert_eq!(
            map.advance_transaction(id),
            Some(TransactionPhase::InFlight {
                remaining_cycles: 2
            })
        );

        map.tick();
        assert_eq!(
            map.transaction_phase(id),
            Some(TransactionPhase::InFlight {
                remaining_cycles: 1
            })
        );

        map.tick();
        assert_eq!(
            map.take_transaction_response(id),
            Some(Ok(TransactionResponse::Read {
                data: [0x5a, 0, 0, 0],
                width: 1,
            }))
        );
        assert_eq!(map.transaction_phase(id), None);
    }

    #[test]
    fn routes_to_shared_devices() {
        let shared = Rc::new(RefCell::new(CounterDevice {
            range: AddressRange::new(0x3000, 4),
            value: 1,
            interrupts: InterruptSet::from(InterruptLine::MachineExternal),
            latency_cycles: 0,
        }));
        let mut map = MemoryMap::new();
        map.map_shared_device(Rc::clone(&shared))
            .expect("shared device should map");

        map.store8(0x3000, 5)
            .expect("write should reach shared device");

        assert_eq!(shared.borrow().value, 5);
        assert_eq!(
            map.pending_interrupts().highest_priority(),
            Some(InterruptLine::MachineExternal)
        );
    }
}
