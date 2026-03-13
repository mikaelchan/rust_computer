/// Structural hazard policy flags for the current machine configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructuralHazardPolicy {
    pub unified_memory: bool,
}

impl Default for StructuralHazardPolicy {
    fn default() -> Self {
        Self {
            unified_memory: true,
        }
    }
}

/// In a unified-memory machine, instruction fetch conflicts with data access in the same cycle.
#[must_use]
pub const fn fetch_blocked_by_memory_access(
    unified_memory: bool,
    memory_stage_accessed: bool,
) -> bool {
    unified_memory && memory_stage_accessed
}
