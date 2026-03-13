use super::{BranchPrediction, BranchPredictor};

/// Always predict not-taken. Good enough for the first CPU milestone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlwaysNotTaken;

impl BranchPredictor for AlwaysNotTaken {
    fn predict(&self, _pc: u32, fallthrough: u32, _target: u32) -> BranchPrediction {
        BranchPrediction {
            taken: false,
            target: fallthrough,
        }
    }

    fn update(&mut self, _pc: u32, _taken: bool) {}
}
