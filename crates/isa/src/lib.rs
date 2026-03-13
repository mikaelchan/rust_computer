//! RISC-V ISA primitives shared by all CPU models.

pub mod csr;
pub mod decode;
pub mod exception;
pub mod instruction;
pub mod opcode;

pub use csr::CsrAddress;
pub use decode::{DecodeError, decode};
pub use exception::{Exception, Interrupt, Trap};
pub use instruction::{DecodedInstruction, InstructionWord};
pub use opcode::{AluOp, BranchKind, InstructionKind, LoadKind, StoreKind, SystemKind};
