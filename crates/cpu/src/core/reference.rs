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
    last_result: ExecutionResult,
}

impl ReferenceCore {
    #[must_use]
    pub fn new(reset_vector: u32) -> Self {
        Self {
            reset_vector,
            cycle: 0,
            state: HartState::new(reset_vector),
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
        self.state.csrs.sync_interrupt_line(bus.pending_interrupt());

        if self.state.halted {
            return Ok(CpuCycle {
                retired_instructions: 0,
                stalled: true,
            });
        }

        if self.state.csrs.machine_timer_interrupt_enabled() {
            let current_pc = self.state.pc;
            self.last_result = apply_trap(
                &mut self.state,
                Trap::Interrupt(rvsim_isa::Interrupt::MachineTimer),
                current_pc,
            );
            return Ok(CpuCycle {
                retired_instructions: 0,
                stalled: true,
            });
        }

        let pc = self.state.pc;
        let raw = match bus.load32(u64::from(pc)) {
            Ok(raw) => raw,
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

        let decoded = match decode(raw, pc) {
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
        };

        self.last_result = execute_decoded(&mut self.state, bus, decoded)?;

        Ok(CpuCycle {
            retired_instructions: self.last_result.retired,
            stalled: self.last_result.retired == 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use rvsim_devices::{MachineTimer, Rom};
    use rvsim_system::{AddressRange, Bus, InterruptLine, Processor};
    use rvsim_system::{Machine, MemoryMap};

    use super::ReferenceCore;
    use crate::core::CpuModel;
    use crate::state::RegisterFile;

    #[derive(Debug, Default)]
    struct TinyBus {
        bytes: [u8; 16],
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
        pending_interrupt: Option<InterruptLine>,
    }

    impl Bus for InterruptBus {
        fn load8(&mut self, _addr: u64) -> Result<u8, rvsim_system::BusError> {
            Ok(0)
        }

        fn store8(&mut self, _addr: u64, _value: u8) -> Result<(), rvsim_system::BusError> {
            Ok(())
        }

        fn pending_interrupt(&self) -> Option<InterruptLine> {
            self.pending_interrupt
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
    fn takes_machine_timer_interrupt_when_enabled() {
        let mut bus = InterruptBus {
            pending_interrupt: Some(InterruptLine::MachineTimer),
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

    fn encode_csrrwi(rd: u8, csr: u16, zimm: u8) -> u32 {
        ((csr as u32) << 20)
            | ((zimm as u32) << 15)
            | (0b101 << 12)
            | ((rd as u32) << 7)
            | 0b1110011
    }

    fn encode_addi(rd: u8, rs1: u8, imm: i16) -> u32 {
        (((imm as u16 as u32) & 0x0fff) << 20)
            | ((rs1 as u32) << 15)
            | ((rd as u32) << 7)
            | 0b0010011
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
