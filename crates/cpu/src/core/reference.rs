use rvsim_isa::{Exception, Trap, decode};
use rvsim_system::{Bus, BusError, CpuCycle, Processor, SimComponent};

use crate::{
    core::{CpuError, CpuModel},
    exec::{ExecutionResult, apply_trap, execute_decoded},
    mmu::{MemoryAccess, PageWalker, TranslationResult},
    state::HartState,
};

/// A simple architectural reference model that retires one instruction per cycle.
#[derive(Debug, Clone)]
pub struct ReferenceCore {
    reset_vector: u32,
    cycle: u64,
    state: HartState,
    page_walker: PageWalker,
    pending_decoded: Option<rvsim_isa::DecodedInstruction>,
    last_result: ExecutionResult,
}

impl ReferenceCore {
    #[must_use]
    pub fn new(reset_vector: u32) -> Self {
        Self {
            reset_vector,
            cycle: 0,
            state: HartState::new(reset_vector),
            page_walker: PageWalker::default(),
            pending_decoded: None,
            last_result: ExecutionResult::default(),
        }
    }

    #[must_use]
    pub fn last_trap(&self) -> Option<Trap> {
        self.last_result.trap
    }
}

impl CpuModel for ReferenceCore {
    fn hart_state(&self) -> &HartState {
        &self.state
    }

    fn hart_state_mut(&mut self) -> &mut HartState {
        &mut self.state
    }

    fn model_name(&self) -> &'static str {
        "reference"
    }
}

impl SimComponent for ReferenceCore {
    fn reset(&mut self) {
        self.cycle = 0;
        self.state.reset(self.reset_vector);
        self.page_walker.reset();
        self.pending_decoded = None;
        self.last_result = ExecutionResult::default();
    }
}

impl Processor for ReferenceCore {
    type Error = CpuError;

    fn cycle(&self) -> u64 {
        self.cycle
    }

    fn step_cycle(&mut self, bus: &mut dyn Bus) -> Result<CpuCycle, Self::Error> {
        self.cycle += 1;
        self.state.csrs.increment_cycle();
        self.state.csrs.sync_interrupts(bus.pending_interrupts());

        if self.state.halted {
            return Ok(CpuCycle {
                retired_instructions: 0,
                stalled: true,
            });
        }

        if self.pending_decoded.is_none()
            && !bus.is_busy()
            && let Some(interrupt) = self.state.csrs.pending_interrupt(self.state.privilege)
        {
            let current_pc = self.state.pc;
            self.last_result = apply_trap(&mut self.state, Trap::Interrupt(interrupt), current_pc);
            return Ok(CpuCycle {
                retired_instructions: 0,
                stalled: true,
            });
        }

        let decoded = if let Some(decoded) = self.pending_decoded {
            decoded
        } else {
            let pc = self.state.pc;
            let physical_address = match self.page_walker.translate(
                bus,
                &self.state.csrs,
                self.state.privilege,
                pc,
                MemoryAccess::Instruction,
            )? {
                TranslationResult::PhysicalAddress(physical_address) => physical_address,
                TranslationResult::Stall => {
                    self.last_result = ExecutionResult {
                        retired: 0,
                        trap: None,
                        memory_access: true,
                    };
                    return Ok(CpuCycle {
                        retired_instructions: 0,
                        stalled: true,
                    });
                }
                TranslationResult::PageFault(trap) => {
                    self.last_result = apply_trap(&mut self.state, trap, pc);
                    return Ok(CpuCycle {
                        retired_instructions: 0,
                        stalled: true,
                    });
                }
            };
            let raw = match bus.fetch32(u64::from(physical_address)) {
                Ok(raw) => raw,
                Err(BusError::Busy { .. }) => {
                    self.last_result = ExecutionResult {
                        retired: 0,
                        trap: None,
                        memory_access: true,
                    };
                    return Ok(CpuCycle {
                        retired_instructions: 0,
                        stalled: true,
                    });
                }
                Err(BusError::MisalignedAccess { .. }) => {
                    self.last_result = apply_trap(
                        &mut self.state,
                        Trap::Exception(Exception::InstructionAddressMisaligned { addr: pc }),
                        pc,
                    );
                    return Ok(CpuCycle {
                        retired_instructions: 0,
                        stalled: true,
                    });
                }
                Err(error) => return Err(error.into()),
            };

            match decode(raw, pc) {
                Ok(decoded) => decoded,
                Err(_error) => {
                    self.last_result = apply_trap(
                        &mut self.state,
                        Trap::Exception(Exception::IllegalInstruction { instruction: raw }),
                        pc,
                    );
                    return Ok(CpuCycle {
                        retired_instructions: 0,
                        stalled: true,
                    });
                }
            }
        };

        self.last_result = execute_decoded(&mut self.state, &mut self.page_walker, bus, decoded)?;
        self.pending_decoded =
            (self.last_result.retired == 0 && self.last_result.trap.is_none()).then_some(decoded);

        Ok(CpuCycle {
            retired_instructions: self.last_result.retired,
            stalled: self.last_result.retired == 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use rvsim_devices::{
        DmaController, InterruptController, LatencyAdapter, MachineSoftwareInterrupt, MachineTimer,
        Ram, Rom, SupervisorSoftwareInterrupt,
    };
    use rvsim_system::{
        AddressRange, ArbiterBus, Bus, CacheConfig, InterruptLine, InterruptSet, Machine,
        MemoryMap, Processor, SplitL1Cache, StoreAllocationPolicy, WritePolicy,
    };

    use super::ReferenceCore;
    use crate::core::CpuModel;
    use crate::state::RegisterFile;

    #[derive(Debug)]
    struct TinyBus {
        bytes: Vec<u8>,
    }

    impl Default for TinyBus {
        fn default() -> Self {
            Self::new(0x10_000)
        }
    }

    impl TinyBus {
        fn new(size: usize) -> Self {
            Self {
                bytes: vec![0; size],
            }
        }

        fn load_program(&mut self, words: &[u32]) {
            self.store_words(0, words);
        }

        fn store_words(&mut self, base: u32, words: &[u32]) {
            for (word_index, word) in words.iter().copied().enumerate() {
                let offset = base as usize + word_index * 4;
                self.bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            }
        }

        fn store_word(&mut self, addr: u32, word: u32) {
            let offset = addr as usize;
            self.bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }

        fn read_word(&self, addr: u32) -> u32 {
            let offset = addr as usize;
            u32::from_le_bytes(self.bytes[offset..offset + 4].try_into().unwrap_or([0; 4]))
        }
    }

    impl Bus for TinyBus {
        fn load8(&mut self, addr: u64) -> Result<u8, rvsim_system::BusError> {
            Ok(self.bytes[addr as usize])
        }

        fn store8(&mut self, addr: u64, value: u8) -> Result<(), rvsim_system::BusError> {
            self.bytes[addr as usize] = value;
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct InterruptBus {
        pending_interrupts: InterruptSet,
    }

    impl Bus for InterruptBus {
        fn load8(&mut self, _addr: u64) -> Result<u8, rvsim_system::BusError> {
            Ok(0)
        }

        fn store8(&mut self, _addr: u64, _value: u8) -> Result<(), rvsim_system::BusError> {
            Ok(())
        }

        fn pending_interrupts(&self) -> InterruptSet {
            self.pending_interrupts
        }
    }

    #[derive(Debug)]
    struct FetchOnlyBus {
        instruction: u32,
    }

    impl Bus for FetchOnlyBus {
        fn load8(&mut self, _addr: u64) -> Result<u8, rvsim_system::BusError> {
            panic!("instruction fetch should use fetch32, not byte loads")
        }

        fn store8(&mut self, _addr: u64, _value: u8) -> Result<(), rvsim_system::BusError> {
            panic!("test program does not perform stores")
        }

        fn fetch32(&mut self, addr: u64) -> Result<u32, rvsim_system::BusError> {
            assert_eq!(addr, 0);
            Ok(self.instruction)
        }
    }

    #[test]
    fn runs_addi_program() {
        let _ = AddressRange::new(0, 16);
        let mut bus = TinyBus::default();
        bus.load_program(&[0x0050_0093, 0x00a0_0113]);

        let mut core = ReferenceCore::new(0);
        core.step_cycle(&mut bus).expect("first cycle should work");
        core.step_cycle(&mut bus).expect("second cycle should work");

        let state = core.hart_state();
        assert_eq!(state.registers.read(1), 5);
        assert_eq!(state.registers.read(2), 10);
        assert_eq!(state.pc, 8);
        assert_eq!(RegisterFile::NUM_REGISTERS, 32);
    }

    #[test]
    fn fetches_instructions_via_bus_fetch_path() {
        let mut bus = FetchOnlyBus {
            instruction: encode_addi(1, 0, 5),
        };
        let mut core = ReferenceCore::new(0);

        let cycle = core.step_cycle(&mut bus).expect("cycle should work");

        assert_eq!(cycle.retired_instructions, 1);
        assert_eq!(core.hart_state().registers.read(1), 5);
        assert_eq!(core.hart_state().pc, 4);
    }

    #[test]
    fn executes_csr_read_write_sequence() {
        let mut bus = TinyBus::default();
        bus.load_program(&[encode_csrrwi(1, rvsim_isa::CsrAddress::Mtvec as u16, 7)]);

        let mut core = ReferenceCore::new(0);
        core.step_cycle(&mut bus).expect("csr cycle should work");

        let state = core.hart_state();
        assert_eq!(state.registers.read(1), 0);
        assert_eq!(state.csrs.read(rvsim_isa::CsrAddress::Mtvec), 7);
    }

    #[test]
    fn fetches_instructions_through_sv32_translation() {
        let mut bus = TinyBus::default();
        bus.store_words(0x0000, &[encode_addi(1, 0, 5), encode_jal(0, 0)]);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..4 {
            core.step_cycle(&mut bus)
                .expect("sv32 instruction fetch should execute");
        }

        assert_eq!(core.hart_state().registers.read(1), 5);
        assert_eq!(core.hart_state().pc, 0x4004);
    }

    #[test]
    fn executes_load_store_through_sv32_translation() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[
                encode_lui(1, 0x8),
                encode_addi(2, 0, 9),
                encode_sw(2, 1, 0),
                encode_lw(3, 1, 0),
                encode_jal(0, 0),
            ],
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_W | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("sv32 load/store flow should execute");
        }

        assert_eq!(core.hart_state().registers.read(3), 9);
        assert_eq!(bus.read_word(0x1000), 9);
    }

    #[test]
    fn hardware_manages_accessed_and_dirty_bits_for_data_pages() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[
                encode_lui(1, 0x8),
                encode_addi(2, 0, 9),
                encode_sw(2, 1, 0),
                encode_lw(3, 1, 0),
                encode_jal(0, 0),
            ],
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(&mut bus, 0x2000, 0x3000, 0x8000, 0x1000, PTE_R | PTE_W);

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("sv32 load/store flow should update ad bits");
        }

        assert_eq!(core.hart_state().registers.read(3), 9);
        assert_eq!(bus.read_word(0x1000), 9);
        assert_eq!(bus.read_word(0x3020) & (PTE_A | PTE_D), PTE_A | PTE_D);
    }

    #[test]
    fn supervisor_sum_allows_loading_user_pages() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[encode_lui(1, 0x8), encode_lw(2, 1, 0), encode_jal(0, 0)],
        );
        bus.store_word(0x1000, 9);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_U | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sstatus, MSTATUS_SUM);

        for _ in 0..6 {
            core.step_cycle(&mut bus)
                .expect("sum-enabled supervisor load should execute");
        }

        assert_eq!(core.hart_state().registers.read(2), 9);
    }

    #[test]
    fn supervisor_mxr_allows_loading_execute_only_pages() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[encode_lui(1, 0x8), encode_lw(2, 1, 0), encode_jal(0, 0)],
        );
        bus.store_word(0x1000, 9);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(&mut bus, 0x2000, 0x3000, 0x8000, 0x1000, PTE_X | PTE_A);

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sstatus, MSTATUS_MXR);

        for _ in 0..6 {
            core.step_cycle(&mut bus)
                .expect("mxr-enabled supervisor load should execute");
        }

        assert_eq!(core.hart_state().registers.read(2), 9);
    }

    #[test]
    fn machine_mprv_load_uses_supervisor_translation() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[encode_lui(1, 0x8), encode_lw(2, 1, 0), encode_jal(0, 0)],
        );
        bus.store_word(0x1000, 9);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x0000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Machine;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut().csrs.write(
            rvsim_isa::CsrAddress::Mstatus,
            MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT),
        );

        for _ in 0..4 {
            core.step_cycle(&mut bus)
                .expect("mprv-enabled machine load should execute");
        }

        assert_eq!(core.hart_state().registers.read(2), 9);
    }

    #[test]
    fn machine_mprv_supervisor_requires_sum_for_user_page_loads() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[encode_lui(1, 0x8), encode_lw(2, 1, 0), encode_jal(0, 0)],
        );
        bus.store_word(0x1000, 9);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_U | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x0000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Machine;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);
        core.hart_state_mut().csrs.write(
            rvsim_isa::CsrAddress::Mstatus,
            MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT),
        );

        core.step_cycle(&mut bus)
            .expect("setup instruction should execute");
        let cycle = core
            .step_cycle(&mut bus)
            .expect("mprv load without sum should trap");

        assert_eq!(cycle.retired_instructions, 0);
        assert!(cycle.stalled);
        assert_eq!(core.hart_state().registers.read(2), 0);
        assert_eq!(core.hart_state().pc, 0x20);
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            13
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mtval),
            0x8000
        );
        assert_eq!(core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mepc), 4);
    }

    #[test]
    fn machine_mprv_supervisor_sum_allows_user_page_loads() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[encode_lui(1, 0x8), encode_lw(2, 1, 0), encode_jal(0, 0)],
        );
        bus.store_word(0x1000, 9);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_U | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x0000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Machine;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut().csrs.write(
            rvsim_isa::CsrAddress::Mstatus,
            MSTATUS_MPRV | MSTATUS_SUM | (1 << MSTATUS_MPP_SHIFT),
        );

        for _ in 0..4 {
            core.step_cycle(&mut bus)
                .expect("mprv sum-enabled machine load should execute");
        }

        assert_eq!(core.hart_state().registers.read(2), 9);
    }

    #[test]
    fn machine_mprv_supervisor_mxr_allows_execute_only_loads() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[encode_lui(1, 0x8), encode_lw(2, 1, 0), encode_jal(0, 0)],
        );
        bus.store_word(0x1000, 9);
        install_sv32_mapping(&mut bus, 0x2000, 0x3000, 0x8000, 0x1000, PTE_X | PTE_A);

        let mut core = ReferenceCore::new(0x0000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Machine;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut().csrs.write(
            rvsim_isa::CsrAddress::Mstatus,
            MSTATUS_MPRV | MSTATUS_MXR | (1 << MSTATUS_MPP_SHIFT),
        );

        for _ in 0..4 {
            core.step_cycle(&mut bus)
                .expect("mprv mxr-enabled machine load should execute");
        }

        assert_eq!(core.hart_state().registers.read(2), 9);
    }

    #[test]
    fn executes_fetch_and_data_access_through_sv32_superpage() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[
                encode_lui(1, 0x408),
                encode_addi(2, 0, 9),
                encode_sw(2, 1, 0),
                encode_lw(3, 1, 0),
                encode_jal(0, 0),
            ],
        );
        install_sv32_superpage_mapping(
            &mut bus,
            0x2000,
            0x400000,
            0x000000,
            PTE_R | PTE_W | PTE_X | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x400000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("sv32 superpage fetch/data flow should execute");
        }

        assert_eq!(core.hart_state().registers.read(3), 9);
        assert_eq!(bus.read_word(0x8000), 9);
        assert_eq!(core.hart_state().pc, 0x400010);
    }

    #[test]
    fn satp_write_switches_address_space() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[
                encode_lui(1, 0x8),
                encode_lw(2, 1, 0),
                encode_lui(3, 0x80000),
                encode_addi(3, 3, 5),
                encode_csrrw(0, rvsim_isa::CsrAddress::Satp as u16, 3),
                encode_lw(4, 1, 0),
                encode_jal(0, 0),
            ],
        );
        bus.store_word(0x1000, 5);
        bus.store_word(0x7000, 9);

        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_A | PTE_D,
        );
        install_sv32_mapping(
            &mut bus,
            0x5000,
            0x6000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x5000,
            0x6000,
            0x8000,
            0x7000,
            PTE_R | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("satp switch should execute through reference core");
        }

        assert_eq!(core.hart_state().registers.read(2), 5);
        assert_eq!(core.hart_state().registers.read(4), 9);
    }

    #[test]
    fn satp_write_preserves_tlb_namespace_until_sfence_vma() {
        let satp_asid_1 = sv32_satp_with_asid(0x2000, 1);
        let satp_asid_2 = sv32_satp_with_asid(0x5000, 2);

        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[
                encode_lui(1, 0x8),
                encode_lw(2, 1, 0),
                encode_lui(3, satp_asid_2 >> 12),
                encode_addi(3, 3, (satp_asid_2 & 0x0fff) as i16),
                encode_csrrw(0, rvsim_isa::CsrAddress::Satp as u16, 3),
                encode_lw(4, 1, 0),
                encode_lui(5, satp_asid_1 >> 12),
                encode_addi(5, 5, (satp_asid_1 & 0x0fff) as i16),
                encode_csrrw(0, rvsim_isa::CsrAddress::Satp as u16, 5),
                encode_lw(6, 1, 0),
                encode_sfence_vma(0, 0),
                encode_lw(7, 1, 0),
                encode_jal(0, 0),
            ],
        );
        bus.store_word(0x1000, 5);
        bus.store_word(0x7000, 7);
        bus.store_word(0x9000, 9);

        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_A | PTE_D,
        );
        install_sv32_mapping(
            &mut bus,
            0x5000,
            0x6000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x5000,
            0x6000,
            0x8000,
            0x7000,
            PTE_R | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, satp_asid_1);

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("first ASID load should execute");
            if core.hart_state().registers.read(2) == 5 {
                break;
            }
        }
        assert_eq!(core.hart_state().registers.read(2), 5);

        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x9000,
            PTE_R | PTE_A | PTE_D,
        );

        for _ in 0..16 {
            core.step_cycle(&mut bus)
                .expect("satp namespace preservation flow should execute");
            if core.hart_state().registers.read(7) == 9 {
                break;
            }
        }

        assert_eq!(core.hart_state().registers.read(4), 7);
        assert_eq!(core.hart_state().registers.read(6), 5);
        assert_eq!(core.hart_state().registers.read(7), 9);
    }

    #[test]
    fn asid_specific_sfence_vma_preserves_global_mapping() {
        let satp_asid_1 = sv32_satp_with_asid(0x2000, 1);
        let satp_asid_2 = sv32_satp_with_asid(0x5000, 2);

        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[
                encode_lui(1, 0x8),
                encode_lw(2, 1, 0),
                encode_addi(4, 0, 1),
                encode_sfence_vma(0, 4),
                encode_lui(3, satp_asid_2 >> 12),
                encode_addi(3, 3, (satp_asid_2 & 0x0fff) as i16),
                encode_csrrw(0, rvsim_isa::CsrAddress::Satp as u16, 3),
                encode_lw(5, 1, 0),
                encode_sfence_vma(0, 0),
                encode_lw(6, 1, 0),
                encode_jal(0, 0),
            ],
        );
        bus.store_word(0x1000, 5);
        bus.store_word(0x3000, 9);

        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_A | PTE_D | PTE_G,
        );
        install_sv32_mapping(
            &mut bus,
            0x5000,
            0x6000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x5000,
            0x6000,
            0x8000,
            0x1000,
            PTE_R | PTE_A | PTE_D | PTE_G,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, satp_asid_1);

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("initial global translation should execute");
            if core.hart_state().registers.read(2) == 5 {
                break;
            }
        }
        assert_eq!(core.hart_state().registers.read(2), 5);

        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x3000,
            PTE_R | PTE_A | PTE_D | PTE_G,
        );
        install_sv32_mapping(
            &mut bus,
            0x5000,
            0x6000,
            0x8000,
            0x3000,
            PTE_R | PTE_A | PTE_D | PTE_G,
        );

        for _ in 0..16 {
            core.step_cycle(&mut bus)
                .expect("global mapping flow should execute");
            if core.hart_state().registers.read(6) == 9 {
                break;
            }
        }

        assert_eq!(core.hart_state().registers.read(5), 5);
        assert_eq!(core.hart_state().registers.read(6), 9);
    }

    #[test]
    fn sfence_vma_flushes_stale_translation() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[
                encode_lui(1, 0x8),
                encode_lw(2, 1, 0),
                encode_addi(0, 0, 0),
                encode_addi(0, 0, 0),
                encode_sfence_vma(0, 0),
                encode_lw(3, 1, 0),
                encode_jal(0, 0),
            ],
        );
        bus.store_word(0x1000, 5);
        bus.store_word(0x2000, 9);
        install_sv32_mapping(
            &mut bus,
            0x3000,
            0x4000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x3000,
            0x4000,
            0x8000,
            0x1000,
            PTE_R | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x3000 >> 12));

        for _ in 0..3 {
            core.step_cycle(&mut bus)
                .expect("first translated load should execute");
            if core.hart_state().registers.read(2) == 5 {
                break;
            }
        }
        assert_eq!(core.hart_state().registers.read(2), 5);

        install_sv32_mapping(
            &mut bus,
            0x3000,
            0x4000,
            0x8000,
            0x2000,
            PTE_R | PTE_A | PTE_D,
        );

        for _ in 0..6 {
            core.step_cycle(&mut bus)
                .expect("sfence.vma flow should observe remapped page");
        }

        assert_eq!(core.hart_state().registers.read(3), 9);
    }

    #[test]
    fn sfence_vma_can_flush_one_virtual_address_only() {
        let mut bus = TinyBus::default();
        bus.store_words(
            0x0000,
            &[
                encode_lui(1, 0x8),
                encode_lw(2, 1, 0),
                encode_lui(3, 0x9),
                encode_lw(4, 3, 0),
                encode_sfence_vma(1, 0),
                encode_lw(5, 1, 0),
                encode_lw(6, 3, 0),
                encode_jal(0, 0),
            ],
        );
        bus.store_word(0x1000, 5);
        bus.store_word(0x5000, 7);
        bus.store_word(0x7000, 11);
        bus.store_word(0x8000, 13);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x1000,
            PTE_R | PTE_A | PTE_D,
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x9000,
            0x5000,
            PTE_R | PTE_A | PTE_D,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..4 {
            core.step_cycle(&mut bus)
                .expect("initial translated loads should execute");
            if core.hart_state().registers.read(4) == 7 {
                break;
            }
        }
        assert_eq!(core.hart_state().registers.read(2), 5);
        assert_eq!(core.hart_state().registers.read(4), 7);

        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x8000,
            0x7000,
            PTE_R | PTE_A | PTE_D,
        );
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x9000,
            0x8000,
            PTE_R | PTE_A | PTE_D,
        );

        for _ in 0..4 {
            core.step_cycle(&mut bus)
                .expect("selective sfence.vma should execute");
        }

        assert_eq!(core.hart_state().registers.read(5), 11);
        assert_eq!(core.hart_state().registers.read(6), 7);
    }

    #[test]
    fn traps_on_instruction_page_fault_during_sv32_fetch() {
        let mut bus = TinyBus::default();
        bus.store_words(0x0080, &[encode_addi(10, 0, 1), encode_jal(0, 0)]);

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x80);

        core.step_cycle(&mut bus)
            .expect("instruction page fault should trap");
        core.step_cycle(&mut bus)
            .expect("machine handler should execute after page fault");

        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::Machine
        );
        assert_eq!(core.hart_state().registers.read(10), 1);
        assert_eq!(core.hart_state().pc, 0x84);
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            12
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mepc),
            0x4000
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mtval),
            0x4000
        );
    }

    #[test]
    fn delegates_user_ecall_to_supervisor_handler_and_returns_with_sret() {
        let mut bus = TinyBus::default();
        bus.load_program(&[
            encode_ecall(),
            encode_addi(1, 0, 1),
            encode_jal(0, 0),
            0,
            0,
            0,
            0,
            0,
            encode_csrrwi(0, rvsim_isa::CsrAddress::Sepc as u16, 4),
            encode_addi(2, 0, 7),
            encode_sret(),
            encode_jal(0, 0),
        ]);

        let mut core = ReferenceCore::new(0);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::User;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Medeleg, 1 << 8);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Stvec, 0x20);

        for _ in 0..6 {
            core.step_cycle(&mut bus)
                .expect("delegated supervisor trap flow should execute");
        }

        assert_eq!(core.hart_state().registers.read(1), 1);
        assert_eq!(core.hart_state().registers.read(2), 7);
        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::User
        );
        assert_eq!(core.hart_state().pc, 8);
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Scause),
            8
        );
        assert_eq!(core.hart_state().csrs.read(rvsim_isa::CsrAddress::Sepc), 4);
    }

    #[test]
    fn user_mode_machine_csr_access_traps_as_illegal_instruction() {
        let instruction = encode_csrrwi(1, rvsim_isa::CsrAddress::Mstatus as u16, 1);
        let mut bus = TinyBus::default();
        bus.load_program(&[instruction]);

        let mut core = ReferenceCore::new(0);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::User;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);

        let cycle = core
            .step_cycle(&mut bus)
            .expect("illegal csr access should trap");

        assert_eq!(cycle.retired_instructions, 0);
        assert!(cycle.stalled);
        assert_eq!(core.hart_state().registers.read(1), 0);
        assert_eq!(core.hart_state().pc, 0x20);
        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::Machine
        );
        assert_eq!(core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mepc), 0);
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            2
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mtval),
            instruction
        );
    }

    #[test]
    fn supervisor_satp_access_traps_when_tvm_is_set() {
        let instruction = encode_csrrw(1, rvsim_isa::CsrAddress::Satp as u16, 0);
        let mut bus = TinyBus::default();
        bus.load_program(&[instruction]);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );

        let mut core = ReferenceCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, MSTATUS_TVM);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        let cycle = core
            .step_cycle(&mut bus)
            .expect("tvm should trap supervisor satp access");

        assert_eq!(cycle.retired_instructions, 0);
        assert!(cycle.stalled);
        assert_eq!(core.hart_state().registers.read(1), 0);
        assert_eq!(core.hart_state().pc, 0x20);
        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::Machine
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Satp),
            SATP_MODE_SV32 | (0x2000 >> 12)
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mepc),
            0x4000
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            2
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mtval),
            instruction
        );
    }

    #[test]
    fn supervisor_sfence_vma_traps_when_tvm_is_set() {
        let instruction = encode_sfence_vma(0, 0);
        let mut bus = TinyBus::default();
        bus.load_program(&[instruction]);

        let mut core = ReferenceCore::new(0);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, MSTATUS_TVM);

        let cycle = core
            .step_cycle(&mut bus)
            .expect("tvm should trap supervisor sfence.vma");

        assert_eq!(cycle.retired_instructions, 0);
        assert!(cycle.stalled);
        assert_eq!(core.hart_state().pc, 0x20);
        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::Machine
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            2
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mtval),
            instruction
        );
    }

    #[test]
    fn supervisor_sret_traps_when_tsr_is_set() {
        let instruction = encode_sret();
        let mut bus = TinyBus::default();
        bus.load_program(&[instruction]);

        let mut core = ReferenceCore::new(0);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, MSTATUS_TSR);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sepc, 0x40);

        let cycle = core
            .step_cycle(&mut bus)
            .expect("tsr should trap supervisor sret");

        assert_eq!(cycle.retired_instructions, 0);
        assert!(cycle.stalled);
        assert_eq!(core.hart_state().pc, 0x20);
        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::Machine
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            2
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mtval),
            instruction
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Sepc),
            0x40
        );
    }

    #[test]
    fn takes_machine_timer_interrupt_when_enabled() {
        let mut bus = InterruptBus {
            pending_interrupts: InterruptSet::from(InterruptLine::MachineTimer),
        };
        let mut core = ReferenceCore::new(0);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, 1 << 3);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mie, 1 << 7);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x40);

        let report = core
            .step_cycle(&mut bus)
            .expect("interrupt cycle should work");

        assert_eq!(report.retired_instructions, 0);
        assert_eq!(core.hart_state().pc, 0x40);
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 7
        );
    }

    #[test]
    fn takes_machine_software_interrupt_when_enabled() {
        let mut bus = InterruptBus {
            pending_interrupts: InterruptSet::from(InterruptLine::MachineSoftware),
        };
        let mut core = ReferenceCore::new(0);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, 1 << 3);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mie, 1 << 3);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x40);

        let report = core
            .step_cycle(&mut bus)
            .expect("software interrupt cycle should work");

        assert_eq!(report.retired_instructions, 0);
        assert_eq!(core.hart_state().pc, 0x40);
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 3
        );
    }

    #[test]
    fn interrupt_priority_prefers_machine_external_over_software_and_timer() {
        let mut bus = InterruptBus {
            pending_interrupts: InterruptSet::from(InterruptLine::MachineSoftware)
                .union(InterruptSet::from(InterruptLine::MachineTimer))
                .union(InterruptSet::from(InterruptLine::MachineExternal)),
        };
        let mut core = ReferenceCore::new(0);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, 1 << 3);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mie, (1 << 7) | (1 << 11));
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x40);

        core.step_cycle(&mut bus)
            .expect("interrupt priority cycle should work");

        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 11
        );
    }

    #[test]
    fn machine_timer_device_interrupts_through_machine_wrapper() {
        const TIMER_BASE: u64 = 0x3000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(Rom::from_words(
                0,
                &[
                    encode_jal(0, 0),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    encode_addi(10, 0, 1),
                    encode_jal(0, 0),
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(MachineTimer::new(TIMER_BASE))
            .expect("timer should map");

        let mut machine = Machine::new(ReferenceCore::new(0), memory);
        machine
            .bus_mut()
            .store32(TIMER_BASE + 8, 1)
            .expect("mtimecmp low should write");
        machine
            .bus_mut()
            .store32(TIMER_BASE + 12, 0)
            .expect("mtimecmp high should write");
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, 1 << 3);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mie, 1 << 7);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);

        machine.step_cycle().expect("interrupt should be taken");
        machine
            .step_cycle()
            .expect("handler instruction should run");

        assert_eq!(machine.cpu().hart_state().pc, 0x24);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 7
        );
    }

    #[test]
    fn interrupt_controller_device_interrupts_through_machine_wrapper() {
        const CONTROLLER_BASE: u64 = 0x4000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(Rom::from_words(
                0,
                &[
                    encode_jal(0, 0),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    encode_addi(10, 0, 2),
                    encode_jal(0, 0),
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(InterruptController::new(CONTROLLER_BASE))
            .expect("controller should map");

        let mut machine = Machine::new(ReferenceCore::new(0), memory);
        machine
            .bus_mut()
            .store32(CONTROLLER_BASE + 4, 1)
            .expect("enable register should write");
        machine
            .bus_mut()
            .store32(CONTROLLER_BASE + 8, 1)
            .expect("set-pending register should write");
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, 1 << 3);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mie, 1 << 11);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);

        machine
            .step_cycle()
            .expect("external interrupt should be taken");
        machine
            .step_cycle()
            .expect("handler instruction should run");

        assert_eq!(machine.cpu().hart_state().pc, 0x24);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 2);
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 11
        );
    }

    #[test]
    fn supervisor_external_interrupt_controller_delegates_through_supervisor_handler() {
        const CONTROLLER_BASE: u64 = 0x4000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(Rom::from_words(
                0,
                &[
                    encode_addi(1, 0, 5),
                    encode_jal(0, 0),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    encode_lui(2, 0x40000),
                    encode_lw(3, 2, 12),
                    encode_sw(3, 2, 12),
                    encode_addi(10, 0, 5),
                    encode_sret(),
                    encode_jal(0, 0),
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(InterruptController::new(CONTROLLER_BASE))
            .expect("controller should map");

        let mut machine = Machine::new(ReferenceCore::new(0), memory);
        machine
            .bus_mut()
            .store32(CONTROLLER_BASE + 4, 1)
            .expect("enable register should write");
        machine
            .bus_mut()
            .store32(CONTROLLER_BASE + 16, 1)
            .expect("route register should write");
        machine
            .bus_mut()
            .store32(CONTROLLER_BASE + 8, 1)
            .expect("set-pending register should write");
        machine.cpu_mut().hart_state_mut().privilege = crate::state::PrivilegeMode::User;
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mideleg, 1 << 9);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sie, 1 << 9);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Stvec, 0x20);

        for _ in 0..7 {
            machine
                .step_cycle()
                .expect("supervisor external interrupt handler should execute");
        }

        assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 5);
        assert_eq!(
            machine.cpu().hart_state().privilege,
            crate::state::PrivilegeMode::User
        );
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Scause),
            (1_u32 << 31) | 9
        );
        assert_eq!(
            machine
                .bus_mut()
                .load32(CONTROLLER_BASE)
                .expect("pending register should read"),
            0
        );
    }

    #[test]
    fn supervisor_external_dma_completion_interrupt_delegates_through_supervisor_handler() {
        const RAM_BASE: u64 = 0x1000_0000;
        const DMA_BASE: u64 = 0x7000_0000;

        let dma = Rc::new(RefCell::new(DmaController::new(DMA_BASE)));
        let mut memory = MemoryMap::new();
        memory
            .map_device(Ram::new(0, 0x100))
            .expect("program ram should map");
        memory
            .map_device(Ram::new(RAM_BASE, 0x100))
            .expect("ram should map");
        memory
            .map_shared_device(Rc::clone(&dma))
            .expect("dma should map");

        let mut bus = ArbiterBus::new(memory);
        bus.add_shared_master(Rc::clone(&dma));
        bus.store32(RAM_BASE, 0x1122_3344)
            .expect("source word 0 should write");
        bus.store32(RAM_BASE + 4, 0x5566_7788)
            .expect("source word 1 should write");
        let mut machine = Machine::new(ReferenceCore::new(0), bus);
        machine.cpu_mut().hart_state_mut().privilege = crate::state::PrivilegeMode::User;
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mideleg, 1 << 9);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sie, 1 << 9);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Stvec, 0x20);
        for (index, instruction) in [
            encode_addi(1, 0, 5),
            encode_jal(0, 0),
            0,
            0,
            0,
            0,
            0,
            0,
            encode_lui(2, 0x70000),
            encode_addi(3, 0, 12),
            encode_sw(3, 2, 12),
            encode_addi(10, 0, 6),
            encode_sret(),
            encode_jal(0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            machine
                .bus_mut()
                .store32((index as u64) * 4, instruction)
                .expect("program word should write");
        }

        for _ in 0..2 {
            machine
                .step_cycle()
                .expect("reference warmup cycle should execute");
        }

        machine
            .bus_mut()
            .store32(DMA_BASE + DmaController::SOURCE_OFFSET, RAM_BASE as u32)
            .expect("dma source should program");
        machine
            .bus_mut()
            .store32(
                DMA_BASE + DmaController::DESTINATION_OFFSET,
                (RAM_BASE + 0x40) as u32,
            )
            .expect("dma destination should program");
        machine
            .bus_mut()
            .store32(DMA_BASE + DmaController::LENGTH_OFFSET, 2)
            .expect("dma length should program");
        machine
            .bus_mut()
            .store32(DMA_BASE + DmaController::ROUTE_OFFSET, 1)
            .expect("dma route should program");
        machine
            .bus_mut()
            .store32(
                DMA_BASE + DmaController::CONTROL_OFFSET,
                DmaController::CONTROL_START | DmaController::CONTROL_IRQ_ENABLE,
            )
            .expect("dma control should start transfer");

        for _ in 0..32 {
            machine
                .step_cycle()
                .expect("supervisor dma interrupt flow should execute");
            if machine.cpu().hart_state().registers.read(10) == 6
                && matches!(
                    machine.cpu().hart_state().privilege,
                    crate::state::PrivilegeMode::User
                )
                && machine
                    .bus_mut()
                    .pending_interrupts()
                    .highest_priority()
                    .is_none()
            {
                break;
            }
        }

        assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 6);
        assert_eq!(
            machine.cpu().hart_state().privilege,
            crate::state::PrivilegeMode::User
        );
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Scause),
            (1_u32 << 31) | 9
        );
        assert_eq!(
            machine
                .bus_mut()
                .load32(DMA_BASE + DmaController::CONTROL_OFFSET)
                .expect("dma control should read")
                & DmaController::STATUS_DONE,
            0
        );
        assert_eq!(
            machine.bus_mut().pending_interrupts().highest_priority(),
            None
        );
        assert_eq!(
            machine
                .bus_mut()
                .load32(RAM_BASE + 0x40)
                .expect("copied word 0 should read"),
            0x1122_3344
        );
        assert_eq!(
            machine
                .bus_mut()
                .load32(RAM_BASE + 0x44)
                .expect("copied word 1 should read"),
            0x5566_7788
        );
    }

    #[test]
    fn machine_software_interrupt_device_interrupts_through_machine_wrapper() {
        const MSIP_BASE: u64 = 0x5000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(Rom::from_words(
                0,
                &[
                    encode_jal(0, 0),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    encode_addi(10, 0, 3),
                    encode_jal(0, 0),
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(MachineSoftwareInterrupt::new(MSIP_BASE))
            .expect("msip device should map");

        let mut machine = Machine::new(ReferenceCore::new(0), memory);
        machine
            .bus_mut()
            .store32(MSIP_BASE, 1)
            .expect("msip register should write");
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, 1 << 3);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mie, 1 << 3);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);

        machine
            .step_cycle()
            .expect("software interrupt should be taken");
        machine
            .step_cycle()
            .expect("handler instruction should run");

        assert_eq!(machine.cpu().hart_state().pc, 0x24);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 3);
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 3
        );
    }

    #[test]
    fn supervisor_software_interrupt_device_delegates_through_supervisor_handler() {
        const SSIP_BASE: u64 = 0x6000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(Rom::from_words(
                0,
                &[
                    encode_addi(1, 0, 5),
                    encode_jal(0, 0),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    encode_lui(2, 0x60000),
                    encode_sw(0, 2, 0),
                    encode_addi(10, 0, 4),
                    encode_sret(),
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(SupervisorSoftwareInterrupt::new(SSIP_BASE))
            .expect("ssip device should map");

        let mut machine = Machine::new(ReferenceCore::new(0), memory);
        machine
            .bus_mut()
            .store32(SSIP_BASE, 1)
            .expect("ssip register should write");
        machine.cpu_mut().hart_state_mut().privilege = crate::state::PrivilegeMode::User;
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mideleg, 1 << 1);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sie, 1 << 1);
        machine
            .cpu_mut()
            .hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Stvec, 0x20);

        for _ in 0..6 {
            machine
                .step_cycle()
                .expect("supervisor interrupt handler should execute");
        }

        assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 4);
        assert_eq!(
            machine.cpu().hart_state().privilege,
            crate::state::PrivilegeMode::User
        );
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Scause),
            (1_u32 << 31) | 1
        );
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Sepc),
            0
        );
    }

    #[test]
    fn stalls_on_instruction_fetch_latency_and_retries_same_pc() {
        let mut memory = MemoryMap::new();
        memory
            .map_device(LatencyAdapter::new(
                Rom::from_words(0, &[encode_addi(1, 0, 5), encode_jal(0, 0)]),
                1,
            ))
            .expect("rom should map");

        let mut machine = Machine::new(ReferenceCore::new(0), memory);

        let first = machine.step_cycle().expect("first cycle should stall");
        assert_eq!(first.retired_instructions, 0);
        assert!(first.stalled);
        assert_eq!(machine.cpu().hart_state().pc, 0);

        let second = machine.step_cycle().expect("second cycle should retire");
        assert_eq!(second.retired_instructions, 1);
        assert_eq!(machine.cpu().hart_state().pc, 4);
        assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
    }

    #[test]
    fn stalls_on_data_access_latency_and_retries_load_store() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(Rom::from_words(
                0,
                &[
                    encode_lui(1, 0x10000),
                    encode_addi(2, 0, 9),
                    encode_sw(2, 1, 0),
                    encode_lw(3, 1, 0),
                    encode_jal(0, 0),
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(LatencyAdapter::new(Ram::new(RAM_BASE, 0x1000), 2))
            .expect("ram should map");

        let mut machine = Machine::new(ReferenceCore::new(0), memory);
        let mut stalled_cycles = 0;
        for _ in 0..10 {
            let cycle = machine
                .step_cycle()
                .expect("reference cycle should work through ram latency");
            stalled_cycles += u64::from(cycle.stalled);
        }

        assert!(stalled_cycles >= 4);
        assert_eq!(machine.cpu().hart_state().registers.read(3), 9);
    }

    #[test]
    fn split_l1_cache_separates_instruction_and_data_paths() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(Rom::from_words(
                0,
                &[
                    encode_lui(1, 0x10000),
                    encode_addi(2, 0, 9),
                    encode_sw(2, 1, 0),
                    encode_lw(3, 1, 0),
                    encode_lw(4, 1, 0),
                    encode_jal(0, 0),
                    0,
                    0,
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(Ram::new(RAM_BASE, 0x1000))
            .expect("ram should map");

        let cache = SplitL1Cache::new(
            memory,
            CacheConfig::new(8, vec![AddressRange::new(0, 0x1000)]).with_line_size(16),
            CacheConfig::new(8, vec![AddressRange::new(RAM_BASE, 0x1000)])
                .with_line_size(16)
                .with_write_policy(WritePolicy::WriteBack)
                .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );

        let mut machine = Machine::new(ReferenceCore::new(0), cache);
        for _ in 0..6 {
            machine
                .step_cycle()
                .expect("reference cycle should work through split cache");
        }

        assert_eq!(machine.cpu().hart_state().registers.read(3), 9);
        assert_eq!(machine.cpu().hart_state().registers.read(4), 9);

        let stats = machine.bus().stats();
        assert_eq!(stats.instruction.read_misses, 2);
        assert_eq!(stats.instruction.refills, 2);
        assert!(stats.instruction.read_hits >= 6);
        assert_eq!(stats.data.refills, 1);
        assert!(stats.data.read_hits >= 2);
    }

    fn encode_csrrwi(rd: u8, csr: u16, zimm: u8) -> u32 {
        ((csr as u32) << 20)
            | ((zimm as u32) << 15)
            | (0b101 << 12)
            | ((rd as u32) << 7)
            | 0b1110011
    }

    fn encode_csrrw(rd: u8, csr: u16, rs1: u8) -> u32 {
        ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b001 << 12) | ((rd as u32) << 7) | 0b1110011
    }

    fn encode_ecall() -> u32 {
        0x0000_0073
    }

    fn encode_addi(rd: u8, rs1: u8, imm: i16) -> u32 {
        (((imm as u16 as u32) & 0x0fff) << 20)
            | ((rs1 as u32) << 15)
            | ((rd as u32) << 7)
            | 0b0010011
    }

    fn encode_sret() -> u32 {
        0x1020_0073
    }

    fn encode_sfence_vma(rs1: u8, rs2: u8) -> u32 {
        0x1200_0073 | ((rs1 as u32) << 15) | ((rs2 as u32) << 20)
    }

    const SATP_MODE_SV32: u32 = 1 << 31;
    const PTE_V: u32 = 1 << 0;
    const PTE_R: u32 = 1 << 1;
    const PTE_W: u32 = 1 << 2;
    const PTE_X: u32 = 1 << 3;
    const PTE_U: u32 = 1 << 4;
    const PTE_G: u32 = 1 << 5;
    const PTE_A: u32 = 1 << 6;
    const PTE_D: u32 = 1 << 7;
    const MSTATUS_MPRV: u32 = 1 << 17;
    const MSTATUS_SUM: u32 = 1 << 18;
    const MSTATUS_MXR: u32 = 1 << 19;
    const MSTATUS_TVM: u32 = 1 << 20;
    const MSTATUS_TSR: u32 = 1 << 22;
    const MSTATUS_MPP_SHIFT: u32 = 11;

    fn encode_lui(rd: u8, upper_20: u32) -> u32 {
        (upper_20 << 12) | ((rd as u32) << 7) | 0b0110111
    }

    fn encode_lw(rd: u8, rs1: u8, imm: i16) -> u32 {
        (((imm as u16 as u32) & 0x0fff) << 20)
            | ((rs1 as u32) << 15)
            | (0b010 << 12)
            | ((rd as u32) << 7)
            | 0b0000011
    }

    fn encode_sw(rs2: u8, rs1: u8, imm: i16) -> u32 {
        let imm = imm as u16 as u32;
        let imm_low = (imm & 0x1f) << 7;
        let imm_high = ((imm >> 5) & 0x7f) << 25;
        imm_high | ((rs2 as u32) << 20) | ((rs1 as u32) << 15) | (0b010 << 12) | imm_low | 0b0100011
    }

    fn encode_jal(rd: u8, imm: i32) -> u32 {
        let imm = imm as u32;
        let bit20 = ((imm >> 20) & 0x1) << 31;
        let bits10_1 = ((imm >> 1) & 0x03ff) << 21;
        let bit11 = ((imm >> 11) & 0x1) << 20;
        let bits19_12 = ((imm >> 12) & 0xff) << 12;
        bit20 | bits19_12 | bit11 | bits10_1 | ((rd as u32) << 7) | 0b1101111
    }

    fn install_sv32_mapping(
        bus: &mut TinyBus,
        root_table: u32,
        leaf_table: u32,
        virtual_page: u32,
        physical_page: u32,
        flags: u32,
    ) {
        let vpn1 = (virtual_page >> 22) & 0x3ff;
        let vpn0 = (virtual_page >> 12) & 0x3ff;
        bus.store_word(root_table + (vpn1 * 4), sv32_nonleaf(leaf_table));
        bus.store_word(
            leaf_table + (vpn0 * 4),
            sv32_leaf(physical_page, flags | PTE_V),
        );
    }

    fn install_sv32_superpage_mapping(
        bus: &mut TinyBus,
        root_table: u32,
        virtual_page: u32,
        physical_page: u32,
        flags: u32,
    ) {
        let vpn1 = (virtual_page >> 22) & 0x3ff;
        bus.store_word(
            root_table + (vpn1 * 4),
            sv32_leaf(physical_page, flags | PTE_V),
        );
    }

    fn sv32_nonleaf(next_table: u32) -> u32 {
        ((next_table >> 12) << 10) | PTE_V
    }

    fn sv32_leaf(physical_page: u32, flags: u32) -> u32 {
        ((physical_page >> 12) << 10) | flags
    }

    fn sv32_satp_with_asid(root_table: u32, asid: u32) -> u32 {
        SATP_MODE_SV32 | (asid << 22) | (root_table >> 12)
    }
}
