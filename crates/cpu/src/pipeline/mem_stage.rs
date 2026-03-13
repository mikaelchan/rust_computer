/// Summary of whether the memory stage touched the bus this cycle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryStageStatus {
    pub accessed_memory: bool,
}
