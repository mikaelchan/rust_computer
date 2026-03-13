use rvsim_isa::{DecodedInstruction, opcode::InstructionKind};

use crate::exec::{branch, load_store};

/// Metadata produced by execute-stage classification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecuteMetadata {
    pub memory_access: bool,
    pub branch_target: Option<u32>,
}

/// Compute branch targets and identify whether a decoded instruction uses the memory stage.
#[must_use]
pub fn classify(decoded: DecodedInstruction, rs1_value: u32, rs2_value: u32) -> ExecuteMetadata {
    match decoded.kind {
        InstructionKind::Branch(kind) => ExecuteMetadata {
            memory_access: false,
            branch_target: branch::branch_taken(kind, rs1_value, rs2_value)
                .then(|| branch::branch_target(decoded.pc, decoded.imm)),
        },
        InstructionKind::Jal => ExecuteMetadata {
            memory_access: false,
            branch_target: Some(branch::branch_target(decoded.pc, decoded.imm)),
        },
        InstructionKind::Jalr => ExecuteMetadata {
            memory_access: false,
            branch_target: Some(load_store::effective_address(rs1_value, decoded.imm) & !1),
        },
        InstructionKind::Load(_) | InstructionKind::Store(_) => ExecuteMetadata {
            memory_access: true,
            branch_target: None,
        },
        _ => ExecuteMetadata::default(),
    }
}
