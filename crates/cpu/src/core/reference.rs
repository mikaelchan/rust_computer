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

        if self.state.halted {
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
    use rvsim_system::{AddressRange, Bus, Processor};

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

    fn encode_csrrwi(rd: u8, csr: u16, zimm: u8) -> u32 {
        ((csr as u32) << 20)
            | ((zimm as u32) << 15)
            | (0b101 << 12)
            | ((rd as u32) << 7)
            | 0b1110011
    }
}
