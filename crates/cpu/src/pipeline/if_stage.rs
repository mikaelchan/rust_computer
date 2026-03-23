use rvsim_isa::Trap;
use rvsim_system::{Bus, BusError};

use crate::{
    exec::map_bus_error_to_trap,
    mmu::{MemoryAccess, PageWalker, TranslationResult},
    pipeline::latches::IfIdPayload,
    predictor::{BranchPrediction, BranchPredictor},
    state::{CsrFile, PrivilegeMode},
};

/// Outcome of one fetch attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchEvent {
    Advance(IfIdPayload),
    Stall,
    Trap(Trap),
}

/// Fetch one instruction from the bus instruction path.
pub fn fetch<P>(
    bus: &mut dyn Bus,
    walker: &mut PageWalker,
    csrs: &CsrFile,
    privilege: PrivilegeMode,
    pc: u32,
    predictor: &P,
) -> Result<FetchEvent, BusError>
where
    P: BranchPredictor + ?Sized,
{
    let physical_address =
        match walker.translate(bus, csrs, privilege, pc, MemoryAccess::Instruction)? {
            TranslationResult::PhysicalAddress(physical_address) => physical_address,
            TranslationResult::Stall => return Ok(FetchEvent::Stall),
            TranslationResult::Fault(trap) => return Ok(FetchEvent::Trap(trap)),
        };
    let raw = match bus.fetch32(u64::from(physical_address)) {
        Ok(raw) => raw,
        Err(BusError::Busy { .. }) => return Ok(FetchEvent::Stall),
        Err(error) => {
            if let Some(trap) = map_bus_error_to_trap(&error, pc, MemoryAccess::Instruction) {
                return Ok(FetchEvent::Trap(trap));
            }
            return Err(error);
        }
    };
    let prediction = predict_from_raw(pc, raw, predictor);
    Ok(FetchEvent::Advance(IfIdPayload {
        pc,
        raw,
        predicted_pc: prediction.target,
        predicted_taken: prediction.taken,
    }))
}

fn predict_from_raw<P>(pc: u32, raw: u32, predictor: &P) -> BranchPrediction
where
    P: BranchPredictor + ?Sized,
{
    let fallthrough = pc.wrapping_add(4);

    match raw & 0x7f {
        0x63 => {
            let target = pc.wrapping_add_signed(imm_b(raw));
            predictor.predict(pc, fallthrough, target)
        }
        0x6f => BranchPrediction {
            taken: true,
            target: pc.wrapping_add_signed(imm_j(raw)),
        },
        _ => BranchPrediction {
            taken: false,
            target: fallthrough,
        },
    }
}

const fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

const fn imm_b(raw: u32) -> i32 {
    let imm = (((raw >> 8) & 0x0f) << 1)
        | (((raw >> 25) & 0x3f) << 5)
        | (((raw >> 7) & 0x01) << 11)
        | (((raw >> 31) & 0x01) << 12);
    sign_extend(imm, 13)
}

const fn imm_j(raw: u32) -> i32 {
    let imm = (((raw >> 21) & 0x03ff) << 1)
        | (((raw >> 20) & 0x0001) << 11)
        | (((raw >> 12) & 0x00ff) << 12)
        | (((raw >> 31) & 0x0001) << 20);
    sign_extend(imm, 21)
}
