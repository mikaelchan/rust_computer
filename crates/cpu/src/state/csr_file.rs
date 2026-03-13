use rvsim_isa::{CsrAddress, Interrupt, Trap};
use rvsim_system::{InterruptLine, InterruptSet};

use super::PrivilegeMode;

const MSTATUS_MIE: u32 = 1 << 3;
const MSTATUS_MPIE: u32 = 1 << 7;
const MSTATUS_MPP_SHIFT: u32 = 11;
const MSTATUS_MPP_MASK: u32 = 0b11 << MSTATUS_MPP_SHIFT;
const MIE_MEIE: u32 = 1 << 11;
const MIE_MTIE: u32 = 1 << 7;
const MIP_MEIP: u32 = 1 << 11;
const MIP_MTIP: u32 = 1 << 7;

/// Machine-mode CSR values required by the first milestone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MachineCsrs {
    pub mstatus: u32,
    pub mie: u32,
    pub mtvec: u32,
    pub mcycle: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mip: u32,
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
            CsrAddress::Mie => self.machine.mie,
            CsrAddress::Mtvec => self.machine.mtvec,
            CsrAddress::Mcycle => self.machine.mcycle,
            CsrAddress::Mepc => self.machine.mepc,
            CsrAddress::Mcause => self.machine.mcause,
            CsrAddress::Mtval => self.machine.mtval,
            CsrAddress::Mip => self.machine.mip,
        }
    }

    pub fn write(&mut self, address: CsrAddress, value: u32) {
        match address {
            CsrAddress::Mstatus => self.machine.mstatus = value,
            CsrAddress::Mie => self.machine.mie = value,
            CsrAddress::Mtvec => self.machine.mtvec = value,
            CsrAddress::Mcycle => self.machine.mcycle = value,
            CsrAddress::Mepc => self.machine.mepc = value,
            CsrAddress::Mcause => self.machine.mcause = value,
            CsrAddress::Mtval => self.machine.mtval = value,
            CsrAddress::Mip => self.machine.mip = value,
        }
    }

    #[must_use]
    pub fn machine(&self) -> &MachineCsrs {
        &self.machine
    }

    pub fn increment_cycle(&mut self) {
        self.machine.mcycle = self.machine.mcycle.wrapping_add(1);
    }

    pub fn sync_interrupts(&mut self, interrupts: InterruptSet) {
        self.machine.mip &= !(MIP_MEIP | MIP_MTIP);

        if interrupts.contains(InterruptLine::MachineExternal) {
            self.machine.mip |= MIP_MEIP;
        }

        if interrupts.contains(InterruptLine::MachineTimer) {
            self.machine.mip |= MIP_MTIP;
        }
    }

    #[must_use]
    pub fn sync_interrupt_line(&mut self, interrupt: Option<InterruptLine>) {
        self.sync_interrupts(
            interrupt
                .map(InterruptSet::from)
                .unwrap_or_else(InterruptSet::empty),
        );
    }

    #[must_use]
    pub fn pending_machine_interrupt(&self) -> Option<Interrupt> {
        if (self.machine.mstatus & MSTATUS_MIE) == 0 {
            return None;
        }

        if (self.machine.mie & MIE_MEIE) != 0 && (self.machine.mip & MIP_MEIP) != 0 {
            return Some(Interrupt::MachineExternal);
        }

        if (self.machine.mie & MIE_MTIE) != 0 && (self.machine.mip & MIP_MTIP) != 0 {
            return Some(Interrupt::MachineTimer);
        }

        None
    }

    #[must_use]
    pub fn machine_timer_interrupt_enabled(&self) -> bool {
        matches!(
            self.pending_machine_interrupt(),
            Some(Interrupt::MachineTimer)
        )
    }

    #[must_use]
    pub fn machine_external_interrupt_enabled(&self) -> bool {
        matches!(
            self.pending_machine_interrupt(),
            Some(Interrupt::MachineExternal)
        )
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

#[cfg(test)]
mod tests {
    use rvsim_isa::{CsrAddress, Interrupt};
    use rvsim_system::{InterruptLine, InterruptSet};

    use super::CsrFile;

    #[test]
    fn syncs_pending_machine_interrupt_bits_from_interrupt_set() {
        let mut csrs = CsrFile::default();
        csrs.sync_interrupts(
            InterruptSet::from(InterruptLine::MachineTimer)
                .union(InterruptSet::from(InterruptLine::MachineExternal)),
        );

        assert_eq!(csrs.read(CsrAddress::Mip), (1 << 7) | (1 << 11));
    }

    #[test]
    fn prefers_machine_external_interrupt_over_machine_timer() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mstatus, 1 << 3);
        csrs.write(CsrAddress::Mie, (1 << 7) | (1 << 11));
        csrs.sync_interrupts(
            InterruptSet::from(InterruptLine::MachineTimer)
                .union(InterruptSet::from(InterruptLine::MachineExternal)),
        );

        assert_eq!(
            csrs.pending_machine_interrupt(),
            Some(Interrupt::MachineExternal)
        );
    }
}
