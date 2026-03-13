mod bimodal_2bit;
mod static_predictor;

pub use bimodal_2bit::BimodalPredictor;
pub use static_predictor::AlwaysNotTaken;

/// Prediction output consumed by the front end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BranchPrediction {
    pub taken: bool,
    pub target: u32,
}

/// Shared interface for branch predictors.
pub trait BranchPredictor {
    fn predict(&self, pc: u32, fallthrough: u32, target: u32) -> BranchPrediction;
    fn update(&mut self, pc: u32, taken: bool);
}
