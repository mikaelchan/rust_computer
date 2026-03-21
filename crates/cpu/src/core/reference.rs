use rvsim_isa::{Exception, Trap, decode};
use rvsim_system::{Bus, BusError, CpuCycle, Processor, SimComponent};

use crate::{
    core::{CpuError, CpuModel},
    exec::{ExecutionResult, apply_trap, execute_decoded},
    state::HartState,
};

/// A simple architectural reference model that retires one instruction per cycle.
#[derive(Debug, Clone)]
pub struct ReferenceCore {
    reset_vector: u32,
    cycle: u64,
    state: HartState,
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
            let raw = match bus.fetch32(u64::from(pc)) {
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

        self.last_result = execute_decoded(&mut self.state, bus, decoded)?;
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
    use rvsim_devices::{
        InterruptController, LatencyAdapter, MachineSoftwareInterrupt, MachineTimer, Ram, Rom,
        SupervisorSoftwareInterrupt,
    };
    use rvsim_system::{
        AddressRange, Bus, CacheConfig, InterruptLine, InterruptSet, Machine, MemoryMap, Processor,
        SplitL1Cache, StoreAllocationPolicy, WritePolicy,
    };

    use super::ReferenceCore;
    use crate::core::CpuModel;
    use crate::state::RegisterFile;

    #[derive(Debug)]
    struct TinyBus {
        bytes: [u8; 128],
    }

    impl Default for TinyBus {
        fn default() -> Self {
            Self { bytes: [0; 128] }
        }
    }

    impl TinyBus {
        fn load_program(&mut self, words: &[u32]) {
            for (word_index, word) in words.iter().copied().enumerate() {
                let base = word_index * 4;
                self.bytes[base..base + 4].copy_from_slice(&word.to_le_bytes());
            }
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
}
