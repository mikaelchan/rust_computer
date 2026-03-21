mod csr_file;
mod privilege;
mod registers;

pub use csr_file::{CsrFile, MachineCsrs, SupervisorCsrs};
pub use privilege::PrivilegeMode;
pub use registers::RegisterFile;

/// Architectural state visible at instruction boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HartState {
    reset_vector: u32,
    pub pc: u32,
    pub registers: RegisterFile,
    pub csrs: CsrFile,
    pub privilege: PrivilegeMode,
    pub halted: bool,
}

impl HartState {
    #[must_use]
    pub fn new(reset_vector: u32) -> Self {
        let mut state = Self {
            reset_vector,
            pc: reset_vector,
            registers: RegisterFile::default(),
            csrs: CsrFile::default(),
            privilege: PrivilegeMode::Machine,
            halted: false,
        };
        state.csrs.write(rvsim_isa::CsrAddress::Mtvec, reset_vector);
        state.csrs.write(rvsim_isa::CsrAddress::Stvec, reset_vector);
        state
    }

    pub fn reset(&mut self, reset_vector: u32) {
        self.reset_vector = reset_vector;
        self.pc = reset_vector;
        self.registers = RegisterFile::default();
        self.csrs = CsrFile::default();
        self.csrs.write(rvsim_isa::CsrAddress::Mtvec, reset_vector);
        self.csrs.write(rvsim_isa::CsrAddress::Stvec, reset_vector);
        self.privilege = PrivilegeMode::Machine;
        self.halted = false;
    }

    #[must_use]
    pub const fn reset_vector(&self) -> u32 {
        self.reset_vector
    }
}
