use rvsim_isa::{CsrAddress, Trap};

use super::PrivilegeMode;

const MSTATUS_MIE: u32 = 1 << 3;
const MSTATUS_MPIE: u32 = 1 << 7;
const MSTATUS_MPP_SHIFT: u32 = 11;
const MSTATUS_MPP_MASK: u32 = 0b11 << MSTATUS_MPP_SHIFT;

/// Machine-mode CSR values required by the first milestone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MachineCsrs {
    pub mstatus: u32,
    pub mtvec: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
}

/// Storage wrapper for CSR reads and writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CsrFile {
    machine: MachineCsrs,
}

impl CsrFile {
    #[must_use]
    pub fn read(&self, address: CsrAddress) -> u32 {
        match address {
            CsrAddress::Mstatus => self.machine.mstatus,
            CsrAddress::Mtvec => self.machine.mtvec,
            CsrAddress::Mepc => self.machine.mepc,
            CsrAddress::Mcause => self.machine.mcause,
            CsrAddress::Mtval => self.machine.mtval,
        }
    }

    pub fn write(&mut self, address: CsrAddress, value: u32) {
        match address {
            CsrAddress::Mstatus => self.machine.mstatus = value,
            CsrAddress::Mtvec => self.machine.mtvec = value,
            CsrAddress::Mepc => self.machine.mepc = value,
            CsrAddress::Mcause => self.machine.mcause = value,
            CsrAddress::Mtval => self.machine.mtval = value,
        }
    }

    #[must_use]
    pub fn machine(&self) -> &MachineCsrs {
        &self.machine
    }

    /// Record machine-mode trap state and return the handler PC derived from `mtvec`.
    #[must_use]
    pub fn enter_trap(
        &mut self,
        trap: Trap,
        current_pc: u32,
        current_privilege: PrivilegeMode,
    ) -> u32 {
        self.machine.mepc = current_pc;
        self.machine.mcause = trap.mcause();
        self.machine.mtval = trap.mtval();

        let mut mstatus = self.machine.mstatus;
        let mie = (mstatus & MSTATUS_MIE) != 0;
        mstatus = set_flag(mstatus, MSTATUS_MPIE, mie);
        mstatus &= !MSTATUS_MIE;
        mstatus = (mstatus & !MSTATUS_MPP_MASK) | privilege_bits(current_privilege);
        self.machine.mstatus = mstatus;

        let base = self.machine.mtvec & !0b11;
        let mode = self.machine.mtvec & 0b11;
        if matches!(trap, Trap::Interrupt(_)) && mode == 0b01 {
            base.wrapping_add((trap.mcause() & 0x7fff_ffff) * 4)
        } else {
            base
        }
    }

    /// Restore privilege state from `mstatus` and return the resume PC from `mepc`.
    #[must_use]
    pub fn return_from_trap(&mut self) -> (PrivilegeMode, u32) {
        let mut mstatus = self.machine.mstatus;
        let mpie = (mstatus & MSTATUS_MPIE) != 0;
        let mpp = (mstatus & MSTATUS_MPP_MASK) >> MSTATUS_MPP_SHIFT;

        mstatus = set_flag(mstatus, MSTATUS_MIE, mpie);
        mstatus |= MSTATUS_MPIE;
        mstatus &= !MSTATUS_MPP_MASK;
        self.machine.mstatus = mstatus;

        (privilege_from_bits(mpp), self.machine.mepc)
    }
}

const fn set_flag(value: u32, mask: u32, enabled: bool) -> u32 {
    if enabled { value | mask } else { value & !mask }
}

const fn privilege_bits(privilege: PrivilegeMode) -> u32 {
    match privilege {
        PrivilegeMode::User => 0 << MSTATUS_MPP_SHIFT,
        PrivilegeMode::Supervisor => 1 << MSTATUS_MPP_SHIFT,
        PrivilegeMode::Machine => 3 << MSTATUS_MPP_SHIFT,
    }
}

const fn privilege_from_bits(bits: u32) -> PrivilegeMode {
    match bits {
        0 => PrivilegeMode::User,
        1 => PrivilegeMode::Supervisor,
        _ => PrivilegeMode::Machine,
    }
}
