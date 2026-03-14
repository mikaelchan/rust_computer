//! Unified bus traits for the first von Neumann machine model.

use core::fmt;

/// Physical address used across the computer model.
pub type Address = u64;

/// Direction of a bus access used by timing models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Fetch,
    Load,
    Store,
}

/// Interrupt lines exposed by memory-mapped devices to the CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptLine {
    MachineSoftware,
    MachineTimer,
    MachineExternal,
}

/// Aggregate interrupt state visible on the unified bus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterruptSet {
    bits: u8,
}

impl InterruptSet {
    const MACHINE_SOFTWARE_BIT: u8 = 1 << 0;
    const MACHINE_TIMER_BIT: u8 = 1 << 1;
    const MACHINE_EXTERNAL_BIT: u8 = 1 << 2;

    #[must_use]
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    #[must_use]
    pub const fn from_line(line: InterruptLine) -> Self {
        let bits = match line {
            InterruptLine::MachineSoftware => Self::MACHINE_SOFTWARE_BIT,
            InterruptLine::MachineTimer => Self::MACHINE_TIMER_BIT,
            InterruptLine::MachineExternal => Self::MACHINE_EXTERNAL_BIT,
        };

        Self { bits }
    }

    #[must_use]
    pub const fn contains(self, line: InterruptLine) -> bool {
        let mask = match line {
            InterruptLine::MachineSoftware => Self::MACHINE_SOFTWARE_BIT,
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
        } else if self.contains(InterruptLine::MachineSoftware) {
            Some(InterruptLine::MachineSoftware)
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
    Busy { remaining_cycles: u32 },
    UnmappedAddress { addr: Address },
    MisalignedAccess { addr: Address, width: usize },
    ReadOnlyAddress { addr: Address },
    DeviceFault { addr: Address, message: String },
}

impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy { remaining_cycles } => {
                write!(f, "bus busy for {remaining_cycles} more cycles")
            }
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

/// A transaction request accepted by the timing-aware bus fabric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionRequest {
    pub addr: Address,
    pub kind: AccessKind,
    pub width: usize,
    pub write_data: [u8; 4],
}

impl TransactionRequest {
    #[must_use]
    pub const fn fetch32(addr: Address) -> Self {
        Self {
            addr,
            kind: AccessKind::Fetch,
            width: 4,
            write_data: [0; 4],
        }
    }

    #[must_use]
    pub const fn load(addr: Address, width: usize) -> Self {
        Self {
            addr,
            kind: AccessKind::Load,
            width,
            write_data: [0; 4],
        }
    }

    #[must_use]
    pub const fn store(addr: Address, width: usize, write_data: [u8; 4]) -> Self {
        Self {
            addr,
            kind: AccessKind::Store,
            width,
            write_data,
        }
    }
}

/// Response payload produced by a completed transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionResponse {
    Read { data: [u8; 4], width: usize },
    WriteComplete,
}

impl TransactionResponse {
    #[must_use]
    pub const fn read_u32(self) -> Option<u32> {
        match self {
            Self::Read { data, width: 4 } => Some(u32::from_le_bytes(data)),
            _ => None,
        }
    }
}

/// Observable lifecycle state for an outstanding bus transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionPhase {
    Accepted,
    InFlight { remaining_cycles: u32 },
    Ready(TransactionResponse),
    Failed(BusError),
}

/// A beat-oriented burst request over contiguous 32-bit words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BurstRequest {
    ReadWords {
        base_addr: Address,
        beats: usize,
        kind: AccessKind,
    },
    WriteWords {
        base_addr: Address,
        words: Box<[u32]>,
    },
}

impl BurstRequest {
    #[must_use]
    pub const fn read_words(base_addr: Address, beats: usize, kind: AccessKind) -> Self {
        Self::ReadWords {
            base_addr,
            beats,
            kind,
        }
    }

    #[must_use]
    pub fn write_words(base_addr: Address, words: Box<[u32]>) -> Self {
        Self::WriteWords { base_addr, words }
    }

    #[must_use]
    pub const fn beats(&self) -> usize {
        match self {
            Self::ReadWords { beats, .. } => *beats,
            Self::WriteWords { words, .. } => words.len(),
        }
    }

    #[must_use]
    pub const fn base_addr(&self) -> Address {
        match self {
            Self::ReadWords { base_addr, .. } | Self::WriteWords { base_addr, .. } => *base_addr,
        }
    }

    #[must_use]
    pub const fn beat_addr(&self, beat_index: usize) -> Address {
        self.base_addr() + (beat_index as u64 * 4)
    }

    #[must_use]
    pub const fn beat_kind(&self) -> AccessKind {
        match self {
            Self::ReadWords { kind, .. } => *kind,
            Self::WriteWords { .. } => AccessKind::Store,
        }
    }
}

/// Response payload produced by a completed burst request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BurstResponse {
    ReadWords(Box<[u32]>),
    WriteComplete { beats: usize },
}

/// Observable lifecycle state for an outstanding burst.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BurstPhase {
    Accepted {
        beat_index: usize,
        total_beats: usize,
    },
    InFlight {
        beat_index: usize,
        total_beats: usize,
        remaining_cycles: u32,
    },
    Ready {
        completed_beats: usize,
    },
    Failed(BusError),
}

/// A single bus transaction issued by a non-CPU bus master.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusMasterRequest {
    Load32 { addr: Address },
    Store32 { addr: Address, value: u32 },
}

/// Completion value returned to a non-CPU bus master.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusMasterResponse {
    Load32(u32),
    StoreComplete,
}

/// A peripheral-side initiator that can compete with the CPU for bus access.
pub trait BusMaster {
    fn name(&self) -> &'static str;
    fn request(&mut self) -> Option<BusMasterRequest>;
    fn on_response(&mut self, response: Result<BusMasterResponse, BusError>);
}

/// A memory-mapped device that can respond to byte loads and stores.
pub trait Addressable {
    fn name(&self) -> &'static str;
    fn address_range(&self) -> AddressRange;

    fn reset(&mut self) {}
    fn tick(&mut self) {}
    fn access_latency(&self, _addr: Address, _kind: AccessKind, _width: usize) -> u32 {
        0
    }
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
    fn reset(&mut self) {}
    fn tick(&mut self) {}
    fn is_busy(&self) -> bool {
        false
    }
    fn pending_interrupts(&self) -> InterruptSet {
        InterruptSet::empty()
    }
    fn pending_interrupt(&self) -> Option<InterruptLine> {
        self.pending_interrupts().highest_priority()
    }

    fn fetch32(&mut self, addr: Address) -> Result<u32, BusError> {
        self.load32(addr)
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

/// A burst-capable bus that exposes an explicit burst lifecycle.
pub trait BurstBus: Bus {
    fn submit_burst(&mut self, request: BurstRequest) -> Result<u64, BusError>;
    fn burst_phase(&self, id: u64) -> Option<BurstPhase>;
    fn advance_burst(&mut self, id: u64) -> Option<BurstPhase>;
    fn take_burst_response(&mut self, id: u64) -> Option<Result<BurstResponse, BusError>>;
}
