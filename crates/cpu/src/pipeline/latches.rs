use rvsim_isa::DecodedInstruction;

/// IF/ID latch state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IfIdLatch {
    pub pc: Option<u32>,
    pub raw: Option<u32>,
}

/// ID/EX latch state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdExLatch {
    pub decoded: Option<DecodedInstruction>,
}

/// EX/MEM latch state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExMemLatch {
    pub destination: Option<u8>,
    pub alu_result: Option<u32>,
    pub memory_access: bool,
}

/// MEM/WB latch state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemWbLatch {
    pub destination: Option<u8>,
    pub value: Option<u32>,
}

/// Full latch set for the five-stage pipeline scaffold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineLatches {
    pub if_id: IfIdLatch,
    pub id_ex: IdExLatch,
    pub ex_mem: ExMemLatch,
    pub mem_wb: MemWbLatch,
}
