use rvsim_isa::{AluOp, DecodedInstruction, Exception, SystemKind, Trap, opcode::InstructionKind};

use crate::{
    exec::{alu, branch, csr, load_store},
    state::CsrFile,
};

/// Result produced by the execute stage before any memory access occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecuteOutcome {
    pub writeback_value: Option<u32>,
    pub csr_write: Option<csr::CsrWrite>,
    pub memory_address: Option<u32>,
    pub store_value: u32,
    pub next_pc: u32,
}

/// Execute-stage completion status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecuteEvent {
    Advance(ExecuteOutcome),
    Trap(Trap),
}

/// Return whether the decoded instruction reads memory in the MEM stage.
#[must_use]
pub const fn uses_memory(decoded: DecodedInstruction) -> bool {
    matches!(
        decoded.kind,
        InstructionKind::Load(_) | InstructionKind::Store(_)
    )
}

/// Return whether the decoded instruction is a load.
#[must_use]
pub const fn is_load(decoded: DecodedInstruction) -> bool {
    matches!(decoded.kind, InstructionKind::Load(_))
}

/// Return whether the decoded instruction writes a general-purpose register.
#[must_use]
pub fn writes_back(decoded: DecodedInstruction) -> bool {
    matches!(
        decoded.kind,
        InstructionKind::Lui
            | InstructionKind::Auipc
            | InstructionKind::Jal
            | InstructionKind::Jalr
            | InstructionKind::Load(_)
            | InstructionKind::OpImm(_)
            | InstructionKind::Op(_)
            | InstructionKind::Csr(_)
    ) && decoded.rd.is_some()
        && decoded.rd != Some(0)
}

/// Execute one instruction using forwarded operands.
#[must_use]
pub fn execute(
    decoded: DecodedInstruction,
    rs1_value: u32,
    rs2_value: u32,
    mepc: u32,
    csrs: &CsrFile,
) -> ExecuteEvent {
    let next_pc = decoded.pc.wrapping_add(4);

    match decoded.kind {
        InstructionKind::Lui => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: Some(decoded.imm as u32),
            csr_write: None,
            memory_address: None,
            store_value: 0,
            next_pc,
        }),
        InstructionKind::Auipc => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: Some(decoded.pc.wrapping_add_signed(decoded.imm)),
            csr_write: None,
            memory_address: None,
            store_value: 0,
            next_pc,
        }),
        InstructionKind::Jal => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: Some(next_pc),
            csr_write: None,
            memory_address: None,
            store_value: 0,
            next_pc: branch::branch_target(decoded.pc, decoded.imm),
        }),
        InstructionKind::Jalr => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: Some(next_pc),
            csr_write: None,
            memory_address: None,
            store_value: 0,
            next_pc: load_store::effective_address(rs1_value, decoded.imm) & !1,
        }),
        InstructionKind::Branch(branch_kind) => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: None,
            csr_write: None,
            memory_address: None,
            store_value: 0,
            next_pc: if branch::branch_taken(branch_kind, rs1_value, rs2_value) {
                branch::branch_target(decoded.pc, decoded.imm)
            } else {
                next_pc
            },
        }),
        InstructionKind::Load(_) => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: None,
            csr_write: None,
            memory_address: Some(load_store::effective_address(rs1_value, decoded.imm)),
            store_value: 0,
            next_pc,
        }),
        InstructionKind::Store(_) => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: None,
            csr_write: None,
            memory_address: Some(load_store::effective_address(rs1_value, decoded.imm)),
            store_value: rs2_value,
            next_pc,
        }),
        InstructionKind::OpImm(op) => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: Some(alu::execute_alu(op, rs1_value, immediate_rhs(op, decoded))),
            csr_write: None,
            memory_address: None,
            store_value: 0,
            next_pc,
        }),
        InstructionKind::Op(op) => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: Some(alu::execute_alu(op, rs1_value, rs2_value)),
            csr_write: None,
            memory_address: None,
            store_value: 0,
            next_pc,
        }),
        InstructionKind::Csr(_op) => {
            let outcome = csr::execute(decoded, csrs, rs1_value)
                .expect("csr instruction should provide an address");
            ExecuteEvent::Advance(ExecuteOutcome {
                writeback_value: Some(outcome.read_value),
                csr_write: outcome.write,
                memory_address: None,
                store_value: 0,
                next_pc,
            })
        }
        InstructionKind::System(SystemKind::Ecall) => {
            ExecuteEvent::Trap(Trap::Exception(Exception::EnvironmentCallFromMMode))
        }
        InstructionKind::System(SystemKind::Ebreak) => {
            ExecuteEvent::Trap(Trap::Exception(Exception::Breakpoint))
        }
        InstructionKind::System(SystemKind::Mret) => ExecuteEvent::Advance(ExecuteOutcome {
            writeback_value: None,
            csr_write: None,
            memory_address: None,
            store_value: 0,
            next_pc: mepc,
        }),
    }
}

const fn immediate_rhs(op: AluOp, decoded: DecodedInstruction) -> u32 {
    if matches!(op, AluOp::Sll | AluOp::Srl | AluOp::Sra) {
        decoded.raw.0 >> 20
    } else {
        decoded.imm as u32
    }
}
