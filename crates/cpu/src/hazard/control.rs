/// Control hazard status for a branch resolution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControlHazardStatus {
    pub flush_required: bool,
}

/// Flush the pipeline when the predicted and actual next PCs diverge.
#[must_use]
pub fn detect_branch_flush(predicted_pc: u32, actual_pc: u32) -> ControlHazardStatus {
    ControlHazardStatus {
        flush_required: predicted_pc != actual_pc,
    }
}
