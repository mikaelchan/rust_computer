use rvsim_isa::DecodedInstruction;

/// IF-stage payload carried into decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfIdPayload {
    pub pc: u32,
    pub raw: u32,
    pub predicted_pc: u32,
    pub predicted_taken: bool,
}

/// IF/ID latch state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IfIdLatch {
    pub payload: Option<IfIdPayload>,
}

/// ID-stage payload carried into execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdExPayload {
    pub decoded: DecodedInstruction,
    pub rs1_value: u32,
    pub rs2_value: u32,
    pub predicted_pc: u32,
    pub predicted_taken: bool,
}

/// ID/EX latch state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdExLatch {
    pub payload: Option<IdExPayload>,
}

/// EX-stage payload carried into memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExMemPayload {
    pub decoded: DecodedInstruction,
    pub writeback_value: Option<u32>,
    pub memory_address: Option<u32>,
    pub store_value: u32,
    pub next_pc: u32,
}

/// EX/MEM latch state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExMemLatch {
    pub payload: Option<ExMemPayload>,
}

/// MEM-stage payload carried into writeback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemWbPayload {
    pub decoded: DecodedInstruction,
    pub writeback_value: Option<u32>,
    pub next_pc: u32,
}

/// MEM/WB latch state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemWbLatch {
    pub payload: Option<MemWbPayload>,
}

/// Full latch set for the five-stage pipeline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineLatches {
    pub if_id: IfIdLatch,
    pub id_ex: IdExLatch,
    pub ex_mem: ExMemLatch,
    pub mem_wb: MemWbLatch,
}
