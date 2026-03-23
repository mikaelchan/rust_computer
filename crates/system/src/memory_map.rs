//! Address-decoded bus implementation for memory-mapped devices.

use core::fmt;
use std::{cell::RefCell, rc::Rc};

use crate::bus::{
    AccessKind, Address, AddressRange, Addressable, BurstBus, BurstPhase, BurstRequest,
    BurstResponse, Bus, BusError, InterruptSet, TransactionBus, TransactionPhase,
    TransactionRequest, TransactionResponse,
};
use crate::cache::CacheMaintenance;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveBurst {
    id: u64,
    device_index: usize,
    request: BurstRequest,
    beat_index: usize,
    read_words: Box<[u32]>,
    phase: BurstPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompatTransaction {
    id: u64,
    request: TransactionRequest,
}

enum BeatAdvance {
    Phase(BurstPhase),
    Completed(Option<u32>),
    Failed(BusError),
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

    fn machine_time(&self) -> Option<u64> {
        self.inner.borrow().machine_time()
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
    active_transactions: Vec<ActiveTransaction>,
    active_bursts: Vec<ActiveBurst>,
    compat_transaction: Option<CompatTransaction>,
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
        self.active_transactions.clear();
        self.active_bursts.clear();
        self.compat_transaction = None;
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

    fn execute_burst_beat(
        &mut self,
        device_index: usize,
        request: &BurstRequest,
        beat_index: usize,
    ) -> Result<Option<u32>, BusError> {
        let addr = request.beat_addr(beat_index);
        match request {
            BurstRequest::ReadWords { .. } => Ok(Some(u32::from_le_bytes(
                self.load_bytes::<4>(device_index, addr)?,
            ))),
            BurstRequest::WriteWords { words, .. } => {
                self.store_bytes(device_index, addr, words[beat_index].to_le_bytes())?;
                Ok(None)
            }
        }
    }

    fn transaction_remaining_cycles(phase: &TransactionPhase) -> u32 {
        match phase {
            TransactionPhase::InFlight { remaining_cycles } => *remaining_cycles,
            TransactionPhase::Accepted
            | TransactionPhase::Ready(_)
            | TransactionPhase::Failed(_) => 1,
        }
    }

    fn compat_remaining_cycles(&self) -> u32 {
        self.compat_transaction
            .and_then(|compat| self.transaction_phase(compat.id))
            .map(|phase| Self::transaction_remaining_cycles(&phase))
            .unwrap_or(0)
    }

    fn transaction_index(&self, id: u64) -> Option<usize> {
        self.active_transactions
            .iter()
            .position(|transaction| transaction.id == id)
    }

    fn burst_index(&self, id: u64) -> Option<usize> {
        self.active_bursts.iter().position(|burst| burst.id == id)
    }

    fn advance_transaction_at(&mut self, index: usize) {
        let Some((device_index, request, phase)) = self
            .active_transactions
            .get(index)
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

        if let Some(active) = self.active_transactions.get_mut(index) {
            active.phase = next_phase;
        }
    }

    fn advance_burst_at(&mut self, index: usize) {
        let Some((device_index, request, beat_index, phase)) =
            self.active_bursts.get(index).map(|active| {
                (
                    active.device_index,
                    active.request.clone(),
                    active.beat_index,
                    active.phase.clone(),
                )
            })
        else {
            return;
        };

        let total_beats = request.beats();
        let next_state = match phase {
            BurstPhase::Accepted { .. } => {
                let latency = self.devices[device_index].device.access_latency(
                    request.beat_addr(beat_index),
                    request.beat_kind(),
                    4,
                );
                if latency == 0 {
                    match self.execute_burst_beat(device_index, &request, beat_index) {
                        Ok(read_word) => BeatAdvance::Completed(read_word),
                        Err(error) => BeatAdvance::Failed(error),
                    }
                } else {
                    BeatAdvance::Phase(BurstPhase::InFlight {
                        beat_index,
                        total_beats,
                        remaining_cycles: latency,
                    })
                }
            }
            BurstPhase::InFlight {
                remaining_cycles, ..
            } => {
                if remaining_cycles > 1 {
                    BeatAdvance::Phase(BurstPhase::InFlight {
                        beat_index,
                        total_beats,
                        remaining_cycles: remaining_cycles - 1,
                    })
                } else {
                    match self.execute_burst_beat(device_index, &request, beat_index) {
                        Ok(read_word) => BeatAdvance::Completed(read_word),
                        Err(error) => BeatAdvance::Failed(error),
                    }
                }
            }
            ready_or_failed => BeatAdvance::Phase(ready_or_failed),
        };

        if let Some(active) = self.active_bursts.get_mut(index) {
            match next_state {
                BeatAdvance::Phase(phase) => {
                    active.phase = phase;
                }
                BeatAdvance::Completed(read_word) => {
                    if let Some(word) = read_word {
                        active.read_words[beat_index] = word;
                    }

                    let next_beat_index = beat_index + 1;
                    active.beat_index = next_beat_index;
                    active.phase = if next_beat_index == total_beats {
                        BurstPhase::Ready {
                            completed_beats: total_beats,
                        }
                    } else {
                        BurstPhase::Accepted {
                            beat_index: next_beat_index,
                            total_beats,
                        }
                    };
                }
                BeatAdvance::Failed(error) => {
                    active.phase = BurstPhase::Failed(error);
                }
            }
        }
    }

    fn clear_compat_if_matches(&mut self, id: u64) {
        if self
            .compat_transaction
            .is_some_and(|compat| compat.id == id)
        {
            self.compat_transaction = None;
        }
    }

    fn perform_request(
        &mut self,
        request: TransactionRequest,
    ) -> Result<TransactionResponse, BusError> {
        match self.compat_transaction {
            Some(compat) if compat.request != request => {
                return Err(BusError::Busy {
                    remaining_cycles: self.compat_remaining_cycles().max(1),
                });
            }
            Some(compat) if self.transaction_phase(compat.id).is_none() => {
                self.compat_transaction = None;
            }
            Some(_) => {}
            None => {
                let id = self.submit_transaction(request)?;
                self.compat_transaction = Some(CompatTransaction { id, request });
            }
        }

        let compat = self
            .compat_transaction
            .expect("compatibility path should install a transaction");
        if matches!(
            self.transaction_phase(compat.id),
            Some(TransactionPhase::Accepted)
        ) {
            self.advance_transaction(compat.id);
        }

        match self.take_transaction_response(compat.id) {
            Some(result) => {
                self.compat_transaction = None;
                result
            }
            None => Err(BusError::Busy {
                remaining_cycles: self.compat_remaining_cycles().max(1),
            }),
        }
    }

    /// Submit a transaction to the memory map and leave it in the `Accepted` phase.
    pub fn submit_transaction(&mut self, request: TransactionRequest) -> Result<u64, BusError> {
        let device_index = self
            .find_device_index(request.addr)
            .ok_or(BusError::UnmappedAddress { addr: request.addr })?;
        let id = self.next_transaction_id;
        self.next_transaction_id = self.next_transaction_id.wrapping_add(1);
        self.active_transactions.push(ActiveTransaction {
            id,
            device_index,
            request,
            phase: TransactionPhase::Accepted,
        });
        Ok(id)
    }

    /// Inspect the current phase of a submitted transaction, if the ID matches.
    #[must_use]
    pub fn transaction_phase(&self, id: u64) -> Option<TransactionPhase> {
        self.transaction_index(id)
            .map(|index| self.active_transactions[index].phase.clone())
    }

    /// Advance a submitted transaction by one protocol step.
    pub fn advance_transaction(&mut self, id: u64) -> Option<TransactionPhase> {
        let index = self.transaction_index(id)?;
        self.advance_transaction_at(index);
        self.transaction_phase(id)
    }

    /// Consume a completed transaction response, or a terminal error, if available.
    pub fn take_transaction_response(
        &mut self,
        id: u64,
    ) -> Option<Result<TransactionResponse, BusError>> {
        let index = self.transaction_index(id)?;
        match self.active_transactions[index].phase.clone() {
            TransactionPhase::Accepted | TransactionPhase::InFlight { .. } => None,
            TransactionPhase::Ready(response) => {
                self.active_transactions.swap_remove(index);
                self.clear_compat_if_matches(id);
                Some(Ok(response))
            }
            TransactionPhase::Failed(error) => {
                self.active_transactions.swap_remove(index);
                self.clear_compat_if_matches(id);
                Some(Err(error))
            }
        }
    }

    /// Submit a contiguous 32-bit word burst to the memory map.
    pub fn submit_burst(&mut self, request: BurstRequest) -> Result<u64, BusError> {
        let total_beats = request.beats();
        assert!(
            total_beats > 0,
            "burst request must contain at least one beat"
        );

        let first_addr = request.base_addr();
        let device_index = self
            .find_device_index(first_addr)
            .ok_or(BusError::UnmappedAddress { addr: first_addr })?;
        for beat_index in 1..total_beats {
            let addr = request.beat_addr(beat_index);
            if self.find_device_index(addr) != Some(device_index) {
                return Err(BusError::DeviceFault {
                    addr,
                    message: "burst crosses device boundary".to_string(),
                });
            }
        }

        let id = self.next_transaction_id;
        self.next_transaction_id = self.next_transaction_id.wrapping_add(1);
        self.active_bursts.push(ActiveBurst {
            id,
            device_index,
            request,
            beat_index: 0,
            read_words: vec![0; total_beats].into_boxed_slice(),
            phase: BurstPhase::Accepted {
                beat_index: 0,
                total_beats,
            },
        });
        Ok(id)
    }

    /// Inspect the current phase of a submitted burst, if the ID matches.
    #[must_use]
    pub fn burst_phase(&self, id: u64) -> Option<BurstPhase> {
        self.burst_index(id)
            .map(|index| self.active_bursts[index].phase.clone())
    }

    /// Advance a submitted burst by one beat-level protocol step.
    pub fn advance_burst(&mut self, id: u64) -> Option<BurstPhase> {
        let index = self.burst_index(id)?;
        self.advance_burst_at(index);
        self.burst_phase(id)
    }

    /// Consume a completed burst response, or a terminal error, if available.
    pub fn take_burst_response(&mut self, id: u64) -> Option<Result<BurstResponse, BusError>> {
        let index = self.burst_index(id)?;
        match self.active_bursts[index].phase.clone() {
            BurstPhase::Accepted { .. } | BurstPhase::InFlight { .. } => None,
            BurstPhase::Ready { completed_beats } => {
                let active = self.active_bursts.swap_remove(index);
                let response = match active.request {
                    BurstRequest::ReadWords { .. } => BurstResponse::ReadWords(active.read_words),
                    BurstRequest::WriteWords { .. } => BurstResponse::WriteComplete {
                        beats: completed_beats,
                    },
                };
                Some(Ok(response))
            }
            BurstPhase::Failed(error) => {
                self.active_bursts.swap_remove(index);
                Some(Err(error))
            }
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
        for index in 0..self.active_transactions.len() {
            self.advance_transaction_at(index);
        }
        for index in 0..self.active_bursts.len() {
            self.advance_burst_at(index);
        }
        for slot in &mut self.devices {
            slot.device.tick();
        }
    }

    fn machine_time(&self) -> Option<u64> {
        self.devices
            .iter()
            .find_map(|slot| slot.device.machine_time())
    }

    fn is_busy(&self) -> bool {
        !self.active_transactions.is_empty() || !self.active_bursts.is_empty()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.devices
            .iter()
            .fold(InterruptSet::empty(), |interrupts, slot| {
                interrupts.union(slot.device.pending_interrupts())
            })
    }
}

impl TransactionBus for MemoryMap {
    fn submit_transaction(&mut self, request: TransactionRequest) -> Result<u64, BusError> {
        MemoryMap::submit_transaction(self, request)
    }

    fn transaction_phase(&self, id: u64) -> Option<TransactionPhase> {
        MemoryMap::transaction_phase(self, id)
    }

    fn advance_transaction(&mut self, id: u64) -> Option<TransactionPhase> {
        MemoryMap::advance_transaction(self, id)
    }

    fn take_transaction_response(
        &mut self,
        id: u64,
    ) -> Option<Result<TransactionResponse, BusError>> {
        MemoryMap::take_transaction_response(self, id)
    }
}

impl CacheMaintenance for MemoryMap {
    fn write_back_range(&mut self, _start: Address, _len: u64) -> Result<(), BusError> {
        Ok(())
    }

    fn invalidate_range(&mut self, _start: Address, _len: u64) -> Result<(), BusError> {
        Ok(())
    }
}

impl BurstBus for MemoryMap {
    fn submit_burst(&mut self, request: BurstRequest) -> Result<u64, BusError> {
        MemoryMap::submit_burst(self, request)
    }

    fn burst_phase(&self, id: u64) -> Option<BurstPhase> {
        MemoryMap::burst_phase(self, id)
    }

    fn advance_burst(&mut self, id: u64) -> Option<BurstPhase> {
        MemoryMap::advance_burst(self, id)
    }

    fn take_burst_response(&mut self, id: u64) -> Option<Result<BurstResponse, BusError>> {
        MemoryMap::take_burst_response(self, id)
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
            .field("active_transactions", &self.active_transactions)
            .field("active_bursts", &self.active_bursts)
            .field("compat_transaction", &self.compat_transaction)
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
        AccessKind, AddressRange, Addressable, BurstPhase, BurstRequest, BurstResponse, Bus,
        BusError, InterruptLine, InterruptSet, TransactionPhase, TransactionRequest,
        TransactionResponse,
    };

    #[derive(Debug)]
    struct CounterDevice {
        range: AddressRange,
        value: u8,
        interrupts: InterruptSet,
        latency_cycles: u32,
        machine_time: Option<u64>,
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

        fn tick(&mut self) {
            if let Some(machine_time) = &mut self.machine_time {
                *machine_time = machine_time.wrapping_add(1);
            }
        }

        fn pending_interrupts(&self) -> InterruptSet {
            self.interrupts
        }

        fn machine_time(&self) -> Option<u64> {
            self.machine_time
        }

        fn access_latency(&self, _addr: u64, _kind: AccessKind, _width: usize) -> u32 {
            self.latency_cycles
        }
    }

    #[derive(Debug)]
    struct ByteArrayDevice {
        range: AddressRange,
        bytes: Vec<u8>,
        latency_cycles: u32,
    }

    impl Addressable for ByteArrayDevice {
        fn name(&self) -> &'static str {
            "byte-array"
        }

        fn address_range(&self) -> AddressRange {
            self.range
        }

        fn load8(&mut self, addr: u64) -> Result<u8, BusError> {
            let offset = (addr - self.range.start) as usize;
            self.bytes
                .get(offset)
                .copied()
                .ok_or(BusError::UnmappedAddress { addr })
        }

        fn store8(&mut self, addr: u64, value: u8) -> Result<(), BusError> {
            let offset = (addr - self.range.start) as usize;
            let byte = self
                .bytes
                .get_mut(offset)
                .ok_or(BusError::UnmappedAddress { addr })?;
            *byte = value;
            Ok(())
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
            machine_time: None,
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
            machine_time: None,
        })
        .expect("timer-like device should map");
        map.map_device(CounterDevice {
            range: AddressRange::new(0x2000, 4),
            value: 0,
            interrupts: InterruptSet::from(InterruptLine::MachineExternal),
            latency_cycles: 0,
            machine_time: None,
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
            machine_time: None,
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
            machine_time: None,
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
    fn explicit_burst_progresses_beat_by_beat() {
        let mut map = MemoryMap::new();
        map.map_device(ByteArrayDevice {
            range: AddressRange::new(0x1000, 8),
            bytes: vec![0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55],
            latency_cycles: 2,
        })
        .expect("device should map");

        let id = map
            .submit_burst(BurstRequest::read_words(0x1000, 2, AccessKind::Load))
            .expect("burst should submit");
        assert_eq!(
            map.burst_phase(id),
            Some(BurstPhase::Accepted {
                beat_index: 0,
                total_beats: 2,
            })
        );

        assert_eq!(
            map.advance_burst(id),
            Some(BurstPhase::InFlight {
                beat_index: 0,
                total_beats: 2,
                remaining_cycles: 2,
            })
        );

        map.tick();
        assert_eq!(
            map.burst_phase(id),
            Some(BurstPhase::InFlight {
                beat_index: 0,
                total_beats: 2,
                remaining_cycles: 1,
            })
        );

        map.tick();
        assert_eq!(
            map.burst_phase(id),
            Some(BurstPhase::Accepted {
                beat_index: 1,
                total_beats: 2,
            })
        );

        map.advance_burst(id);
        map.tick();
        map.tick();

        assert_eq!(
            map.take_burst_response(id),
            Some(Ok(BurstResponse::ReadWords(
                vec![0x1122_3344, 0x5566_7788].into_boxed_slice()
            )))
        );
        assert_eq!(map.burst_phase(id), None);
    }

    #[test]
    fn explicit_transactions_can_overlap() {
        let mut map = MemoryMap::new();
        map.map_device(ByteArrayDevice {
            range: AddressRange::new(0x1000, 4),
            bytes: vec![0x11, 0x22, 0x33, 0x44],
            latency_cycles: 2,
        })
        .expect("device should map");

        let first = map
            .submit_transaction(TransactionRequest::load(0x1000, 1))
            .expect("first transaction should submit");
        let second = map
            .submit_transaction(TransactionRequest::load(0x1001, 1))
            .expect("second transaction should submit while first is still pending");

        assert_eq!(
            map.transaction_phase(first),
            Some(TransactionPhase::Accepted)
        );
        assert_eq!(
            map.transaction_phase(second),
            Some(TransactionPhase::Accepted)
        );

        map.tick();
        assert_eq!(
            map.transaction_phase(first),
            Some(TransactionPhase::InFlight {
                remaining_cycles: 2,
            })
        );
        assert_eq!(
            map.transaction_phase(second),
            Some(TransactionPhase::InFlight {
                remaining_cycles: 2,
            })
        );

        map.tick();
        map.tick();

        assert_eq!(
            map.take_transaction_response(first),
            Some(Ok(TransactionResponse::Read {
                data: [0x11, 0, 0, 0],
                width: 1,
            }))
        );
        assert_eq!(
            map.take_transaction_response(second),
            Some(Ok(TransactionResponse::Read {
                data: [0x22, 0, 0, 0],
                width: 1,
            }))
        );
    }

    #[test]
    fn transaction_and_burst_can_progress_concurrently() {
        let mut map = MemoryMap::new();
        map.map_device(ByteArrayDevice {
            range: AddressRange::new(0x1000, 12),
            bytes: vec![
                0x11, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x33, 0x00, 0x00, 0x00,
            ],
            latency_cycles: 1,
        })
        .expect("device should map");

        let transaction = map
            .submit_transaction(TransactionRequest::load(0x1000, 1))
            .expect("transaction should submit");
        let burst = map
            .submit_burst(BurstRequest::read_words(0x1004, 2, AccessKind::Load))
            .expect("burst should submit while transaction is pending");

        map.tick();
        assert_eq!(
            map.transaction_phase(transaction),
            Some(TransactionPhase::InFlight {
                remaining_cycles: 1,
            })
        );
        assert_eq!(
            map.burst_phase(burst),
            Some(BurstPhase::InFlight {
                beat_index: 0,
                total_beats: 2,
                remaining_cycles: 1,
            })
        );

        map.tick();
        assert_eq!(
            map.take_transaction_response(transaction),
            Some(Ok(TransactionResponse::Read {
                data: [0x11, 0, 0, 0],
                width: 1,
            }))
        );
        assert_eq!(
            map.burst_phase(burst),
            Some(BurstPhase::Accepted {
                beat_index: 1,
                total_beats: 2,
            })
        );

        map.tick();
        map.tick();
        assert_eq!(
            map.take_burst_response(burst),
            Some(Ok(BurstResponse::ReadWords(
                vec![0x22, 0x33].into_boxed_slice()
            )))
        );
    }

    #[test]
    fn compatibility_bus_request_can_overlap_with_explicit_transaction() {
        let mut map = MemoryMap::new();
        map.map_device(ByteArrayDevice {
            range: AddressRange::new(0x1000, 4),
            bytes: vec![0xaa, 0xbb, 0xcc, 0xdd],
            latency_cycles: 1,
        })
        .expect("device should map");

        let explicit = map
            .submit_transaction(TransactionRequest::load(0x1000, 1))
            .expect("explicit transaction should submit");

        let error = map
            .load8(0x1001)
            .expect_err("compatibility request should start and stall");
        assert_eq!(
            error,
            BusError::Busy {
                remaining_cycles: 1,
            }
        );
        assert_eq!(
            map.transaction_phase(explicit),
            Some(TransactionPhase::Accepted)
        );

        map.tick();
        assert_eq!(
            map.load8(0x1001)
                .expect("compatibility request should complete"),
            0xbb
        );
        assert_eq!(
            map.transaction_phase(explicit),
            Some(TransactionPhase::InFlight {
                remaining_cycles: 1,
            })
        );

        map.tick();
        assert_eq!(
            map.take_transaction_response(explicit),
            Some(Ok(TransactionResponse::Read {
                data: [0xaa, 0, 0, 0],
                width: 1,
            }))
        );
    }

    #[test]
    fn routes_to_shared_devices() {
        let shared = Rc::new(RefCell::new(CounterDevice {
            range: AddressRange::new(0x3000, 4),
            value: 1,
            interrupts: InterruptSet::from(InterruptLine::MachineExternal),
            latency_cycles: 0,
            machine_time: None,
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

    #[test]
    fn exposes_machine_time_from_mapped_devices() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x4000, 4),
            value: 0,
            interrupts: InterruptSet::empty(),
            latency_cycles: 0,
            machine_time: Some(7),
        })
        .expect("time-source device should map");

        assert_eq!(map.machine_time(), Some(7));

        map.tick();
        map.tick();

        assert_eq!(map.machine_time(), Some(9));
    }
}
