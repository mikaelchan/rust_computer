//! Decoded instruction classifications.

/// Integer ALU operations for RV32I.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOp {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Sll,
    Slt,
    Sltu,
    Srl,
    Sra,
}

/// Branch conditions supported by RV32I.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchKind {
    Beq,
    Bne,
    Blt,
    Bge,
    Bltu,
    Bgeu,
}

/// Load widths supported by RV32I.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadKind {
    Byte,
    Half,
    Word,
    ByteUnsigned,
    HalfUnsigned,
}

/// Store widths supported by RV32I.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    Byte,
    Half,
    Word,
}

/// CSR read/modify/write operations supported by the base privileged ISA.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrOp {
    ReadWrite,
    ReadSet,
    ReadClear,
    ReadWriteImmediate,
    ReadSetImmediate,
    ReadClearImmediate,
}

/// System instructions used in the initial CPU model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemKind {
    Ecall,
    Ebreak,
    SfenceVma,
    Mret,
    Sret,
}

/// High-level decoded instruction kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionKind {
    Lui,
    Auipc,
    Jal,
    Jalr,
    Branch(BranchKind),
    Load(LoadKind),
    Store(StoreKind),
    OpImm(AluOp),
    Op(AluOp),
    Csr(CsrOp),
    System(SystemKind),
}
