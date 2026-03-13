//! Instruction decoding for the first RV32I milestone.

use core::fmt;

use crate::{
    instruction::DecodedInstruction,
    opcode::{AluOp, BranchKind, InstructionKind, LoadKind, StoreKind, SystemKind},
};

/// Returned when the decoder encounters an instruction outside the current model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError {
    raw: u32,
}

impl DecodeError {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self { raw }
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.raw
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported instruction 0x{:08x}", self.raw)
    }
}

impl std::error::Error for DecodeError {}

/// Decode a 32-bit RV32I instruction at the given program counter.
pub fn decode(raw: u32, pc: u32) -> Result<DecodedInstruction, DecodeError> {
    let opcode = (raw & 0x7f) as u8;
    let rd = ((raw >> 7) & 0x1f) as u8;
    let funct3 = ((raw >> 12) & 0x7) as u8;
    let rs1 = ((raw >> 15) & 0x1f) as u8;
    let rs2 = ((raw >> 20) & 0x1f) as u8;
    let funct7 = ((raw >> 25) & 0x7f) as u8;

    match opcode {
        0x37 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::Lui,
            Some(rd),
            None,
            None,
            imm_u(raw),
            None,
        )),
        0x17 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::Auipc,
            Some(rd),
            None,
            None,
            imm_u(raw),
            None,
        )),
        0x6f => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::Jal,
            Some(rd),
            None,
            None,
            imm_j(raw),
            None,
        )),
        0x67 if funct3 == 0 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::Jalr,
            Some(rd),
            Some(rs1),
            None,
            imm_i(raw),
            None,
        )),
        0x63 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::Branch(decode_branch(funct3, raw)?),
            None,
            Some(rs1),
            Some(rs2),
            imm_b(raw),
            None,
        )),
        0x03 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::Load(decode_load(funct3, raw)?),
            Some(rd),
            Some(rs1),
            None,
            imm_i(raw),
            None,
        )),
        0x23 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::Store(decode_store(funct3, raw)?),
            None,
            Some(rs1),
            Some(rs2),
            imm_s(raw),
            None,
        )),
        0x13 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::OpImm(decode_op_imm(funct3, funct7, raw)?),
            Some(rd),
            Some(rs1),
            None,
            imm_i(raw),
            None,
        )),
        0x33 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::Op(decode_op(funct3, funct7, raw)?),
            Some(rd),
            Some(rs1),
            Some(rs2),
            0,
            None,
        )),
        0x73 => Ok(DecodedInstruction::new(
            raw,
            pc,
            InstructionKind::System(decode_system(raw)?),
            None,
            None,
            None,
            0,
            None,
        )),
        _ => Err(DecodeError::new(raw)),
    }
}

fn decode_branch(funct3: u8, raw: u32) -> Result<BranchKind, DecodeError> {
    match funct3 {
        0b000 => Ok(BranchKind::Beq),
        0b001 => Ok(BranchKind::Bne),
        0b100 => Ok(BranchKind::Blt),
        0b101 => Ok(BranchKind::Bge),
        0b110 => Ok(BranchKind::Bltu),
        0b111 => Ok(BranchKind::Bgeu),
        _ => Err(DecodeError::new(raw)),
    }
}

fn decode_load(funct3: u8, raw: u32) -> Result<LoadKind, DecodeError> {
    match funct3 {
        0b000 => Ok(LoadKind::Byte),
        0b001 => Ok(LoadKind::Half),
        0b010 => Ok(LoadKind::Word),
        0b100 => Ok(LoadKind::ByteUnsigned),
        0b101 => Ok(LoadKind::HalfUnsigned),
        _ => Err(DecodeError::new(raw)),
    }
}

fn decode_store(funct3: u8, raw: u32) -> Result<StoreKind, DecodeError> {
    match funct3 {
        0b000 => Ok(StoreKind::Byte),
        0b001 => Ok(StoreKind::Half),
        0b010 => Ok(StoreKind::Word),
        _ => Err(DecodeError::new(raw)),
    }
}

fn decode_op_imm(funct3: u8, funct7: u8, raw: u32) -> Result<AluOp, DecodeError> {
    match (funct3, funct7) {
        (0b000, _) => Ok(AluOp::Add),
        (0b010, _) => Ok(AluOp::Slt),
        (0b011, _) => Ok(AluOp::Sltu),
        (0b100, _) => Ok(AluOp::Xor),
        (0b110, _) => Ok(AluOp::Or),
        (0b111, _) => Ok(AluOp::And),
        (0b001, 0b0000000) => Ok(AluOp::Sll),
        (0b101, 0b0000000) => Ok(AluOp::Srl),
        (0b101, 0b0100000) => Ok(AluOp::Sra),
        _ => Err(DecodeError::new(raw)),
    }
}

fn decode_op(funct3: u8, funct7: u8, raw: u32) -> Result<AluOp, DecodeError> {
    match (funct3, funct7) {
        (0b000, 0b0000000) => Ok(AluOp::Add),
        (0b000, 0b0100000) => Ok(AluOp::Sub),
        (0b001, 0b0000000) => Ok(AluOp::Sll),
        (0b010, 0b0000000) => Ok(AluOp::Slt),
        (0b011, 0b0000000) => Ok(AluOp::Sltu),
        (0b100, 0b0000000) => Ok(AluOp::Xor),
        (0b101, 0b0000000) => Ok(AluOp::Srl),
        (0b101, 0b0100000) => Ok(AluOp::Sra),
        (0b110, 0b0000000) => Ok(AluOp::Or),
        (0b111, 0b0000000) => Ok(AluOp::And),
        _ => Err(DecodeError::new(raw)),
    }
}

fn decode_system(raw: u32) -> Result<SystemKind, DecodeError> {
    match raw {
        0x0000_0073 => Ok(SystemKind::Ecall),
        0x0010_0073 => Ok(SystemKind::Ebreak),
        _ => Err(DecodeError::new(raw)),
    }
}

const fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

const fn imm_i(raw: u32) -> i32 {
    sign_extend(raw >> 20, 12)
}

const fn imm_s(raw: u32) -> i32 {
    let imm = ((raw >> 7) & 0x1f) | (((raw >> 25) & 0x7f) << 5);
    sign_extend(imm, 12)
}

const fn imm_b(raw: u32) -> i32 {
    let imm = (((raw >> 8) & 0x0f) << 1)
        | (((raw >> 25) & 0x3f) << 5)
        | (((raw >> 7) & 0x01) << 11)
        | (((raw >> 31) & 0x01) << 12);
    sign_extend(imm, 13)
}

const fn imm_u(raw: u32) -> i32 {
    (raw & 0xffff_f000) as i32
}

const fn imm_j(raw: u32) -> i32 {
    let imm = (((raw >> 21) & 0x03ff) << 1)
        | (((raw >> 20) & 0x0001) << 11)
        | (((raw >> 12) & 0x00ff) << 12)
        | (((raw >> 31) & 0x0001) << 20);
    sign_extend(imm, 21)
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::{AluOp, opcode::InstructionKind};

    #[test]
    fn decode_addi() {
        let decoded = decode(0x0050_0093, 0).expect("addi should decode");
        assert_eq!(decoded.kind, InstructionKind::OpImm(AluOp::Add));
        assert_eq!(decoded.rd, Some(1));
        assert_eq!(decoded.rs1, Some(0));
        assert_eq!(decoded.imm, 5);
    }

    #[test]
    fn decode_store_word() {
        let decoded = decode(0x0032_2023, 0).expect("sw should decode");
        assert_eq!(decoded.rs1, Some(4));
        assert_eq!(decoded.rs2, Some(3));
    }
}
