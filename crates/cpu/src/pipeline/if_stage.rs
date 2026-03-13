use rvsim_system::{Bus, BusError};

use crate::pipeline::latches::IfIdPayload;

/// Fetch one instruction from the unified memory bus.
pub fn fetch(bus: &mut dyn Bus, pc: u32, predicted_pc: u32) -> Result<IfIdPayload, BusError> {
    let raw = bus.load32(u64::from(pc))?;
    Ok(IfIdPayload {
        pc,
        raw,
        predicted_pc,
    })
}
