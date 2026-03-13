/// Minimal trace record emitted by the pipeline core.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineTrace {
    pub cycle: u64,
    pub fetched_pc: Option<u32>,
    pub decode_pc: Option<u32>,
    pub execute_pc: Option<u32>,
    pub memory_pc: Option<u32>,
    pub writeback_pc: Option<u32>,
    pub retired_instructions: u64,
    pub fetch_stalled: bool,
    pub decode_stalled: bool,
    pub flushed: bool,
    pub note: &'static str,
}
