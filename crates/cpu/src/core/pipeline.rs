use rvsim_isa::{Exception, Trap, opcode::InstructionKind};
use rvsim_system::{Bus, BusError, CpuCycle, Processor, SimComponent};

use crate::{
    core::{CpuError, CpuModel},
    exec::{ExecutionResult, apply_trap, execute_decoded},
    hazard::structural::{StructuralHazardPolicy, fetch_blocked_by_memory_access},
    pipeline::{
        ex_stage::classify,
        id_stage::decode_stage,
        if_stage::fetch,
        latches::{ExMemLatch, IdExLatch, IfIdLatch, MemWbLatch, PipelineLatches},
    },
    predictor::{AlwaysNotTaken, BranchPredictor},
    state::HartState,
    trace::PipelineTrace,
};

/// A five-stage pipeline scaffold with explicit latch state and prediction hooks.
#[derive(Debug, Clone)]
pub struct PipelineCore {
    reset_vector: u32,
    cycle: u64,
    state: HartState,
    predictor: AlwaysNotTaken,
    latches: PipelineLatches,
    structural_hazards: StructuralHazardPolicy,
    last_trace: PipelineTrace,
    last_result: ExecutionResult,
}

impl PipelineCore {
    #[must_use]
    pub fn new(reset_vector: u32) -> Self {
        Self {
            reset_vector,
            cycle: 0,
            state: HartState::new(reset_vector),
            predictor: AlwaysNotTaken,
            latches: PipelineLatches::default(),
            structural_hazards: StructuralHazardPolicy::default(),
            last_trace: PipelineTrace::default(),
            last_result: ExecutionResult::default(),
        }
    }

    #[must_use]
    pub fn latches(&self) -> &PipelineLatches {
        &self.latches
    }

    #[must_use]
    pub fn last_trace(&self) -> &PipelineTrace {
        &self.last_trace
    }

    fn update_latches(
        &mut self,
        raw: u32,
        decoded: rvsim_isa::DecodedInstruction,
        result: ExecutionResult,
    ) {
        self.latches.if_id = IfIdLatch {
            pc: Some(decoded.pc),
            raw: Some(raw),
        };
        self.latches.id_ex = IdExLatch {
            decoded: Some(decoded),
        };
        self.latches.ex_mem = ExMemLatch {
            destination: decoded.rd,
            alu_result: Some(self.state.registers.read(decoded.rd.unwrap_or(0))),
            memory_access: result.memory_access,
        };
        self.latches.mem_wb = MemWbLatch {
            destination: decoded.rd,
            value: decoded
                .rd
                .map(|register| self.state.registers.read(register)),
        };
    }
}

impl CpuModel for PipelineCore {
    fn hart_state(&self) -> &HartState {
        &self.state
    }

    fn hart_state_mut(&mut self) -> &mut HartState {
        &mut self.state
    }

    fn model_name(&self) -> &'static str {
        "pipeline"
    }
}

impl SimComponent for PipelineCore {
    fn reset(&mut self) {
        self.cycle = 0;
        self.state.reset(self.reset_vector);
        self.latches = PipelineLatches::default();
        self.last_trace = PipelineTrace::default();
        self.last_result = ExecutionResult::default();
    }
}

impl Processor for PipelineCore {
    type Error = CpuError;

    fn cycle(&self) -> u64 {
        self.cycle
    }

    fn step_cycle(&mut self, bus: &mut dyn Bus) -> Result<CpuCycle, Self::Error> {
        self.cycle += 1;

        if self.state.halted {
            self.last_trace = PipelineTrace {
                cycle: self.cycle,
                fetched_pc: None,
                retired_instructions: 0,
                note: "halted",
            };
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
                self.last_trace = PipelineTrace {
                    cycle: self.cycle,
                    fetched_pc: Some(pc),
                    retired_instructions: 0,
                    note: "instruction address misaligned",
                };
                return Ok(CpuCycle {
                    retired_instructions: 0,
                    stalled: true,
                });
            }
            Err(error) => return Err(error.into()),
        };

        let decoded = match decode_stage(raw, pc) {
            Ok(decoded) => decoded,
            Err(_error) => {
                self.last_result = apply_trap(
                    &mut self.state,
                    Trap::Exception(Exception::IllegalInstruction { instruction: raw }),
                    pc,
                );
                self.last_trace = PipelineTrace {
                    cycle: self.cycle,
                    fetched_pc: Some(pc),
                    retired_instructions: 0,
                    note: "illegal instruction",
                };
                return Ok(CpuCycle {
                    retired_instructions: 0,
                    stalled: true,
                });
            }
        };

        let fetch_output = fetch(bus, pc, pc.wrapping_add(4))?;
        let execution_metadata = classify(
            decoded,
            self.state.registers.read(decoded.rs1.unwrap_or_default()),
            self.state.registers.read(decoded.rs2.unwrap_or_default()),
        );

        let structural_stall = fetch_blocked_by_memory_access(
            self.structural_hazards.unified_memory,
            matches!(
                decoded.kind,
                InstructionKind::Load(_) | InstructionKind::Store(_)
            ),
        );

        let fallthrough = pc.wrapping_add(4);
        let target = execution_metadata.branch_target.unwrap_or(fallthrough);
        let prediction = self.predictor.predict(pc, fallthrough, target);

        self.last_result = execute_decoded(&mut self.state, bus, decoded)?;
        self.update_latches(raw, decoded, self.last_result);

        self.last_trace = PipelineTrace {
            cycle: self.cycle,
            fetched_pc: Some(fetch_output.pc),
            retired_instructions: self.last_result.retired,
            note: if structural_stall {
                "structural hazard"
            } else if prediction.taken {
                "predicted taken"
            } else {
                "retired"
            },
        };

        Ok(CpuCycle {
            retired_instructions: self.last_result.retired,
            stalled: structural_stall && self.last_result.retired == 0,
        })
    }
}
