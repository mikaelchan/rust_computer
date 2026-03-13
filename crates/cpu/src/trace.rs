/// Minimal trace record emitted by the pipeline core.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineTrace {
    pub cycle: u64,
    pub fetched_pc: Option<u32>,
    pub retired_instructions: u64,
    pub note: &'static str,
}
