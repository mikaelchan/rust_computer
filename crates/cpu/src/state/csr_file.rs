use rvsim_isa::{CsrAddress, Interrupt, Trap};
use rvsim_system::{InterruptLine, InterruptSet};

use super::PrivilegeMode;

const MSTATUS_SIE: u32 = 1 << 1;
const MSTATUS_MIE: u32 = 1 << 3;
const MSTATUS_SPIE: u32 = 1 << 5;
const MSTATUS_MPIE: u32 = 1 << 7;
const MSTATUS_SPP: u32 = 1 << 8;
const MSTATUS_SUM: u32 = 1 << 18;
const MSTATUS_MXR: u32 = 1 << 19;
const MSTATUS_TVM: u32 = 1 << 20;
const MSTATUS_TW: u32 = 1 << 21;
const MSTATUS_TSR: u32 = 1 << 22;
const MSTATUS_MPP_SHIFT: u32 = 11;
const MSTATUS_MPP_MASK: u32 = 0b11 << MSTATUS_MPP_SHIFT;
const SSTATUS_MASK: u32 = MSTATUS_SIE | MSTATUS_SPIE | MSTATUS_SPP | MSTATUS_SUM | MSTATUS_MXR;
const MIE_SSIE: u32 = 1 << 1;
const MIE_MSIE: u32 = 1 << 3;
const MIE_STIE: u32 = 1 << 5;
const MIE_SEIE: u32 = 1 << 9;
const MIE_MEIE: u32 = 1 << 11;
const MIE_MTIE: u32 = 1 << 7;
const SIE_MASK: u32 = MIE_SSIE | MIE_STIE | MIE_SEIE;
const MIP_SSIP: u32 = 1 << 1;
const MIP_MSIP: u32 = 1 << 3;
const MIP_STIP: u32 = 1 << 5;
const MIP_SEIP: u32 = 1 << 9;
const MIP_MEIP: u32 = 1 << 11;
const MIP_MTIP: u32 = 1 << 7;
const SIP_MASK: u32 = MIP_SSIP | MIP_STIP | MIP_SEIP;
const MANAGED_INTERRUPT_MASK: u32 = MIP_SSIP | MIP_MSIP | MIP_STIP | MIP_MTIP | MIP_SEIP | MIP_MEIP;
const MIP_WRITABLE_MASK: u32 = MIP_SSIP | MIP_MSIP;
const SIP_WRITABLE_MASK: u32 = MIP_SSIP;
const MIDELEG_MASK: u32 = MIP_SSIP | MIP_STIP | MIP_SEIP;
const COUNTEREN_CY: u32 = 1 << 0;
const COUNTEREN_TM: u32 = 1 << 1;
const COUNTEREN_IR: u32 = 1 << 2;
const COUNTEREN_MASK: u32 = COUNTEREN_CY | COUNTEREN_TM | COUNTEREN_IR;
const INTERRUPT_PRIORITY: [Interrupt; 6] = [
    Interrupt::MachineExternal,
    Interrupt::MachineSoftware,
    Interrupt::MachineTimer,
    Interrupt::SupervisorExternal,
    Interrupt::SupervisorSoftware,
    Interrupt::SupervisorTimer,
];

/// Machine-mode CSR values required by the first milestone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MachineCsrs {
    pub mstatus: u32,
    pub medeleg: u32,
    pub mideleg: u32,
    pub mie: u32,
    pub mtvec: u32,
    pub mcounteren: u32,
    pub mcycle: u64,
    pub minstret: u64,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub mip: u32,
}

/// Supervisor-visible trap and address-translation state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SupervisorCsrs {
    pub stvec: u32,
    pub scounteren: u32,
    pub sepc: u32,
    pub scause: u32,
    pub stval: u32,
    pub satp: u32,
}

/// Storage wrapper for CSR reads and writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CsrFile {
    machine: MachineCsrs,
    supervisor: SupervisorCsrs,
    software_pending: u32,
    external_pending: u32,
}

impl CsrFile {
    #[must_use]
    pub fn read(&self, address: CsrAddress) -> u32 {
        match address {
            CsrAddress::Sstatus => self.machine.mstatus & SSTATUS_MASK,
            CsrAddress::Sie => self.machine.mie & SIE_MASK,
            CsrAddress::Stvec => self.supervisor.stvec,
            CsrAddress::Scounteren => self.supervisor.scounteren,
            CsrAddress::Satp => self.supervisor.satp,
            CsrAddress::Mstatus => self.machine.mstatus,
            CsrAddress::Medeleg => self.machine.medeleg,
            CsrAddress::Mideleg => self.machine.mideleg,
            CsrAddress::Mie => self.machine.mie,
            CsrAddress::Mtvec => self.machine.mtvec,
            CsrAddress::Mcounteren => self.machine.mcounteren,
            CsrAddress::Mcycle => low_half(self.machine.mcycle),
            CsrAddress::Minstret => low_half(self.machine.minstret),
            CsrAddress::Mcycleh => high_half(self.machine.mcycle),
            CsrAddress::Minstreth => high_half(self.machine.minstret),
            CsrAddress::Cycle | CsrAddress::Time => low_half(self.machine.mcycle),
            CsrAddress::Instret => low_half(self.machine.minstret),
            CsrAddress::Cycleh | CsrAddress::Timeh => high_half(self.machine.mcycle),
            CsrAddress::Instreth => high_half(self.machine.minstret),
            CsrAddress::Sepc => self.supervisor.sepc,
            CsrAddress::Scause => self.supervisor.scause,
            CsrAddress::Stval => self.supervisor.stval,
            CsrAddress::Sip => self.machine.mip & SIP_MASK,
            CsrAddress::Mepc => self.machine.mepc,
            CsrAddress::Mcause => self.machine.mcause,
            CsrAddress::Mtval => self.machine.mtval,
            CsrAddress::Mip => self.machine.mip,
        }
    }

    pub fn write(&mut self, address: CsrAddress, value: u32) {
        match address {
            CsrAddress::Sstatus => {
                self.machine.mstatus =
                    (self.machine.mstatus & !SSTATUS_MASK) | (value & SSTATUS_MASK);
            }
            CsrAddress::Sie => {
                self.machine.mie = (self.machine.mie & !SIE_MASK) | (value & SIE_MASK);
            }
            CsrAddress::Stvec => self.supervisor.stvec = value,
            CsrAddress::Scounteren => self.supervisor.scounteren = value & COUNTEREN_MASK,
            CsrAddress::Satp => self.supervisor.satp = value,
            CsrAddress::Mstatus => self.machine.mstatus = value,
            CsrAddress::Medeleg => self.machine.medeleg = value,
            CsrAddress::Mideleg => self.machine.mideleg = value & MIDELEG_MASK,
            CsrAddress::Mie => self.machine.mie = value,
            CsrAddress::Mtvec => self.machine.mtvec = value,
            CsrAddress::Mcounteren => self.machine.mcounteren = value & COUNTEREN_MASK,
            CsrAddress::Mcycle | CsrAddress::Cycle | CsrAddress::Time => {
                write_low_half(&mut self.machine.mcycle, value);
            }
            CsrAddress::Minstret | CsrAddress::Instret => {
                write_low_half(&mut self.machine.minstret, value);
            }
            CsrAddress::Mcycleh | CsrAddress::Cycleh | CsrAddress::Timeh => {
                write_high_half(&mut self.machine.mcycle, value);
            }
            CsrAddress::Minstreth | CsrAddress::Instreth => {
                write_high_half(&mut self.machine.minstret, value);
            }
            CsrAddress::Sepc => self.supervisor.sepc = value,
            CsrAddress::Scause => self.supervisor.scause = value,
            CsrAddress::Stval => self.supervisor.stval = value,
            CsrAddress::Sip => {
                self.software_pending =
                    (self.software_pending & !SIP_WRITABLE_MASK) | (value & SIP_WRITABLE_MASK);
                self.refresh_mip();
            }
            CsrAddress::Mepc => self.machine.mepc = value,
            CsrAddress::Mcause => self.machine.mcause = value,
            CsrAddress::Mtval => self.machine.mtval = value,
            CsrAddress::Mip => {
                self.software_pending =
                    (self.software_pending & !MIP_WRITABLE_MASK) | (value & MIP_WRITABLE_MASK);
                self.refresh_mip();
            }
        }
    }

    #[must_use]
    pub fn machine(&self) -> &MachineCsrs {
        &self.machine
    }

    #[must_use]
    pub fn supervisor(&self) -> &SupervisorCsrs {
        &self.supervisor
    }

    pub fn increment_cycle(&mut self) {
        self.machine.mcycle = self.machine.mcycle.wrapping_add(1);
    }

    pub fn increment_instret(&mut self, retired: u64) {
        self.machine.minstret = self.machine.minstret.wrapping_add(retired);
    }

    pub fn sync_interrupts(&mut self, interrupts: InterruptSet) {
        self.external_pending = 0;

        if interrupts.contains(InterruptLine::SupervisorSoftware) {
            self.external_pending |= MIP_SSIP;
        }

        if interrupts.contains(InterruptLine::MachineExternal) {
            self.external_pending |= MIP_MEIP;
        }

        if interrupts.contains(InterruptLine::MachineSoftware) {
            self.external_pending |= MIP_MSIP;
        }

        if interrupts.contains(InterruptLine::SupervisorTimer) {
            self.external_pending |= MIP_STIP;
        }

        if interrupts.contains(InterruptLine::MachineTimer) {
            self.external_pending |= MIP_MTIP;
        }

        if interrupts.contains(InterruptLine::SupervisorExternal) {
            self.external_pending |= MIP_SEIP;
        }

        self.refresh_mip();
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
    pub fn pending_interrupt(&self, current_privilege: PrivilegeMode) -> Option<Interrupt> {
        for interrupt in INTERRUPT_PRIORITY {
            if !self.interrupt_is_pending(interrupt) || !self.interrupt_is_enabled(interrupt) {
                continue;
            }

            let Some(target_privilege) =
                self.interrupt_target_privilege(interrupt, current_privilege)
            else {
                continue;
            };

            if self.interrupt_globally_enabled(target_privilege, current_privilege) {
                return Some(interrupt);
            }
        }

        None
    }

    #[must_use]
    pub fn allows_csr_access(&self, privilege: PrivilegeMode, address: CsrAddress) -> bool {
        if privilege.csr_level() < address.min_privilege_level() {
            return false;
        }

        let Some(counteren_mask) = address.counteren_mask() else {
            return true;
        };

        match privilege {
            PrivilegeMode::Machine => true,
            PrivilegeMode::Supervisor => (self.machine.mcounteren & counteren_mask) != 0,
            PrivilegeMode::User => {
                (self.machine.mcounteren & counteren_mask) != 0
                    && (self.supervisor.scounteren & counteren_mask) != 0
            }
        }
    }

    #[must_use]
    pub fn tvm_enabled(&self) -> bool {
        (self.machine.mstatus & MSTATUS_TVM) != 0
    }

    #[must_use]
    pub fn tsr_enabled(&self) -> bool {
        (self.machine.mstatus & MSTATUS_TSR) != 0
    }

    #[must_use]
    pub fn tw_enabled(&self) -> bool {
        (self.machine.mstatus & MSTATUS_TW) != 0
    }

    fn refresh_mip(&mut self) {
        self.machine.mip = (self.machine.mip & !MANAGED_INTERRUPT_MASK)
            | (self.software_pending & MIP_WRITABLE_MASK)
            | (self.external_pending & MANAGED_INTERRUPT_MASK);
    }

    fn delegates_trap_to_supervisor(&self, trap: Trap, current_privilege: PrivilegeMode) -> bool {
        if matches!(current_privilege, PrivilegeMode::Machine) {
            return false;
        }

        match trap {
            Trap::Exception(_) => (self.machine.medeleg & (1 << trap.cause_code())) != 0,
            Trap::Interrupt(interrupt) => self.delegates_interrupt_to_supervisor(interrupt),
        }
    }

    fn interrupt_is_pending(&self, interrupt: Interrupt) -> bool {
        (self.machine.mip & interrupt_pending_mask(interrupt)) != 0
    }

    fn interrupt_is_enabled(&self, interrupt: Interrupt) -> bool {
        (self.machine.mie & interrupt_enable_mask(interrupt)) != 0
    }

    fn delegates_interrupt_to_supervisor(&self, interrupt: Interrupt) -> bool {
        interrupt.is_supervisor() && (self.machine.mideleg & (1 << interrupt.cause_code())) != 0
    }

    fn interrupt_target_privilege(
        &self,
        interrupt: Interrupt,
        current_privilege: PrivilegeMode,
    ) -> Option<PrivilegeMode> {
        if self.delegates_interrupt_to_supervisor(interrupt) {
            if matches!(current_privilege, PrivilegeMode::Machine) {
                None
            } else {
                Some(PrivilegeMode::Supervisor)
            }
        } else {
            Some(PrivilegeMode::Machine)
        }
    }

    fn interrupt_globally_enabled(
        &self,
        target_privilege: PrivilegeMode,
        current_privilege: PrivilegeMode,
    ) -> bool {
        match target_privilege {
            PrivilegeMode::Machine => {
                !matches!(current_privilege, PrivilegeMode::Machine)
                    || (self.machine.mstatus & MSTATUS_MIE) != 0
            }
            PrivilegeMode::Supervisor => {
                matches!(current_privilege, PrivilegeMode::User)
                    || (matches!(current_privilege, PrivilegeMode::Supervisor)
                        && (self.machine.mstatus & MSTATUS_SIE) != 0)
            }
            PrivilegeMode::User => false,
        }
    }

    fn enter_machine_trap(
        &mut self,
        trap: Trap,
        current_pc: u32,
        current_privilege: PrivilegeMode,
    ) -> u32 {
        self.machine.mepc = current_pc;
        self.machine.mcause = trap.cause_bits();
        self.machine.mtval = trap.tval();

        let mut mstatus = self.machine.mstatus;
        let mie = (mstatus & MSTATUS_MIE) != 0;
        mstatus = set_flag(mstatus, MSTATUS_MPIE, mie);
        mstatus &= !MSTATUS_MIE;
        mstatus = (mstatus & !MSTATUS_MPP_MASK) | privilege_bits(current_privilege);
        self.machine.mstatus = mstatus;

        let base = self.machine.mtvec & !0b11;
        let mode = self.machine.mtvec & 0b11;
        if trap.is_interrupt() && mode == 0b01 {
            base.wrapping_add(trap.cause_code() * 4)
        } else {
            base
        }
    }

    fn enter_supervisor_trap(
        &mut self,
        trap: Trap,
        current_pc: u32,
        current_privilege: PrivilegeMode,
    ) -> u32 {
        self.supervisor.sepc = current_pc;
        self.supervisor.scause = trap.cause_bits();
        self.supervisor.stval = trap.tval();

        let mut mstatus = self.machine.mstatus;
        let sie = (mstatus & MSTATUS_SIE) != 0;
        mstatus = set_flag(mstatus, MSTATUS_SPIE, sie);
        mstatus &= !MSTATUS_SIE;
        mstatus = set_flag(
            mstatus,
            MSTATUS_SPP,
            matches!(current_privilege, PrivilegeMode::Supervisor),
        );
        self.machine.mstatus = mstatus;

        let base = self.supervisor.stvec & !0b11;
        let mode = self.supervisor.stvec & 0b11;
        if trap.is_interrupt() && mode == 0b01 {
            base.wrapping_add(trap.cause_code() * 4)
        } else {
            base
        }
    }

    /// Record trap state and return the target privilege plus trap vector.
    #[must_use]
    pub fn enter_trap(
        &mut self,
        trap: Trap,
        current_pc: u32,
        current_privilege: PrivilegeMode,
    ) -> (PrivilegeMode, u32) {
        if self.delegates_trap_to_supervisor(trap, current_privilege) {
            (
                PrivilegeMode::Supervisor,
                self.enter_supervisor_trap(trap, current_pc, current_privilege),
            )
        } else {
            (
                PrivilegeMode::Machine,
                self.enter_machine_trap(trap, current_pc, current_privilege),
            )
        }
    }

    /// Restore privilege state from `mstatus`/`mepc`.
    #[must_use]
    pub fn return_from_machine_trap(&mut self) -> (PrivilegeMode, u32) {
        let mut mstatus = self.machine.mstatus;
        let mpie = (mstatus & MSTATUS_MPIE) != 0;
        let mpp = (mstatus & MSTATUS_MPP_MASK) >> MSTATUS_MPP_SHIFT;

        mstatus = set_flag(mstatus, MSTATUS_MIE, mpie);
        mstatus |= MSTATUS_MPIE;
        mstatus &= !MSTATUS_MPP_MASK;
        self.machine.mstatus = mstatus;

        (privilege_from_bits(mpp), self.machine.mepc)
    }

    /// Restore privilege state from `sstatus`/`sepc`.
    #[must_use]
    pub fn return_from_supervisor_trap(&mut self) -> (PrivilegeMode, u32) {
        let mut mstatus = self.machine.mstatus;
        let spie = (mstatus & MSTATUS_SPIE) != 0;
        let spp = (mstatus & MSTATUS_SPP) != 0;

        mstatus = set_flag(mstatus, MSTATUS_SIE, spie);
        mstatus |= MSTATUS_SPIE;
        mstatus &= !MSTATUS_SPP;
        self.machine.mstatus = mstatus;

        (
            if spp {
                PrivilegeMode::Supervisor
            } else {
                PrivilegeMode::User
            },
            self.supervisor.sepc,
        )
    }
}

const fn low_half(value: u64) -> u32 {
    value as u32
}

const fn high_half(value: u64) -> u32 {
    (value >> 32) as u32
}

fn write_low_half(target: &mut u64, value: u32) {
    *target = (*target & !0xffff_ffff) | u64::from(value);
}

fn write_high_half(target: &mut u64, value: u32) {
    *target = (u64::from(value) << 32) | (*target & 0xffff_ffff);
}

const fn set_flag(value: u32, mask: u32, enabled: bool) -> u32 {
    if enabled { value | mask } else { value & !mask }
}

const fn interrupt_pending_mask(interrupt: Interrupt) -> u32 {
    match interrupt {
        Interrupt::SupervisorSoftware => MIP_SSIP,
        Interrupt::MachineSoftware => MIP_MSIP,
        Interrupt::SupervisorTimer => MIP_STIP,
        Interrupt::MachineTimer => MIP_MTIP,
        Interrupt::SupervisorExternal => MIP_SEIP,
        Interrupt::MachineExternal => MIP_MEIP,
    }
}

const fn interrupt_enable_mask(interrupt: Interrupt) -> u32 {
    match interrupt {
        Interrupt::SupervisorSoftware => MIE_SSIE,
        Interrupt::MachineSoftware => MIE_MSIE,
        Interrupt::SupervisorTimer => MIE_STIE,
        Interrupt::MachineTimer => MIE_MTIE,
        Interrupt::SupervisorExternal => MIE_SEIE,
        Interrupt::MachineExternal => MIE_MEIE,
    }
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
    use rvsim_isa::{CsrAddress, Exception, Interrupt, Trap};
    use rvsim_system::{InterruptLine, InterruptSet};

    use super::CsrFile;
    use crate::state::PrivilegeMode;

    #[test]
    fn syncs_pending_supervisor_and_machine_interrupt_bits_from_interrupt_set() {
        let mut csrs = CsrFile::default();
        csrs.sync_interrupts(
            InterruptSet::from(InterruptLine::SupervisorSoftware)
                .union(InterruptSet::from(InterruptLine::MachineSoftware))
                .union(InterruptSet::from(InterruptLine::SupervisorTimer))
                .union(InterruptSet::from(InterruptLine::MachineTimer))
                .union(InterruptSet::from(InterruptLine::SupervisorExternal))
                .union(InterruptSet::from(InterruptLine::MachineExternal)),
        );

        assert_eq!(
            csrs.read(CsrAddress::Mip),
            (1 << 1) | (1 << 3) | (1 << 5) | (1 << 7) | (1 << 9) | (1 << 11)
        );
    }

    #[test]
    fn prefers_machine_external_interrupt_over_software_and_timer() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mstatus, 1 << 3);
        csrs.write(CsrAddress::Mie, (1 << 3) | (1 << 7) | (1 << 11));
        csrs.sync_interrupts(
            InterruptSet::from(InterruptLine::MachineSoftware)
                .union(InterruptSet::from(InterruptLine::MachineTimer))
                .union(InterruptSet::from(InterruptLine::MachineExternal)),
        );

        assert_eq!(
            csrs.pending_interrupt(PrivilegeMode::Machine),
            Some(Interrupt::MachineExternal)
        );
    }

    #[test]
    fn prefers_machine_software_interrupt_over_machine_timer() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mstatus, 1 << 3);
        csrs.write(CsrAddress::Mie, (1 << 3) | (1 << 7));
        csrs.sync_interrupts(
            InterruptSet::from(InterruptLine::MachineSoftware)
                .union(InterruptSet::from(InterruptLine::MachineTimer)),
        );

        assert_eq!(
            csrs.pending_interrupt(PrivilegeMode::Machine),
            Some(Interrupt::MachineSoftware)
        );
    }

    #[test]
    fn software_pending_bits_survive_sync_without_bus_sources() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mip, super::MIP_MSIP | super::MIP_SSIP);
        csrs.sync_interrupts(InterruptSet::empty());

        assert_eq!(
            csrs.read(CsrAddress::Mip) & (super::MIP_MSIP | super::MIP_SSIP),
            super::MIP_MSIP | super::MIP_SSIP
        );
    }

    #[test]
    fn mip_write_only_controls_modeled_software_pending_bits() {
        let mut csrs = CsrFile::default();
        csrs.sync_interrupts(InterruptSet::from(InterruptLine::MachineTimer));
        csrs.write(CsrAddress::Mip, u32::MAX);

        assert_eq!(
            csrs.read(CsrAddress::Mip),
            super::MIP_MSIP | super::MIP_SSIP | super::MIP_MTIP
        );
    }

    #[test]
    fn sip_write_only_controls_ssip_and_preserves_device_pending_bits() {
        let mut csrs = CsrFile::default();
        csrs.sync_interrupts(
            InterruptSet::from(InterruptLine::SupervisorTimer)
                .union(InterruptSet::from(InterruptLine::SupervisorExternal)),
        );
        csrs.write(CsrAddress::Sip, u32::MAX);

        assert_eq!(
            csrs.read(CsrAddress::Sip),
            super::MIP_SSIP | super::MIP_STIP | super::MIP_SEIP
        );
        assert_eq!(csrs.read(CsrAddress::Mip) & super::MIP_MSIP, 0);
    }

    #[test]
    fn delegated_supervisor_interrupt_is_pending_in_user_mode() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mideleg, 1 << 9);
        csrs.write(CsrAddress::Sie, 1 << 9);
        csrs.sync_interrupts(InterruptSet::from(InterruptLine::SupervisorExternal));

        assert_eq!(
            csrs.pending_interrupt(PrivilegeMode::User),
            Some(Interrupt::SupervisorExternal)
        );
    }

    #[test]
    fn delegated_supervisor_interrupt_is_masked_in_machine_mode() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mstatus, 1 << 3);
        csrs.write(CsrAddress::Mideleg, 1 << 9);
        csrs.write(CsrAddress::Sie, 1 << 9);
        csrs.sync_interrupts(InterruptSet::from(InterruptLine::SupervisorExternal));

        assert_eq!(csrs.pending_interrupt(PrivilegeMode::Machine), None);
    }

    #[test]
    fn supervisor_mode_requires_sie_for_delegated_supervisor_interrupts() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mideleg, 1 << 1);
        csrs.write(CsrAddress::Sie, 1 << 1);
        csrs.sync_interrupts(InterruptSet::from(InterruptLine::SupervisorSoftware));

        assert_eq!(csrs.pending_interrupt(PrivilegeMode::Supervisor), None);

        csrs.write(CsrAddress::Sstatus, 1 << 1);

        assert_eq!(
            csrs.pending_interrupt(PrivilegeMode::Supervisor),
            Some(Interrupt::SupervisorSoftware)
        );
    }

    #[test]
    fn clearing_software_pending_does_not_clear_bus_driven_interrupt_line() {
        let mut csrs = CsrFile::default();
        csrs.sync_interrupts(InterruptSet::from(InterruptLine::MachineSoftware));
        csrs.write(CsrAddress::Mip, 0);

        assert_eq!(
            csrs.read(CsrAddress::Mip) & super::MIP_MSIP,
            super::MIP_MSIP
        );
    }

    #[test]
    fn nondelegated_supervisor_interrupt_still_targets_machine_mode() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mie, 1 << 9);
        csrs.write(CsrAddress::Mtvec, 0x40);

        let (privilege, handler_pc) = csrs.enter_trap(
            Trap::Interrupt(Interrupt::SupervisorExternal),
            0x18,
            PrivilegeMode::User,
        );

        assert_eq!(privilege, PrivilegeMode::Machine);
        assert_eq!(handler_pc, 0x40);
        assert_eq!(csrs.read(CsrAddress::Mcause), (1 << 31) | 9);
        assert_eq!(csrs.read(CsrAddress::Mepc), 0x18);
    }

    #[test]
    fn machine_interrupt_uses_vectored_mtvec_offset() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mtvec, 0x40 | 0b01);

        let (privilege, handler_pc) = csrs.enter_trap(
            Trap::Interrupt(Interrupt::MachineSoftware),
            0x18,
            PrivilegeMode::User,
        );

        assert_eq!(privilege, PrivilegeMode::Machine);
        assert_eq!(handler_pc, 0x40 + (3 * 4));
        assert_eq!(csrs.read(CsrAddress::Mcause), (1 << 31) | 3);
        assert_eq!(csrs.read(CsrAddress::Mepc), 0x18);
    }

    #[test]
    fn delegated_supervisor_interrupt_enters_supervisor_trap_state() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mideleg, 1 << 1);
        csrs.write(CsrAddress::Stvec, 0x80);
        csrs.write(CsrAddress::Sie, 1 << 1);

        let (privilege, handler_pc) = csrs.enter_trap(
            Trap::Interrupt(Interrupt::SupervisorSoftware),
            0x24,
            PrivilegeMode::User,
        );

        assert_eq!(privilege, PrivilegeMode::Supervisor);
        assert_eq!(handler_pc, 0x80);
        assert_eq!(csrs.read(CsrAddress::Sepc), 0x24);
        assert_eq!(csrs.read(CsrAddress::Scause), (1 << 31) | 1);
        assert_eq!(csrs.read(CsrAddress::Stval), 0);
        assert_eq!(csrs.read(CsrAddress::Sstatus) & (1 << 5), 0);
    }

    #[test]
    fn delegated_supervisor_interrupt_uses_vectored_stvec_offset() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mideleg, 1 << 9);
        csrs.write(CsrAddress::Stvec, 0x80 | 0b01);
        csrs.write(CsrAddress::Sie, 1 << 9);

        let (privilege, handler_pc) = csrs.enter_trap(
            Trap::Interrupt(Interrupt::SupervisorExternal),
            0x24,
            PrivilegeMode::User,
        );

        assert_eq!(privilege, PrivilegeMode::Supervisor);
        assert_eq!(handler_pc, 0x80 + (9 * 4));
        assert_eq!(csrs.read(CsrAddress::Scause), (1 << 31) | 9);
        assert_eq!(csrs.read(CsrAddress::Sepc), 0x24);
    }

    #[test]
    fn delegated_user_ecall_enters_supervisor_trap_state() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Medeleg, 1 << 8);
        csrs.write(CsrAddress::Stvec, 0x80);
        csrs.write(CsrAddress::Sstatus, 1 << 1);

        let (privilege, handler_pc) = csrs.enter_trap(
            Trap::Exception(Exception::EnvironmentCallFromUMode),
            0x24,
            PrivilegeMode::User,
        );

        assert_eq!(privilege, PrivilegeMode::Supervisor);
        assert_eq!(handler_pc, 0x80);
        assert_eq!(csrs.read(CsrAddress::Sepc), 0x24);
        assert_eq!(csrs.read(CsrAddress::Scause), 8);
        assert_eq!(csrs.read(CsrAddress::Stval), 0);
        assert_eq!(csrs.read(CsrAddress::Sstatus) & (1 << 5), 1 << 5);
        assert_eq!(csrs.read(CsrAddress::Sstatus) & (1 << 1), 0);
    }

    #[test]
    fn supervisor_return_restores_user_mode_and_sepc() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Sepc, 0x44);
        csrs.write(CsrAddress::Sstatus, 1 << 5);

        let (privilege, pc) = csrs.return_from_supervisor_trap();

        assert_eq!(privilege, PrivilegeMode::User);
        assert_eq!(pc, 0x44);
        assert_eq!(csrs.read(CsrAddress::Sstatus) & (1 << 1), 1 << 1);
        assert_eq!(csrs.read(CsrAddress::Sstatus) & (1 << 8), 0);
    }

    #[test]
    fn sstatus_is_a_masked_view_of_mstatus() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mstatus, 0xffff_ffff);
        csrs.write(CsrAddress::Sstatus, 0);

        assert_eq!(csrs.read(CsrAddress::Sstatus), 0);
        assert_eq!(
            csrs.read(CsrAddress::Mstatus) & !super::SSTATUS_MASK,
            !super::SSTATUS_MASK
        );
    }

    #[test]
    fn sstatus_exposes_sum_and_mxr_bits() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Sstatus, super::MSTATUS_SUM | super::MSTATUS_MXR);

        assert_eq!(
            csrs.read(CsrAddress::Sstatus),
            super::MSTATUS_SUM | super::MSTATUS_MXR
        );
        assert_eq!(
            csrs.read(CsrAddress::Mstatus) & (super::MSTATUS_SUM | super::MSTATUS_MXR),
            super::MSTATUS_SUM | super::MSTATUS_MXR
        );
    }

    #[test]
    fn tw_enabled_reflects_mstatus_without_leaking_into_sstatus() {
        let mut csrs = CsrFile::default();
        assert!(!csrs.tw_enabled());

        csrs.write(CsrAddress::Mstatus, super::MSTATUS_TW);

        assert!(csrs.tw_enabled());
        assert_eq!(csrs.read(CsrAddress::Sstatus) & super::MSTATUS_TW, 0);
    }

    #[test]
    fn supervisor_counter_access_requires_mcounteren() {
        let mut csrs = CsrFile::default();

        assert!(!csrs.allows_csr_access(PrivilegeMode::Supervisor, CsrAddress::Cycleh));

        csrs.write(CsrAddress::Mcounteren, super::COUNTEREN_CY);

        assert!(csrs.allows_csr_access(PrivilegeMode::Supervisor, CsrAddress::Cycleh));
    }

    #[test]
    fn user_counter_access_requires_machine_and_supervisor_counteren() {
        let mut csrs = CsrFile::default();

        csrs.write(CsrAddress::Mcounteren, super::COUNTEREN_IR);
        assert!(!csrs.allows_csr_access(PrivilegeMode::User, CsrAddress::Instreth));

        csrs.write(CsrAddress::Scounteren, super::COUNTEREN_IR);
        assert!(csrs.allows_csr_access(PrivilegeMode::User, CsrAddress::Instreth));
    }

    #[test]
    fn counter_shadows_reflect_both_halves_and_time_follows_cycle_domain() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mcycle, 7);
        csrs.write(CsrAddress::Mcycleh, 1);
        csrs.write(CsrAddress::Minstret, 3);
        csrs.write(CsrAddress::Minstreth, 2);

        assert_eq!(csrs.read(CsrAddress::Cycle), 7);
        assert_eq!(csrs.read(CsrAddress::Time), 7);
        assert_eq!(csrs.read(CsrAddress::Cycleh), 1);
        assert_eq!(csrs.read(CsrAddress::Timeh), 1);
        assert_eq!(csrs.read(CsrAddress::Instret), 3);
        assert_eq!(csrs.read(CsrAddress::Instreth), 2);
    }

    #[test]
    fn counter_high_halves_preserve_other_half_on_write_and_increment_carries() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mcycleh, 4);
        csrs.write(CsrAddress::Mcycle, u32::MAX);
        csrs.write(CsrAddress::Minstreth, 7);
        csrs.write(CsrAddress::Minstret, u32::MAX);

        csrs.increment_cycle();
        csrs.increment_instret(2);

        assert_eq!(csrs.read(CsrAddress::Mcycle), 0);
        assert_eq!(csrs.read(CsrAddress::Mcycleh), 5);
        assert_eq!(csrs.read(CsrAddress::Time), 0);
        assert_eq!(csrs.read(CsrAddress::Timeh), 5);
        assert_eq!(csrs.read(CsrAddress::Minstret), 1);
        assert_eq!(csrs.read(CsrAddress::Minstreth), 8);
    }

    #[test]
    fn counteren_writes_are_masked_to_modeled_bits() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mcounteren, u32::MAX);
        csrs.write(CsrAddress::Scounteren, u32::MAX);

        assert_eq!(csrs.read(CsrAddress::Mcounteren), super::COUNTEREN_MASK);
        assert_eq!(csrs.read(CsrAddress::Scounteren), super::COUNTEREN_MASK);
    }
}
