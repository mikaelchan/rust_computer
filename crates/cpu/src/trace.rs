use rvsim_isa::{InstructionKind, Trap};

/// A committed architectural update observed at the end of a cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitEvent {
    pub pc: u32,
    pub next_pc: u32,
    pub kind: InstructionKind,
    pub destination: Option<u8>,
    pub value: Option<u32>,
}

/// Why the front end was flushed this cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushReason {
    BranchRedirect,
    Trap,
    ReturnFromTrap,
}

/// Cumulative pipeline counters useful for later benchmarking and debugging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineStats {
    pub cycles: u64,
    pub retired_instructions: u64,
    pub fetch_stall_cycles: u64,
    pub decode_stall_cycles: u64,
    pub flush_cycles: u64,
    pub branch_flushes: u64,
    pub trap_flushes: u64,
    pub return_flushes: u64,
    pub predicted_taken_fetches: u64,
    pub trap_count: u64,
}

/// Trace record emitted by the pipeline core for one cycle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineTrace {
    pub cycle: u64,
    pub fetched_pc: Option<u32>,
    pub decode_pc: Option<u32>,
    pub execute_pc: Option<u32>,
    pub memory_pc: Option<u32>,
    pub writeback_pc: Option<u32>,
    pub commit: Option<CommitEvent>,
    pub trap: Option<Trap>,
    pub flush_reason: Option<FlushReason>,
    pub retired_instructions: u64,
    pub predicted_taken: bool,
    pub fetch_stalled: bool,
    pub decode_stalled: bool,
    pub flushed: bool,
    pub note: &'static str,
}
