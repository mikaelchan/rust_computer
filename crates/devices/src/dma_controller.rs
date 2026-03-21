use std::collections::VecDeque;

use rvsim_system::{
    Address, AddressRange, Addressable, BusError, BusMaster, BusMasterRequest, BusMasterResponse,
    InterruptLine, InterruptSet,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingWriteBurst {
    destination: Address,
    words: Box<[u32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IssuedDmaRequest {
    ReadBurst { destination: Address, beats: usize },
    WriteBurst { beats: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTransfer {
    next_source: Address,
    next_destination: Address,
    remaining_words: u32,
    ready_writes: VecDeque<PendingWriteBurst>,
    issued_requests: VecDeque<IssuedDmaRequest>,
}

impl ActiveTransfer {
    fn buffered_read_bursts(&self) -> usize {
        self.ready_writes.len()
            + self
                .issued_requests
                .iter()
                .filter(|request| matches!(request, IssuedDmaRequest::ReadBurst { .. }))
                .count()
    }

    fn is_complete(&self) -> bool {
        self.remaining_words == 0 && self.ready_writes.is_empty() && self.issued_requests.is_empty()
    }
}

/// A simple memory-to-memory DMA engine with bounded read-ahead bursts.
#[derive(Debug, Clone)]
pub struct DmaController {
    range: AddressRange,
    source: u32,
    destination: u32,
    length_words: u32,
    transferred_words: u32,
    irq_enabled: bool,
    done: bool,
    error: bool,
    active: Option<ActiveTransfer>,
}

impl DmaController {
    const MAX_BURST_WORDS: usize = 8;
    const MAX_OUTSTANDING_REQUESTS: usize = 4;

    pub const SOURCE_OFFSET: Address = 0;
    pub const DESTINATION_OFFSET: Address = 4;
    pub const LENGTH_OFFSET: Address = 8;
    pub const CONTROL_OFFSET: Address = 12;
    pub const TRANSFERRED_OFFSET: Address = 16;

    pub const CONTROL_START: u32 = 1 << 0;
    pub const STATUS_BUSY: u32 = 1 << 1;
    pub const STATUS_DONE: u32 = 1 << 2;
    pub const CONTROL_IRQ_ENABLE: u32 = 1 << 3;
    pub const STATUS_ERROR: u32 = 1 << 4;

    #[must_use]
    pub fn new(base: Address) -> Self {
        Self {
            range: AddressRange::new(base, 20),
            source: 0,
            destination: 0,
            length_words: 0,
            transferred_words: 0,
            irq_enabled: false,
            done: false,
            error: false,
            active: None,
        }
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    #[must_use]
    pub const fn is_done(&self) -> bool {
        self.done
    }

    #[must_use]
    pub const fn has_error(&self) -> bool {
        self.error
    }

    fn offset(&self, addr: Address) -> Result<Address, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }

        Ok(addr - self.range.start)
    }

    fn control_word(&self) -> u32 {
        let mut value = 0;
        if self.is_busy() {
            value |= Self::STATUS_BUSY;
        }
        if self.done {
            value |= Self::STATUS_DONE;
        }
        if self.irq_enabled {
            value |= Self::CONTROL_IRQ_ENABLE;
        }
        if self.error {
            value |= Self::STATUS_ERROR;
        }
        value
    }

    fn read_register_word(&self, offset: Address) -> Result<u32, BusError> {
        match offset {
            Self::SOURCE_OFFSET => Ok(self.source),
            Self::DESTINATION_OFFSET => Ok(self.destination),
            Self::LENGTH_OFFSET => Ok(self.length_words),
            Self::CONTROL_OFFSET => Ok(self.control_word()),
            Self::TRANSFERRED_OFFSET => Ok(self.transferred_words),
            _ => Err(BusError::UnmappedAddress {
                addr: self.range.start + offset,
            }),
        }
    }

    fn write_u32_byte(value: &mut u32, byte_index: usize, byte: u8) {
        let mut bytes = value.to_le_bytes();
        bytes[byte_index] = byte;
        *value = u32::from_le_bytes(bytes);
    }

    fn start_transfer(&mut self) {
        if self.is_busy() {
            return;
        }

        self.done = false;
        self.error = false;
        self.transferred_words = 0;

        if self.length_words == 0 {
            self.error = true;
            return;
        }

        self.active = Some(ActiveTransfer {
            next_source: u64::from(self.source),
            next_destination: u64::from(self.destination),
            remaining_words: self.length_words,
            ready_writes: VecDeque::new(),
            issued_requests: VecDeque::new(),
        });
    }

    fn burst_words(remaining_words: u32) -> usize {
        remaining_words.min(Self::MAX_BURST_WORDS as u32) as usize
    }

    fn apply_control_byte(&mut self, value: u8) {
        self.irq_enabled = (u32::from(value) & Self::CONTROL_IRQ_ENABLE) != 0;
        if (u32::from(value) & Self::STATUS_DONE) != 0 {
            self.done = false;
        }
        if (u32::from(value) & Self::STATUS_ERROR) != 0 {
            self.error = false;
        }
        if (u32::from(value) & Self::CONTROL_START) != 0 {
            self.start_transfer();
        }
    }
}

impl Addressable for DmaController {
    fn name(&self) -> &'static str {
        "dma-controller"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.source = 0;
        self.destination = 0;
        self.length_words = 0;
        self.transferred_words = 0;
        self.irq_enabled = false;
        self.done = false;
        self.error = false;
        self.active = None;
    }

    fn pending_interrupts(&self) -> InterruptSet {
        if self.irq_enabled && (self.done || self.error) {
            InterruptSet::from(InterruptLine::MachineExternal)
        } else {
            InterruptSet::empty()
        }
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let offset = self.offset(addr)?;
        let byte_index = (offset & 0b11) as usize;
        let word = self.read_register_word(offset & !0b11)?;
        Ok(word.to_le_bytes()[byte_index])
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        let offset = self.offset(addr)?;
        let byte_index = (offset & 0b11) as usize;

        match offset & !0b11 {
            Self::SOURCE_OFFSET => Self::write_u32_byte(&mut self.source, byte_index, value),
            Self::DESTINATION_OFFSET => {
                Self::write_u32_byte(&mut self.destination, byte_index, value);
            }
            Self::LENGTH_OFFSET => Self::write_u32_byte(&mut self.length_words, byte_index, value),
            Self::CONTROL_OFFSET => {
                if byte_index == 0 {
                    self.apply_control_byte(value);
                }
            }
            Self::TRANSFERRED_OFFSET => return Err(BusError::ReadOnlyAddress { addr }),
            _ => return Err(BusError::UnmappedAddress { addr }),
        }

        Ok(())
    }
}

impl BusMaster for DmaController {
    fn name(&self) -> &'static str {
        "dma-controller"
    }

    fn max_outstanding_requests(&self) -> usize {
        Self::MAX_OUTSTANDING_REQUESTS
    }

    fn request(&mut self) -> Option<BusMasterRequest> {
        let active = self.active.as_mut()?;

        if !active.ready_writes.is_empty()
            && (active.remaining_words == 0
                || active.buffered_read_bursts() >= Self::MAX_OUTSTANDING_REQUESTS)
        {
            let write = active
                .ready_writes
                .pop_front()
                .expect("checked that a pending write is available");
            let beats = write.words.len();
            active
                .issued_requests
                .push_back(IssuedDmaRequest::WriteBurst { beats });
            return Some(BusMasterRequest::WriteWords {
                base_addr: write.destination,
                words: write.words,
            });
        }

        if active.remaining_words > 0
            && active.buffered_read_bursts() < Self::MAX_OUTSTANDING_REQUESTS
        {
            let beats = Self::burst_words(active.remaining_words);
            let source = active.next_source;
            let destination = active.next_destination;
            let advance_words = beats as u32;
            active.next_source += u64::from(advance_words) * 4;
            active.next_destination += u64::from(advance_words) * 4;
            active.remaining_words -= advance_words;
            active
                .issued_requests
                .push_back(IssuedDmaRequest::ReadBurst { destination, beats });
            return Some(BusMasterRequest::ReadWords {
                base_addr: source,
                beats,
            });
        }

        let write = active.ready_writes.pop_front()?;
        let beats = write.words.len();
        active
            .issued_requests
            .push_back(IssuedDmaRequest::WriteBurst { beats });
        Some(BusMasterRequest::WriteWords {
            base_addr: write.destination,
            words: write.words,
        })
    }

    fn on_response(&mut self, response: Result<BusMasterResponse, BusError>) {
        let Some(mut active) = self.active.take() else {
            return;
        };

        let Some(issued_request) = active.issued_requests.pop_front() else {
            self.error = true;
            return;
        };

        match (issued_request, response) {
            (
                IssuedDmaRequest::ReadBurst { destination, beats },
                Ok(BusMasterResponse::ReadWords(words)),
            ) if words.len() == beats => {
                active
                    .ready_writes
                    .push_back(PendingWriteBurst { destination, words });
            }
            (
                IssuedDmaRequest::WriteBurst { beats },
                Ok(BusMasterResponse::WriteWordsComplete {
                    beats: completed_beats,
                }),
            ) if completed_beats == beats => {
                self.transferred_words += beats as u32;
            }
            (
                IssuedDmaRequest::ReadBurst { .. },
                Ok(BusMasterResponse::Load32(_) | BusMasterResponse::StoreComplete),
            )
            | (
                IssuedDmaRequest::WriteBurst { .. },
                Ok(BusMasterResponse::Load32(_) | BusMasterResponse::StoreComplete),
            )
            | (
                IssuedDmaRequest::ReadBurst { .. },
                Ok(BusMasterResponse::WriteWordsComplete { .. }),
            )
            | (IssuedDmaRequest::WriteBurst { .. }, Ok(BusMasterResponse::ReadWords(_)))
            | (_, Err(_)) => {
                self.error = true;
                return;
            }
            _ => {
                self.error = true;
                return;
            }
        }

        if active.is_complete() {
            self.done = true;
            return;
        }

        self.active = Some(active);
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use rvsim_system::{
        AddressRange, ArbiterBus, Bus, BusError, CacheConfig, CacheMaintenance, DirectMappedCache,
        InterruptLine, MemoryMap, SplitL1Cache, StoreAllocationPolicy, WritePolicy,
    };

    use crate::{LatencyAdapter, Ram};

    use super::DmaController;

    const RAM_BASE: u64 = 0x1000_0000;
    const DMA_BASE: u64 = 0x4000_0000;

    fn blocking_store32<B>(bus: &mut B, addr: u64, value: u32)
    where
        B: Bus,
    {
        loop {
            match bus.store32(addr, value) {
                Ok(()) => return,
                Err(BusError::Busy { .. }) => bus.tick(),
                Err(error) => panic!("store32 at 0x{addr:08x} failed: {error:?}"),
            }
        }
    }

    fn blocking_load32<B>(bus: &mut B, addr: u64) -> u32
    where
        B: Bus,
    {
        loop {
            match bus.load32(addr) {
                Ok(value) => return value,
                Err(BusError::Busy { .. }) => bus.tick(),
                Err(error) => panic!("load32 at 0x{addr:08x} failed: {error:?}"),
            }
        }
    }

    fn blocking_write_back_range<B>(bus: &mut B, start: u64, len: u64)
    where
        B: Bus + CacheMaintenance,
    {
        loop {
            match bus.write_back_range(start, len) {
                Ok(()) => return,
                Err(BusError::Busy { .. }) => bus.tick(),
                Err(error) => panic!("write_back_range failed: {error:?}"),
            }
        }
    }

    fn blocking_invalidate_range<B>(bus: &mut B, start: u64, len: u64)
    where
        B: Bus + CacheMaintenance,
    {
        loop {
            match bus.invalidate_range(start, len) {
                Ok(()) => return,
                Err(BusError::Busy { .. }) => bus.tick(),
                Err(error) => panic!("invalidate_range failed: {error:?}"),
            }
        }
    }

    #[test]
    fn copies_words_and_raises_completion_interrupt() {
        let dma = Rc::new(RefCell::new(DmaController::new(DMA_BASE)));
        let mut memory = MemoryMap::new();
        memory
            .map_device(Ram::new(RAM_BASE, 0x100))
            .expect("RAM should map");
        memory
            .map_shared_device(Rc::clone(&dma))
            .expect("DMA should map");

        let mut bus = ArbiterBus::new(memory);
        bus.add_shared_master(Rc::clone(&dma));

        bus.store32(RAM_BASE, 0x1122_3344)
            .expect("source word 0 should write");
        bus.store32(RAM_BASE + 4, 0x5566_7788)
            .expect("source word 1 should write");
        bus.store32(DMA_BASE + DmaController::SOURCE_OFFSET, RAM_BASE as u32)
            .expect("DMA source should program");
        bus.store32(
            DMA_BASE + DmaController::DESTINATION_OFFSET,
            (RAM_BASE + 0x40) as u32,
        )
        .expect("DMA destination should program");
        bus.store32(DMA_BASE + DmaController::LENGTH_OFFSET, 2)
            .expect("DMA length should program");
        bus.store32(
            DMA_BASE + DmaController::CONTROL_OFFSET,
            DmaController::CONTROL_START | DmaController::CONTROL_IRQ_ENABLE,
        )
        .expect("DMA control should start transfer");

        for _ in 0..16 {
            bus.tick();
            if dma.borrow().is_done() {
                break;
            }
        }
        bus.tick();

        assert_eq!(
            bus.load32(RAM_BASE + 0x40)
                .expect("copied word 0 should read"),
            0x1122_3344
        );
        assert_eq!(
            bus.load32(RAM_BASE + 0x44)
                .expect("copied word 1 should read"),
            0x5566_7788
        );
        assert!(dma.borrow().is_done(), "{:?}", dma.borrow());
        assert!(!dma.borrow().has_error());
        assert_eq!(
            bus.pending_interrupts().highest_priority(),
            Some(InterruptLine::MachineExternal)
        );
    }

    #[test]
    fn latches_error_for_bad_source_address() {
        let dma = Rc::new(RefCell::new(DmaController::new(DMA_BASE)));
        let mut memory = MemoryMap::new();
        memory
            .map_device(Ram::new(RAM_BASE, 0x100))
            .expect("RAM should map");
        memory
            .map_shared_device(Rc::clone(&dma))
            .expect("DMA should map");

        let mut bus = ArbiterBus::new(memory);
        bus.add_shared_master(Rc::clone(&dma));

        bus.store32(DMA_BASE + DmaController::SOURCE_OFFSET, 0xdead_0000)
            .expect("DMA source should program");
        bus.store32(
            DMA_BASE + DmaController::DESTINATION_OFFSET,
            RAM_BASE as u32,
        )
        .expect("DMA destination should program");
        bus.store32(DMA_BASE + DmaController::LENGTH_OFFSET, 1)
            .expect("DMA length should program");
        bus.store32(
            DMA_BASE + DmaController::CONTROL_OFFSET,
            DmaController::CONTROL_START | DmaController::CONTROL_IRQ_ENABLE,
        )
        .expect("DMA control should start transfer");

        for _ in 0..4 {
            bus.tick();
            if !dma.borrow().is_busy() {
                break;
            }
        }
        bus.tick();

        assert!(dma.borrow().has_error());
        assert!(!dma.borrow().is_done());
        assert_eq!(
            bus.pending_interrupts().highest_priority(),
            Some(InterruptLine::MachineExternal)
        );
        assert_eq!(
            bus.load32(DMA_BASE + DmaController::CONTROL_OFFSET)
                .expect("DMA status should read")
                & DmaController::STATUS_ERROR,
            DmaController::STATUS_ERROR
        );
    }

    #[test]
    fn issues_burst_requests_for_multiword_transfers() {
        let dma = Rc::new(RefCell::new(DmaController::new(DMA_BASE)));
        let mut memory = MemoryMap::new();
        memory
            .map_device(Ram::new(RAM_BASE, 0x200))
            .expect("RAM should map");
        memory
            .map_shared_device(Rc::clone(&dma))
            .expect("DMA should map");

        let mut bus = ArbiterBus::new(memory);
        bus.add_shared_master(Rc::clone(&dma));

        for (index, word) in [0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]
            .into_iter()
            .enumerate()
        {
            bus.store32(RAM_BASE + (index as u64 * 4), word)
                .expect("source word should write");
        }

        bus.store32(DMA_BASE + DmaController::SOURCE_OFFSET, RAM_BASE as u32)
            .expect("DMA source should program");
        bus.store32(
            DMA_BASE + DmaController::DESTINATION_OFFSET,
            (RAM_BASE + 0x80) as u32,
        )
        .expect("DMA destination should program");
        bus.store32(DMA_BASE + DmaController::LENGTH_OFFSET, 4)
            .expect("DMA length should program");
        bus.store32(
            DMA_BASE + DmaController::CONTROL_OFFSET,
            DmaController::CONTROL_START,
        )
        .expect("DMA control should start transfer");

        for _ in 0..16 {
            bus.tick();
            if dma.borrow().is_done() {
                break;
            }
        }

        assert!(dma.borrow().is_done(), "{:?}", dma.borrow());
        assert_eq!(bus.stats().master_grants, 2);
        for index in 0..4 {
            assert_eq!(
                bus.load32(RAM_BASE + 0x80 + (index as u64 * 4))
                    .expect("copied word should read"),
                [0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444][index]
            );
        }
    }

    #[test]
    fn overlaps_multiple_dma_bursts_on_high_latency_memory() {
        let dma = Rc::new(RefCell::new(DmaController::new(DMA_BASE)));
        let mut memory = MemoryMap::new();
        memory
            .map_device(LatencyAdapter::new(Ram::new(RAM_BASE, 0x400), 3))
            .expect("RAM should map");
        memory
            .map_shared_device(Rc::clone(&dma))
            .expect("DMA should map");

        let mut bus = ArbiterBus::new(memory);
        bus.add_shared_master(Rc::clone(&dma));

        for index in 0..24 {
            blocking_store32(
                &mut bus,
                RAM_BASE + (index as u64 * 4),
                0x1000_0000 + index as u32,
            );
        }

        bus.store32(DMA_BASE + DmaController::SOURCE_OFFSET, RAM_BASE as u32)
            .expect("DMA source should program");
        bus.store32(
            DMA_BASE + DmaController::DESTINATION_OFFSET,
            (RAM_BASE + 0x100) as u32,
        )
        .expect("DMA destination should program");
        bus.store32(DMA_BASE + DmaController::LENGTH_OFFSET, 24)
            .expect("DMA length should program");
        bus.store32(
            DMA_BASE + DmaController::CONTROL_OFFSET,
            DmaController::CONTROL_START,
        )
        .expect("DMA control should start transfer");

        for _ in 0..3 {
            bus.tick();
        }

        assert!(
            dma.borrow().is_busy(),
            "DMA should still be active while multiple reads are in flight"
        );
        assert_eq!(
            bus.stats().master_grants,
            3,
            "DMA should issue multiple bursts before the first response completes"
        );

        for _ in 0..128 {
            bus.tick();
            if dma.borrow().is_done() {
                break;
            }
        }

        assert!(dma.borrow().is_done(), "{:?}", dma.borrow());
        assert!(!dma.borrow().has_error());
        for index in 0..24 {
            assert_eq!(
                blocking_load32(&mut bus, RAM_BASE + 0x100 + (index as u64 * 4)),
                0x1000_0000 + index as u32
            );
        }
    }

    #[test]
    fn cache_maintenance_makes_cached_dma_buffers_visible() {
        let dma = Rc::new(RefCell::new(DmaController::new(DMA_BASE)));
        let mut memory = MemoryMap::new();
        memory
            .map_device(LatencyAdapter::new(Ram::new(RAM_BASE, 0x400), 2))
            .expect("RAM should map");
        memory
            .map_shared_device(Rc::clone(&dma))
            .expect("DMA should map");

        let mut fabric = ArbiterBus::new(memory);
        fabric.add_shared_master(Rc::clone(&dma));

        let l2 = DirectMappedCache::new(
            fabric,
            CacheConfig::new(16, vec![AddressRange::new(RAM_BASE, 0x400)])
                .with_line_size(16)
                .with_write_policy(WritePolicy::WriteBack)
                .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );
        let mut bus = SplitL1Cache::new(
            l2,
            CacheConfig::new(8, vec![]),
            CacheConfig::new(8, vec![AddressRange::new(RAM_BASE, 0x400)])
                .with_line_size(16)
                .with_write_policy(WritePolicy::WriteBack)
                .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );

        let source = RAM_BASE + 0x40;
        let destination = RAM_BASE + 0x80;
        for (index, word) in [0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]
            .into_iter()
            .enumerate()
        {
            blocking_store32(&mut bus, source + (index as u64 * 4), word);
        }

        for index in 0..4 {
            let source_word = {
                let memory = bus.inner_mut().inner_mut().inner_mut();
                blocking_load32(memory, source + (index as u64 * 4))
            };
            assert_eq!(source_word, 0);
            assert_eq!(
                blocking_load32(&mut bus, destination + (index as u64 * 4)),
                0
            );
        }

        blocking_write_back_range(&mut bus, source, 16);

        for (index, word) in [0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]
            .into_iter()
            .enumerate()
        {
            let source_word = {
                let memory = bus.inner_mut().inner_mut().inner_mut();
                blocking_load32(memory, source + (index as u64 * 4))
            };
            assert_eq!(source_word, word);
        }

        blocking_store32(
            &mut bus,
            DMA_BASE + DmaController::SOURCE_OFFSET,
            source as u32,
        );
        blocking_store32(
            &mut bus,
            DMA_BASE + DmaController::DESTINATION_OFFSET,
            destination as u32,
        );
        blocking_store32(&mut bus, DMA_BASE + DmaController::LENGTH_OFFSET, 4);
        blocking_store32(
            &mut bus,
            DMA_BASE + DmaController::CONTROL_OFFSET,
            DmaController::CONTROL_START,
        );

        for _ in 0..64 {
            bus.tick();
            if dma.borrow().is_done() {
                break;
            }
        }

        assert!(dma.borrow().is_done(), "{:?}", dma.borrow());
        blocking_invalidate_range(&mut bus, destination, 16);

        for (index, word) in [0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                blocking_load32(&mut bus, destination + (index as u64 * 4)),
                word
            );
        }
    }
}
