use rvsim_system::{
    Address, AddressRange, Addressable, BusError, BusMaster, BusMasterRequest, BusMasterResponse,
    InterruptLine, InterruptSet,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum DmaPhase {
    Read,
    Write { word: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveTransfer {
    source: Address,
    destination: Address,
    remaining_words: u32,
    phase: DmaPhase,
}

/// A simple memory-to-memory DMA engine with one in-flight transfer.
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
            source: u64::from(self.source),
            destination: u64::from(self.destination),
            remaining_words: self.length_words,
            phase: DmaPhase::Read,
        });
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

    fn request(&mut self) -> Option<BusMasterRequest> {
        let active = self.active.as_ref()?;
        Some(match active.phase {
            DmaPhase::Read => BusMasterRequest::Load32 {
                addr: active.source,
            },
            DmaPhase::Write { word } => BusMasterRequest::Store32 {
                addr: active.destination,
                value: word,
            },
        })
    }

    fn on_response(&mut self, response: Result<BusMasterResponse, BusError>) {
        let Some(mut active) = self.active.take() else {
            return;
        };

        match response {
            Ok(BusMasterResponse::Load32(word)) => {
                active.phase = DmaPhase::Write { word };
                self.active = Some(active);
            }
            Ok(BusMasterResponse::StoreComplete) => {
                active.source += 4;
                active.destination += 4;
                active.remaining_words -= 1;
                self.transferred_words += 1;

                if active.remaining_words == 0 {
                    self.done = true;
                } else {
                    active.phase = DmaPhase::Read;
                    self.active = Some(active);
                }
            }
            Err(_) => {
                self.error = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use rvsim_system::{ArbiterBus, Bus, InterruptLine, MemoryMap};

    use crate::Ram;

    use super::DmaController;

    const RAM_BASE: u64 = 0x1000_0000;
    const DMA_BASE: u64 = 0x4000_0000;

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

        for _ in 0..8 {
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
        assert!(dma.borrow().is_done());
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
}
