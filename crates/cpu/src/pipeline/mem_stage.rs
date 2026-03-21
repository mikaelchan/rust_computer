use rvsim_isa::{Exception, LoadKind, StoreKind, Trap, opcode::InstructionKind};
use rvsim_system::{Bus, BusError};

use crate::{
    exec::load_store,
    mmu::{MemoryAccess, PageWalker, TranslationResult},
    pipeline::latches::{ExMemPayload, MemWbPayload},
    state::{CsrFile, PrivilegeMode},
};

/// Outcome of the memory stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryEvent {
    Advance(MemWbPayload),
    Stall(ExMemPayload),
    Trap(Trap),
}

/// Perform memory access or pass through a non-memory instruction.
pub fn access(
    bus: &mut dyn Bus,
    walker: &mut PageWalker,
    csrs: &CsrFile,
    privilege: PrivilegeMode,
    payload: ExMemPayload,
) -> Result<MemoryEvent, BusError> {
    match payload.decoded.kind {
        InstructionKind::Load(load_kind) => {
            let virtual_address = payload.memory_address.unwrap_or_default();
            let physical_address = match walker.translate(
                bus,
                csrs,
                privilege,
                virtual_address,
                MemoryAccess::Load,
            )? {
                TranslationResult::PhysicalAddress(physical_address) => physical_address,
                TranslationResult::Stall => return Ok(MemoryEvent::Stall(payload)),
                TranslationResult::PageFault(trap) => return Ok(MemoryEvent::Trap(trap)),
            };
            match load_value(bus, load_kind, u64::from(physical_address)) {
                Ok(value) => Ok(MemoryEvent::Advance(MemWbPayload {
                    decoded: payload.decoded,
                    writeback_value: Some(value),
                    csr_write: payload.csr_write,
                    next_pc: payload.next_pc,
                })),
                Err(BusError::Busy { .. }) => Ok(MemoryEvent::Stall(payload)),
                Err(BusError::MisalignedAccess { .. }) => Ok(MemoryEvent::Trap(Trap::Exception(
                    Exception::LoadAddressMisaligned {
                        addr: virtual_address,
                    },
                ))),
                Err(error) => Err(error),
            }
        }
        InstructionKind::Store(store_kind) => {
            let virtual_address = payload.memory_address.unwrap_or_default();
            let physical_address = match walker.translate(
                bus,
                csrs,
                privilege,
                virtual_address,
                MemoryAccess::Store,
            )? {
                TranslationResult::PhysicalAddress(physical_address) => physical_address,
                TranslationResult::Stall => return Ok(MemoryEvent::Stall(payload)),
                TranslationResult::PageFault(trap) => return Ok(MemoryEvent::Trap(trap)),
            };
            match store_value(
                bus,
                store_kind,
                u64::from(physical_address),
                payload.store_value,
            ) {
                Ok(()) => Ok(MemoryEvent::Advance(MemWbPayload {
                    decoded: payload.decoded,
                    writeback_value: None,
                    csr_write: payload.csr_write,
                    next_pc: payload.next_pc,
                })),
                Err(BusError::Busy { .. }) => Ok(MemoryEvent::Stall(payload)),
                Err(BusError::MisalignedAccess { .. }) => Ok(MemoryEvent::Trap(Trap::Exception(
                    Exception::StoreAddressMisaligned {
                        addr: virtual_address,
                    },
                ))),
                Err(error) => Err(error),
            }
        }
        _ => Ok(MemoryEvent::Advance(MemWbPayload {
            decoded: payload.decoded,
            writeback_value: payload.writeback_value,
            csr_write: payload.csr_write,
            next_pc: payload.next_pc,
        })),
    }
}

fn load_value(bus: &mut dyn Bus, kind: LoadKind, address: u64) -> Result<u32, BusError> {
    match kind {
        LoadKind::Byte => Ok(load_store::sign_extend_byte(bus.load8(address)?)),
        LoadKind::Half => Ok(load_store::sign_extend_half(bus.load16(address)?)),
        LoadKind::Word => bus.load32(address),
        LoadKind::ByteUnsigned => Ok(bus.load8(address)? as u32),
        LoadKind::HalfUnsigned => Ok(bus.load16(address)? as u32),
    }
}

fn store_value(
    bus: &mut dyn Bus,
    kind: StoreKind,
    address: u64,
    value: u32,
) -> Result<(), BusError> {
    match kind {
        StoreKind::Byte => bus.store8(address, value as u8),
        StoreKind::Half => bus.store16(address, value as u16),
        StoreKind::Word => bus.store32(address, value),
    }
}
