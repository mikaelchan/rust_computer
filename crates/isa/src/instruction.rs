//! Shared instruction containers passed between decode and execution.

use crate::{csr::CsrAddress, opcode::InstructionKind};

/// Raw instruction word fetched from memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstructionWord(pub u32);

/// A decoded RV32I instruction plus the fields needed by microarchitecture models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedInstruction {
    pub raw: InstructionWord,
    pub pc: u32,
    pub kind: InstructionKind,
    pub rd: Option<u8>,
    pub rs1: Option<u8>,
    pub rs2: Option<u8>,
    pub imm: i32,
    pub csr: Option<CsrAddress>,
}

impl DecodedInstruction {
    #[must_use]
    pub const fn new(
        raw: u32,
        pc: u32,
        kind: InstructionKind,
        rd: Option<u8>,
        rs1: Option<u8>,
        rs2: Option<u8>,
        imm: i32,
        csr: Option<CsrAddress>,
    ) -> Self {
        Self {
            raw: InstructionWord(raw),
            pc,
            kind,
            rd,
            rs1,
            rs2,
            imm,
            csr,
        }
    }
}
