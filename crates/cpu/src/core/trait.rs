use core::fmt;

use rvsim_isa::DecodeError;
use rvsim_system::BusError;

use crate::state::HartState;

/// Common inspection interface for all CPU models in this crate.
pub trait CpuModel {
    fn hart_state(&self) -> &HartState;
    fn hart_state_mut(&mut self) -> &mut HartState;
    fn model_name(&self) -> &'static str;
}

/// Fatal CPU errors that should stop the simulation.
#[derive(Debug)]
pub enum CpuError {
    Bus(BusError),
    Decode(DecodeError),
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus(error) => write!(f, "{error}"),
            Self::Decode(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CpuError {}

impl From<BusError> for CpuError {
    fn from(value: BusError) -> Self {
        Self::Bus(value)
    }
}

impl From<DecodeError> for CpuError {
    fn from(value: DecodeError) -> Self {
        Self::Decode(value)
    }
}
