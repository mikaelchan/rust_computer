//! Unified bus traits for the first von Neumann machine model.

use core::fmt;

/// Physical address used across the computer model.
pub type Address = u64;

/// Interrupt lines exposed by memory-mapped devices to the CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptLine {
    MachineTimer,
    MachineExternal,
}

/// Aggregate interrupt state visible on the unified bus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterruptSet {
    bits: u8,
}

impl InterruptSet {
    const MACHINE_TIMER_BIT: u8 = 1 << 0;
    const MACHINE_EXTERNAL_BIT: u8 = 1 << 1;

    #[must_use]
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    #[must_use]
    pub const fn from_line(line: InterruptLine) -> Self {
        let bits = match line {
            InterruptLine::MachineTimer => Self::MACHINE_TIMER_BIT,
            InterruptLine::MachineExternal => Self::MACHINE_EXTERNAL_BIT,
        };

        Self { bits }
    }

    #[must_use]
    pub const fn contains(self, line: InterruptLine) -> bool {
        let mask = match line {
            InterruptLine::MachineTimer => Self::MACHINE_TIMER_BIT,
            InterruptLine::MachineExternal => Self::MACHINE_EXTERNAL_BIT,
        };

        (self.bits & mask) != 0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    #[must_use]
    pub const fn highest_priority(self) -> Option<InterruptLine> {
        if self.contains(InterruptLine::MachineExternal) {
            Some(InterruptLine::MachineExternal)
        } else if self.contains(InterruptLine::MachineTimer) {
            Some(InterruptLine::MachineTimer)
        } else {
            None
        }
    }
}

impl From<InterruptLine> for InterruptSet {
    fn from(value: InterruptLine) -> Self {
        Self::from_line(value)
    }
}

/// A half-open physical address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressRange {
    pub start: Address,
    pub size: u64,
}

impl AddressRange {
    #[must_use]
    pub const fn new(start: Address, size: u64) -> Self {
        Self { start, size }
    }

    #[must_use]
    pub const fn end(self) -> Address {
        self.start + self.size
    }

    #[must_use]
    pub const fn contains(self, addr: Address) -> bool {
        addr >= self.start && addr < self.end()
    }

    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end() && other.start < self.end()
    }
}

/// Bus-level failures exposed by devices and memory maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusError {
    UnmappedAddress { addr: Address },
    MisalignedAccess { addr: Address, width: usize },
    ReadOnlyAddress { addr: Address },
    DeviceFault { addr: Address, message: String },
}

impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmappedAddress { addr } => write!(f, "unmapped address 0x{addr:08x}"),
            Self::MisalignedAccess { addr, width } => {
                write!(f, "misaligned access at 0x{addr:08x} for width {width}")
            }
            Self::ReadOnlyAddress { addr } => write!(f, "read-only address 0x{addr:08x}"),
            Self::DeviceFault { addr, message } => {
                write!(f, "device fault at 0x{addr:08x}: {message}")
            }
        }
    }
}

impl std::error::Error for BusError {}

/// A memory-mapped device that can respond to byte loads and stores.
pub trait Addressable {
    fn name(&self) -> &'static str;
    fn address_range(&self) -> AddressRange;

    fn reset(&mut self) {}
    fn tick(&mut self) {}
    fn pending_interrupts(&self) -> InterruptSet {
        InterruptSet::empty()
    }
    fn pending_interrupt(&self) -> Option<InterruptLine> {
        self.pending_interrupts().highest_priority()
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError>;
    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError>;
}

/// A byte-addressable bus with default helpers for larger accesses.
pub trait Bus {
    fn load8(&mut self, addr: Address) -> Result<u8, BusError>;
    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError>;
    fn tick(&mut self) {}
    fn pending_interrupts(&self) -> InterruptSet {
        InterruptSet::empty()
    }
    fn pending_interrupt(&self) -> Option<InterruptLine> {
        self.pending_interrupts().highest_priority()
    }

    fn load16(&mut self, addr: Address) -> Result<u16, BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        let b0 = self.load8(addr)?;
        let b1 = self.load8(addr + 1)?;
        Ok(u16::from_le_bytes([b0, b1]))
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        let b0 = self.load8(addr)?;
        let b1 = self.load8(addr + 1)?;
        let b2 = self.load8(addr + 2)?;
        let b3 = self.load8(addr + 3)?;
        Ok(u32::from_le_bytes([b0, b1, b2, b3]))
    }

    fn store16(&mut self, addr: Address, value: u16) -> Result<(), BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.store8(addr + offset as u64, byte)?;
        }
        Ok(())
    }

    fn store32(&mut self, addr: Address, value: u32) -> Result<(), BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.store8(addr + offset as u64, byte)?;
        }
        Ok(())
    }
}
