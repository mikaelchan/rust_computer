use rvsim_system::{Bus, BusError};

/// Output of the fetch stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchOutput {
    pub pc: u32,
    pub raw: u32,
    pub predicted_pc: u32,
}

/// Fetch one instruction from the unified memory bus.
pub fn fetch(bus: &mut dyn Bus, pc: u32, predicted_pc: u32) -> Result<FetchOutput, BusError> {
    let raw = bus.load32(u64::from(pc))?;
    Ok(FetchOutput {
        pc,
        raw,
        predicted_pc,
    })
}
