//! Shared component traits for clocked simulation.

use crate::bus::Bus;

/// Minimal per-cycle status returned by a processor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuCycle {
    pub retired_instructions: u64,
    pub stalled: bool,
}

/// A simulation object that can be reset to its power-on state.
pub trait SimComponent {
    fn reset(&mut self);
}

/// A processor that advances its architectural state one cycle at a time.
pub trait Processor: SimComponent {
    type Error;

    fn cycle(&self) -> u64;
    fn step_cycle(&mut self, bus: &mut dyn Bus) -> Result<CpuCycle, Self::Error>;
}
