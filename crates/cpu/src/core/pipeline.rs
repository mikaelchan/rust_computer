use rvsim_isa::{Exception, Trap, opcode::InstructionKind};
use rvsim_system::{Bus, BusError, CpuCycle, Processor, SimComponent};

use crate::{
    core::{CpuError, CpuModel},
    exec::{ExecutionResult, apply_trap, return_from_trap},
    hazard::{
        control::detect_branch_flush,
        data::detect_raw_hazard,
        structural::{StructuralHazardPolicy, fetch_blocked_by_memory_access},
    },
    pipeline::{
        ex_stage::{self, ExecuteEvent},
        id_stage::decode_stage,
        if_stage::fetch,
        latches::{ExMemPayload, IdExPayload, MemWbPayload, PipelineLatches},
        mem_stage::{self, MemoryEvent},
        wb_stage,
    },
    predictor::{BimodalPredictor, BranchPredictor},
    state::HartState,
    trace::PipelineTrace,
};

/// A five-stage in-order pipeline with explicit latches and hazard handling.
#[derive(Debug, Clone)]
pub struct PipelineCore {
    reset_vector: u32,
    cycle: u64,
    state: HartState,
    predictor: BimodalPredictor,
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
            predictor: BimodalPredictor::default(),
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

    fn forwarded_operand(
        &self,
        source: Option<u8>,
        latched_value: u32,
        ex_mem_payload: Option<ExMemPayload>,
        mem_wb_payload: Option<MemWbPayload>,
    ) -> u32 {
        let Some(source) = source else {
            return latched_value;
        };

        if source == 0 {
            return 0;
        }

        if let Some(payload) = ex_mem_payload {
            if payload.decoded.rd == Some(source) && ex_stage::writes_back(payload.decoded) {
                if let Some(value) = payload.writeback_value {
                    return value;
                }
            }
        }

        if let Some(payload) = mem_wb_payload {
            if payload.decoded.rd == Some(source) && payload.writeback_value.is_some() {
                return payload.writeback_value.unwrap_or(latched_value);
            }
        }

        latched_value
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
                note: "halted",
                predicted_taken: false,
                fetch_stalled: true,
                ..PipelineTrace::default()
            };
            return Ok(CpuCycle {
                retired_instructions: 0,
                stalled: true,
            });
        }

        let current = self.latches;
        let writeback_pc = current.mem_wb.payload.map(|payload| payload.decoded.pc);
        let memory_pc = current.ex_mem.payload.map(|payload| payload.decoded.pc);
        let execute_pc = current.id_ex.payload.map(|payload| payload.decoded.pc);
        let decode_pc = current.if_id.payload.map(|payload| payload.pc);

        let mut next = PipelineLatches::default();
        let mut next_fetch_pc = self.state.pc;
        let mut retired_instructions = 0;
        let mut fetch_stalled = false;
        let mut decode_stalled = false;
        let mut flushed = false;
        let mut note = "progress";
        let mut fetched_pc = None;
        let mut predicted_taken = false;
        let mut trap_result = None;

        if let Some(payload) = current.mem_wb.payload {
            retired_instructions += wb_stage::write_back(&mut self.state, payload);
        }

        if let Some(payload) = current.ex_mem.payload {
            fetch_stalled = fetch_blocked_by_memory_access(
                self.structural_hazards.unified_memory,
                ex_stage::uses_memory(payload.decoded),
            );

            match mem_stage::access(bus, payload)? {
                MemoryEvent::Advance(payload) => {
                    next.mem_wb.payload = Some(payload);
                }
                MemoryEvent::Trap(trap) => {
                    trap_result = Some(apply_trap(&mut self.state, trap, payload.decoded.pc));
                    next_fetch_pc = self.state.pc;
                    flushed = true;
                    note = "memory trap";
                }
            }
        }

        if !flushed {
            if let Some(payload) = current.id_ex.payload {
                let rs1_value = self.forwarded_operand(
                    payload.decoded.rs1,
                    payload.rs1_value,
                    current.ex_mem.payload,
                    current.mem_wb.payload,
                );
                let rs2_value = self.forwarded_operand(
                    payload.decoded.rs2,
                    payload.rs2_value,
                    current.ex_mem.payload,
                    current.mem_wb.payload,
                );

                match ex_stage::execute(payload.decoded, rs1_value, rs2_value) {
                    ExecuteEvent::Advance(outcome) => {
                        if matches!(payload.decoded.kind, InstructionKind::Branch(_)) {
                            self.predictor.update(
                                payload.decoded.pc,
                                outcome.next_pc != payload.decoded.pc.wrapping_add(4),
                            );
                        }

                        let flush_status =
                            detect_branch_flush(payload.predicted_pc, outcome.next_pc);
                        if flush_status.flush_required {
                            flushed = true;
                            next_fetch_pc = outcome.next_pc;
                            note = "branch flush";
                        }

                        next.ex_mem.payload = Some(ExMemPayload {
                            decoded: payload.decoded,
                            writeback_value: outcome.writeback_value,
                            memory_address: outcome.memory_address,
                            store_value: outcome.store_value,
                            next_pc: outcome.next_pc,
                        });
                    }
                    ExecuteEvent::Trap(trap) => {
                        trap_result = Some(apply_trap(&mut self.state, trap, payload.decoded.pc));
                        next_fetch_pc = self.state.pc;
                        flushed = true;
                        note = "execute trap";
                    }
                    ExecuteEvent::ReturnFromTrap => {
                        trap_result = Some(return_from_trap(&mut self.state));
                        next_fetch_pc = self.state.pc;
                        flushed = true;
                        note = "mret";
                    }
                }
            }
        }

        if !flushed {
            if let Some(payload) = current.if_id.payload {
                match decode_stage(payload.raw, payload.pc) {
                    Ok(decoded) => {
                        let load_use_hazard = current
                            .id_ex
                            .payload
                            .filter(|producer| ex_stage::is_load(producer.decoded))
                            .map(|producer| {
                                detect_raw_hazard(producer.decoded.rd, decoded.rs1, decoded.rs2)
                            })
                            .unwrap_or_default();

                        if load_use_hazard.stall {
                            decode_stalled = true;
                            next.if_id.payload = Some(payload);
                            note = "load-use stall";
                        } else {
                            next.id_ex.payload = Some(IdExPayload {
                                decoded,
                                rs1_value: self
                                    .state
                                    .registers
                                    .read(decoded.rs1.unwrap_or_default()),
                                rs2_value: self
                                    .state
                                    .registers
                                    .read(decoded.rs2.unwrap_or_default()),
                                predicted_pc: payload.predicted_pc,
                                predicted_taken: payload.predicted_taken,
                            });
                        }
                    }
                    Err(_) => {
                        trap_result = Some(apply_trap(
                            &mut self.state,
                            Trap::Exception(Exception::IllegalInstruction {
                                instruction: payload.raw,
                            }),
                            payload.pc,
                        ));
                        next_fetch_pc = self.state.pc;
                        flushed = true;
                        note = "illegal instruction";
                    }
                }
            }
        }

        if flushed {
            next.if_id.payload = None;
            next.id_ex.payload = None;
        } else if !decode_stalled && !fetch_stalled {
            let fetch_pc = next_fetch_pc;
            match fetch(bus, fetch_pc, &self.predictor) {
                Ok(payload) => {
                    fetched_pc = Some(payload.pc);
                    predicted_taken = payload.predicted_taken;
                    next.if_id.payload = Some(payload);
                    next_fetch_pc = payload.predicted_pc;
                    if payload.predicted_taken && note == "progress" {
                        note = "predicted taken";
                    }
                }
                Err(BusError::MisalignedAccess { .. }) => {
                    trap_result = Some(apply_trap(
                        &mut self.state,
                        Trap::Exception(Exception::InstructionAddressMisaligned { addr: fetch_pc }),
                        fetch_pc,
                    ));
                    next_fetch_pc = self.state.pc;
                    note = "instruction address misaligned";
                }
                Err(error) => return Err(error.into()),
            }
        } else if fetch_stalled && note == "progress" {
            note = "structural hazard";
        }

        self.state.pc = next_fetch_pc;
        self.latches = next;
        let mut result = trap_result.unwrap_or(ExecutionResult {
            retired: retired_instructions,
            trap: None,
            memory_access: current
                .ex_mem
                .payload
                .map(|payload| ex_stage::uses_memory(payload.decoded))
                .unwrap_or(false),
        });
        result.retired = retired_instructions;
        self.last_result = result;
        self.last_trace = PipelineTrace {
            cycle: self.cycle,
            fetched_pc,
            decode_pc,
            execute_pc,
            memory_pc,
            writeback_pc,
            retired_instructions,
            predicted_taken,
            fetch_stalled,
            decode_stalled,
            flushed,
            note,
        };

        Ok(CpuCycle {
            retired_instructions,
            stalled: fetch_stalled || decode_stalled,
        })
    }
}

#[cfg(test)]
mod tests {
    use rvsim_system::{Bus, BusError, Processor};

    use super::PipelineCore;
    use crate::core::CpuModel;

    #[derive(Debug, Clone)]
    struct TestBus {
        bytes: Vec<u8>,
    }

    impl TestBus {
        fn new(size: usize) -> Self {
            Self {
                bytes: vec![0; size],
            }
        }

        fn load_program(&mut self, program: &[u32]) {
            for (index, word) in program.iter().copied().enumerate() {
                let offset = index * 4;
                self.bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
    }

    impl Bus for TestBus {
        fn load8(&mut self, addr: u64) -> Result<u8, BusError> {
            self.bytes
                .get(addr as usize)
                .copied()
                .ok_or(BusError::UnmappedAddress { addr })
        }

        fn store8(&mut self, addr: u64, value: u8) -> Result<(), BusError> {
            let slot = self
                .bytes
                .get_mut(addr as usize)
                .ok_or(BusError::UnmappedAddress { addr })?;
            *slot = value;
            Ok(())
        }
    }

    #[test]
    fn forwards_alu_results_without_stalling() {
        let mut bus = TestBus::new(64);
        bus.load_program(&[
            encode_addi(1, 0, 5),
            encode_addi(2, 1, 7),
            encode_add(3, 2, 1),
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
        }

        assert_eq!(core.hart_state().registers.read(1), 5);
        assert_eq!(core.hart_state().registers.read(2), 12);
        assert_eq!(core.hart_state().registers.read(3), 17);
    }

    #[test]
    fn inserts_load_use_stall_and_then_forwards_loaded_value() {
        let mut bus = TestBus::new(128);
        bus.load_program(&[
            encode_lui(1, 0),
            encode_addi(2, 0, 9),
            encode_sw(2, 1, 32),
            encode_lw(3, 1, 32),
            encode_add(4, 3, 3),
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        let mut observed_decode_stall = false;
        for _ in 0..12 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
            observed_decode_stall |= core.last_trace().decode_stalled;
        }

        assert!(observed_decode_stall);
        assert_eq!(core.hart_state().registers.read(3), 9);
        assert_eq!(core.hart_state().registers.read(4), 18);
    }

    #[test]
    fn flushes_wrong_path_after_taken_branch() {
        let mut bus = TestBus::new(128);
        bus.load_program(&[
            encode_addi(1, 0, 1),
            encode_beq(1, 1, 8),
            encode_addi(2, 0, 99),
            encode_addi(3, 0, 7),
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        let mut observed_flush = false;
        for _ in 0..12 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
            observed_flush |= core.last_trace().flushed;
        }

        assert!(observed_flush);
        assert_eq!(core.hart_state().registers.read(2), 0);
        assert_eq!(core.hart_state().registers.read(3), 7);
    }

    #[test]
    fn trains_bimodal_predictor_on_taken_branches() {
        let mut bus = TestBus::new(128);
        bus.load_program(&[
            encode_addi(1, 0, 3),
            encode_addi(1, 1, -1),
            encode_bne(1, 0, -4),
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        let mut observed_predicted_taken = false;
        for _ in 0..16 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
            observed_predicted_taken |= core.last_trace().predicted_taken;
        }

        assert!(observed_predicted_taken);
        assert_eq!(core.hart_state().registers.read(1), 0);
    }

    #[test]
    fn mret_restores_execution_from_mepc() {
        let mut bus = TestBus::new(128);
        bus.load_program(&[
            0x3020_0073,
            encode_addi(1, 0, 99),
            encode_addi(2, 0, 7),
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mepc, 8);

        for _ in 0..10 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
        }

        assert_eq!(core.hart_state().registers.read(1), 0);
        assert_eq!(core.hart_state().registers.read(2), 7);
    }

    fn encode_add(rd: u8, rs1: u8, rs2: u8) -> u32 {
        encode_r(0b0000000, rs2, rs1, 0b000, rd, 0b0110011)
    }

    fn encode_addi(rd: u8, rs1: u8, imm: i16) -> u32 {
        encode_i(imm, rs1, 0b000, rd, 0b0010011)
    }

    fn encode_lw(rd: u8, rs1: u8, imm: i16) -> u32 {
        encode_i(imm, rs1, 0b010, rd, 0b0000011)
    }

    fn encode_lui(rd: u8, upper_20: u32) -> u32 {
        (upper_20 << 12) | ((rd as u32) << 7) | 0b0110111
    }

    fn encode_sw(rs2: u8, rs1: u8, imm: i16) -> u32 {
        let imm = imm as u16 as u32;
        let imm_low = (imm & 0x1f) << 7;
        let imm_high = ((imm >> 5) & 0x7f) << 25;
        imm_high | ((rs2 as u32) << 20) | ((rs1 as u32) << 15) | (0b010 << 12) | imm_low | 0b0100011
    }

    fn encode_beq(rs1: u8, rs2: u8, imm: i16) -> u32 {
        encode_b(rs1, rs2, imm, 0b000)
    }

    fn encode_bne(rs1: u8, rs2: u8, imm: i16) -> u32 {
        encode_b(rs1, rs2, imm, 0b001)
    }

    fn encode_b(rs1: u8, rs2: u8, imm: i16, funct3: u32) -> u32 {
        let imm = imm as u16 as u32;
        let bit12 = ((imm >> 12) & 0x1) << 31;
        let bit11 = ((imm >> 11) & 0x1) << 7;
        let bits10_5 = ((imm >> 5) & 0x3f) << 25;
        let bits4_1 = ((imm >> 1) & 0x0f) << 8;
        bit12
            | bits10_5
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | bits4_1
            | bit11
            | 0b1100011
    }

    fn encode_jal(rd: u8, imm: i32) -> u32 {
        let imm = imm as u32;
        let bit20 = ((imm >> 20) & 0x1) << 31;
        let bits10_1 = ((imm >> 1) & 0x03ff) << 21;
        let bit11 = ((imm >> 11) & 0x1) << 20;
        let bits19_12 = ((imm >> 12) & 0xff) << 12;
        bit20 | bits19_12 | bit11 | bits10_1 | ((rd as u32) << 7) | 0b1101111
    }

    fn encode_i(imm: i16, rs1: u8, funct3: u32, rd: u8, opcode: u32) -> u32 {
        (((imm as u16 as u32) & 0x0fff) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | ((rd as u32) << 7)
            | opcode
    }

    fn encode_r(funct7: u32, rs2: u8, rs1: u8, funct3: u32, rd: u8, opcode: u32) -> u32 {
        (funct7 << 25)
            | ((rs2 as u32) << 20)
            | ((rs1 as u32) << 15)
            | (funct3 << 12)
            | ((rd as u32) << 7)
            | opcode
    }
}
