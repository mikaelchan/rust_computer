use rvsim_isa::{CsrAddress, DecodedInstruction, Exception, Trap, opcode::InstructionKind};
use rvsim_system::{Bus, CpuCycle, Processor, SimComponent};

use crate::{
    core::{CpuError, CpuModel},
    exec::{ExecutionResult, apply_trap},
    hazard::{
        control::detect_branch_flush,
        data::detect_raw_hazard,
        structural::{StructuralHazardPolicy, fetch_blocked_by_memory_access},
    },
    mmu::PageWalker,
    pipeline::{
        ex_stage::{self, ExecuteEvent},
        id_stage::decode_stage,
        if_stage::{self, FetchEvent},
        latches::{ExMemPayload, IdExPayload, MemWbPayload, PipelineLatches},
        mem_stage::{self, MemoryEvent},
        wb_stage,
    },
    predictor::{BimodalPredictor, BranchPredictor},
    state::HartState,
    trace::{CommitEvent, FlushReason, PipelineStats, PipelineTrace},
};

/// A five-stage in-order pipeline with explicit latches and hazard handling.
#[derive(Debug, Clone)]
pub struct PipelineCore {
    reset_vector: u32,
    cycle: u64,
    front_end_pc: u32,
    state: HartState,
    page_walker: PageWalker,
    translation_barrier: bool,
    predictor: BimodalPredictor,
    latches: PipelineLatches,
    structural_hazards: StructuralHazardPolicy,
    last_commit: Option<CommitEvent>,
    last_trace: PipelineTrace,
    last_result: ExecutionResult,
    stats: PipelineStats,
}

impl PipelineCore {
    #[must_use]
    pub fn new(reset_vector: u32) -> Self {
        Self {
            reset_vector,
            cycle: 0,
            front_end_pc: reset_vector,
            state: HartState::new(reset_vector),
            page_walker: PageWalker::default(),
            translation_barrier: false,
            predictor: BimodalPredictor::default(),
            latches: PipelineLatches::default(),
            structural_hazards: StructuralHazardPolicy::default(),
            last_commit: None,
            last_trace: PipelineTrace::default(),
            last_result: ExecutionResult::default(),
            stats: PipelineStats::default(),
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

    #[must_use]
    pub const fn front_end_pc(&self) -> u32 {
        self.front_end_pc
    }

    #[must_use]
    pub const fn last_commit(&self) -> Option<CommitEvent> {
        self.last_commit
    }

    #[must_use]
    pub const fn stats(&self) -> &PipelineStats {
        &self.stats
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
        self.front_end_pc = self.reset_vector;
        self.state.reset(self.reset_vector);
        self.page_walker.reset();
        self.translation_barrier = false;
        self.latches = PipelineLatches::default();
        self.last_commit = None;
        self.last_trace = PipelineTrace::default();
        self.last_result = ExecutionResult::default();
        self.stats = PipelineStats::default();
    }
}

impl Processor for PipelineCore {
    type Error = CpuError;

    fn cycle(&self) -> u64 {
        self.cycle
    }

    fn step_cycle(&mut self, bus: &mut dyn Bus) -> Result<CpuCycle, Self::Error> {
        self.cycle += 1;
        self.stats.cycles += 1;
        self.state.csrs.increment_cycle();
        self.state.csrs.sync_interrupts(bus.pending_interrupts());

        if self.state.halted {
            self.stats.fetch_stall_cycles += 1;
            self.last_commit = None;
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
        let mut next_front_end_pc = self.front_end_pc;
        let mut retired_instructions = 0;
        let mut fetch_stalled = false;
        let mut decode_stalled = false;
        let mut flushed = false;
        let mut note = "progress";
        let mut fetched_pc = None;
        let mut predicted_taken = false;
        let mut trap_result = None;
        let mut commit = None;
        let mut flush_reason = None;
        let mut memory_wait = false;
        let mut translation_barrier = self.translation_barrier;

        if let Some(payload) = current.mem_wb.payload {
            commit = Some(wb_stage::write_back(
                &mut self.state,
                &mut self.page_walker,
                payload,
            ));
            retired_instructions += 1;
            if is_translation_barrier_payload(payload.decoded, payload.csr_write) {
                translation_barrier = false;
            }
        }

        if !translation_barrier
            && !bus.is_busy()
            && let Some(interrupt) = self.state.csrs.pending_interrupt(self.state.privilege)
        {
            let current_pc = self.state.pc;
            trap_result = Some(apply_trap(
                &mut self.state,
                Trap::Interrupt(interrupt),
                current_pc,
            ));
            next_front_end_pc = self.state.pc;
            flushed = true;
            flush_reason = Some(FlushReason::Trap);
            note = "interrupt";
        }

        if !flushed {
            if let Some(payload) = current.ex_mem.payload {
                fetch_stalled = fetch_blocked_by_memory_access(
                    self.structural_hazards.unified_memory,
                    ex_stage::uses_memory(payload.decoded),
                );

                match mem_stage::access(
                    bus,
                    &mut self.page_walker,
                    &self.state.csrs,
                    self.state.privilege,
                    payload,
                )? {
                    MemoryEvent::Advance(payload) => {
                        next.mem_wb.payload = Some(payload);
                    }
                    MemoryEvent::Stall(payload) => {
                        memory_wait = true;
                        fetch_stalled = true;
                        decode_stalled = current.if_id.payload.is_some();
                        next.ex_mem.payload = Some(payload);
                        next.id_ex.payload = current.id_ex.payload;
                        next.if_id.payload = current.if_id.payload;
                        note = "memory wait";
                    }
                    MemoryEvent::Trap(trap) => {
                        trap_result = Some(apply_trap(&mut self.state, trap, payload.decoded.pc));
                        next_front_end_pc = self.state.pc;
                        flushed = true;
                        flush_reason = Some(FlushReason::Trap);
                        note = "memory trap";
                    }
                }
            }
        }

        if !flushed && !memory_wait && !translation_barrier {
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

                match ex_stage::execute(
                    payload.decoded,
                    rs1_value,
                    rs2_value,
                    self.state.privilege,
                    &self.state.csrs,
                ) {
                    ExecuteEvent::Advance(outcome) => {
                        if matches!(payload.decoded.kind, InstructionKind::Branch(_)) {
                            self.predictor.update(
                                payload.decoded.pc,
                                outcome.next_pc != payload.decoded.pc.wrapping_add(4),
                            );
                        }

                        let flush_status =
                            detect_branch_flush(payload.predicted_pc, outcome.next_pc);
                        let translation_serializing =
                            is_translation_barrier_payload(payload.decoded, outcome.csr_write);
                        if translation_serializing || flush_status.flush_required {
                            flushed = true;
                            next_front_end_pc = outcome.next_pc;
                            if translation_serializing {
                                translation_barrier = true;
                                flush_reason = Some(FlushReason::TranslationBarrier);
                                note = "translation barrier";
                            } else {
                                flush_reason = Some(
                                    if matches!(
                                        payload.decoded.kind,
                                        InstructionKind::System(
                                            rvsim_isa::SystemKind::Mret
                                                | rvsim_isa::SystemKind::Sret
                                        )
                                    ) {
                                        FlushReason::ReturnFromTrap
                                    } else {
                                        FlushReason::BranchRedirect
                                    },
                                );
                                note = if matches!(
                                    payload.decoded.kind,
                                    InstructionKind::System(
                                        rvsim_isa::SystemKind::Mret | rvsim_isa::SystemKind::Sret
                                    )
                                ) {
                                    "xret"
                                } else {
                                    "branch flush"
                                };
                            }
                        }

                        next.ex_mem.payload = Some(ExMemPayload {
                            decoded: payload.decoded,
                            writeback_value: outcome.writeback_value,
                            csr_write: outcome.csr_write,
                            translation_fence: outcome.translation_fence,
                            memory_address: outcome.memory_address,
                            store_value: outcome.store_value,
                            next_pc: outcome.next_pc,
                        });
                    }
                    ExecuteEvent::Trap(trap) => {
                        trap_result = Some(apply_trap(&mut self.state, trap, payload.decoded.pc));
                        next_front_end_pc = self.state.pc;
                        flushed = true;
                        flush_reason = Some(FlushReason::Trap);
                        note = "execute trap";
                    }
                }
            }
        }

        if !flushed && !memory_wait && !translation_barrier {
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
                        let csr_hazard = matches!(decoded.kind, InstructionKind::Csr(_))
                            && (current
                                .id_ex
                                .payload
                                .map(|payload| {
                                    matches!(payload.decoded.kind, InstructionKind::Csr(_))
                                })
                                .unwrap_or(false)
                                || current
                                    .ex_mem
                                    .payload
                                    .map(|payload| {
                                        matches!(payload.decoded.kind, InstructionKind::Csr(_))
                                    })
                                    .unwrap_or(false)
                                || current
                                    .mem_wb
                                    .payload
                                    .map(|payload| {
                                        matches!(payload.decoded.kind, InstructionKind::Csr(_))
                                    })
                                    .unwrap_or(false));

                        if load_use_hazard.stall || csr_hazard {
                            decode_stalled = true;
                            next.if_id.payload = Some(payload);
                            note = if csr_hazard {
                                "csr stall"
                            } else {
                                "load-use stall"
                            };
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
                        next_front_end_pc = self.state.pc;
                        flushed = true;
                        flush_reason = Some(FlushReason::Trap);
                        note = "illegal instruction";
                    }
                }
            }
        } else if !flushed && !memory_wait && translation_barrier {
            fetch_stalled = true;
            decode_stalled = current.if_id.payload.is_some();
            next.if_id.payload = current.if_id.payload;
            next.id_ex.payload = current.id_ex.payload;
            note = "translation barrier";
        }

        if flushed {
            next.if_id.payload = None;
            next.id_ex.payload = None;
        } else if !memory_wait && !decode_stalled && !fetch_stalled && !translation_barrier {
            let fetch_pc = next_front_end_pc;
            match if_stage::fetch(
                bus,
                &mut self.page_walker,
                &self.state.csrs,
                self.state.privilege,
                fetch_pc,
                &self.predictor,
            )? {
                FetchEvent::Advance(payload) => {
                    fetched_pc = Some(payload.pc);
                    predicted_taken = payload.predicted_taken;
                    next.if_id.payload = Some(payload);
                    next_front_end_pc = payload.predicted_pc;
                    if payload.predicted_taken && note == "progress" {
                        note = "predicted taken";
                    }
                }
                FetchEvent::Stall => {
                    fetch_stalled = true;
                    if note == "progress" {
                        note = "fetch wait";
                    }
                }
                FetchEvent::Trap(trap) => {
                    trap_result = Some(apply_trap(&mut self.state, trap, fetch_pc));
                    next_front_end_pc = self.state.pc;
                    flushed = true;
                    flush_reason = Some(FlushReason::Trap);
                    note = "fetch trap";
                }
            }
        } else if fetch_stalled && note == "progress" {
            note = "structural hazard";
        }

        let result = ExecutionResult {
            retired: retired_instructions + trap_result.map(|value| value.retired).unwrap_or(0),
            trap: trap_result.and_then(|value| value.trap),
            memory_access: current
                .ex_mem
                .payload
                .map(|payload| ex_stage::uses_memory(payload.decoded))
                .unwrap_or(false)
                || trap_result
                    .map(|value| value.memory_access)
                    .unwrap_or(false),
        };

        self.stats.retired_instructions += result.retired;
        self.stats.fetch_stall_cycles += u64::from(fetch_stalled);
        self.stats.decode_stall_cycles += u64::from(decode_stalled);
        self.stats.flush_cycles += u64::from(flushed);
        self.stats.predicted_taken_fetches += u64::from(predicted_taken);

        if let Some(reason) = flush_reason {
            match reason {
                FlushReason::BranchRedirect => self.stats.branch_flushes += 1,
                FlushReason::Trap => self.stats.trap_flushes += 1,
                FlushReason::ReturnFromTrap => self.stats.return_flushes += 1,
                FlushReason::TranslationBarrier => {
                    self.stats.translation_barrier_flushes += 1;
                }
            }
        }

        if result.trap.is_some() {
            self.stats.trap_count += 1;
        }

        self.front_end_pc = next_front_end_pc;
        self.translation_barrier = translation_barrier;
        self.latches = next;
        self.last_commit = commit;
        self.last_result = result;
        self.last_trace = PipelineTrace {
            cycle: self.cycle,
            fetched_pc,
            decode_pc,
            execute_pc,
            memory_pc,
            writeback_pc,
            commit,
            trap: self.last_result.trap,
            flush_reason,
            retired_instructions,
            predicted_taken,
            fetch_stalled,
            decode_stalled,
            flushed,
            note,
        };

        Ok(CpuCycle {
            retired_instructions: self.last_result.retired,
            stalled: fetch_stalled || decode_stalled,
        })
    }
}

fn is_translation_barrier_payload(
    decoded: DecodedInstruction,
    csr_write: Option<crate::exec::csr::CsrWrite>,
) -> bool {
    matches!(
        decoded.kind,
        InstructionKind::System(rvsim_isa::SystemKind::SfenceVma)
    ) || matches!(csr_write, Some(write) if write.address == CsrAddress::Satp)
}

#[cfg(test)]
mod tests {
    use rvsim_devices::{
        InterruptController, LatencyAdapter, MachineSoftwareInterrupt, MachineTimer, Ram, Rom,
        SupervisorSoftwareInterrupt,
    };
    use rvsim_system::{
        AddressRange, Bus, BusError, CacheConfig, Machine, MemoryMap, Processor, SplitL1Cache,
        StoreAllocationPolicy, WritePolicy,
    };

    use super::PipelineCore;
    use crate::{FlushReason, core::CpuModel};

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
            self.store_words(0, program);
        }

        fn store_words(&mut self, base: u32, words: &[u32]) {
            for (index, word) in words.iter().copied().enumerate() {
                let offset = base as usize + index * 4;
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
    fn keeps_front_end_pc_ahead_of_architectural_pc_until_writeback() {
        let mut bus = TestBus::new(64);
        bus.load_program(&[encode_addi(1, 0, 5), encode_addi(2, 0, 6), encode_jal(0, 0)]);

        let mut core = PipelineCore::new(0);

        core.step_cycle(&mut bus)
            .expect("first pipeline cycle should work");
        assert_eq!(core.hart_state().pc, 0);
        assert_eq!(core.front_end_pc(), 4);

        for _ in 0..4 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
        }

        assert_eq!(core.hart_state().pc, 4);
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

    #[test]
    fn fetches_instructions_through_sv32_translation() {
        let mut bus = TestBus::new(0x10_000);
        bus.store_words(0x0000, &[encode_addi(1, 0, 5), encode_jal(0, 0)]);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("sv32 instruction fetch should execute");
        }

        assert_eq!(core.hart_state().registers.read(1), 5);
        assert_eq!(core.hart_state().pc, 0x4004);
    }

    #[test]
    fn executes_load_store_through_sv32_translation() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..14 {
            core.step_cycle(&mut bus)
                .expect("sv32 load/store flow should execute");
        }

        assert_eq!(core.hart_state().registers.read(3), 9);
        assert_eq!(bus.read_word(0x1000), 9);
    }

    #[test]
    fn hardware_manages_accessed_and_dirty_bits_for_data_pages() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..14 {
            core.step_cycle(&mut bus)
                .expect("sv32 load/store flow should update ad bits");
        }

        assert_eq!(core.hart_state().registers.read(3), 9);
        assert_eq!(bus.read_word(0x1000), 9);
        assert_eq!(bus.read_word(0x3020) & (PTE_A | PTE_D), PTE_A | PTE_D);
    }

    #[test]
    fn supervisor_sum_allows_loading_user_pages() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sstatus, MSTATUS_SUM);

        for _ in 0..10 {
            core.step_cycle(&mut bus)
                .expect("sum-enabled supervisor load should execute");
        }

        assert_eq!(core.hart_state().registers.read(2), 9);
    }

    #[test]
    fn supervisor_mxr_allows_loading_execute_only_pages() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sstatus, MSTATUS_MXR);

        for _ in 0..10 {
            core.step_cycle(&mut bus)
                .expect("mxr-enabled supervisor load should execute");
        }

        assert_eq!(core.hart_state().registers.read(2), 9);
    }

    #[test]
    fn machine_mprv_load_uses_supervisor_translation() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x0000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Machine;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut().csrs.write(
            rvsim_isa::CsrAddress::Mstatus,
            MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT),
        );

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("mprv-enabled machine load should execute");
        }

        assert_eq!(core.hart_state().registers.read(2), 9);
    }

    #[test]
    fn executes_fetch_and_data_access_through_sv32_superpage() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x400000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..14 {
            core.step_cycle(&mut bus)
                .expect("sv32 superpage fetch/data flow should execute");
        }

        assert_eq!(core.hart_state().registers.read(3), 9);
        assert_eq!(bus.read_word(0x8000), 9);
        assert_eq!(core.hart_state().pc, 0x400010);
    }

    #[test]
    fn satp_write_switches_address_space() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..16 {
            core.step_cycle(&mut bus)
                .expect("satp switch should execute through pipeline");
        }

        assert_eq!(core.hart_state().registers.read(2), 5);
        assert_eq!(core.hart_state().registers.read(4), 9);
        assert_eq!(core.stats().translation_barrier_flushes, 1);
    }

    #[test]
    fn satp_write_preserves_tlb_namespace_until_sfence_vma() {
        let satp_asid_1 = sv32_satp_with_asid(0x2000, 1);
        let satp_asid_2 = sv32_satp_with_asid(0x5000, 2);

        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, satp_asid_1);

        for _ in 0..12 {
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

        for _ in 0..32 {
            core.step_cycle(&mut bus)
                .expect("satp namespace preservation flow should execute");
            if core.hart_state().registers.read(7) == 9 {
                break;
            }
        }

        assert_eq!(core.hart_state().registers.read(4), 7);
        assert_eq!(core.hart_state().registers.read(6), 5);
        assert_eq!(core.hart_state().registers.read(7), 9);
        assert_eq!(core.stats().translation_barrier_flushes, 3);
    }

    #[test]
    fn asid_specific_sfence_vma_preserves_global_mapping() {
        let satp_asid_1 = sv32_satp_with_asid(0x2000, 1);
        let satp_asid_2 = sv32_satp_with_asid(0x5000, 2);

        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, satp_asid_1);

        for _ in 0..12 {
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

        for _ in 0..24 {
            core.step_cycle(&mut bus)
                .expect("global mapping flow should execute");
            if core.hart_state().registers.read(6) == 9 {
                break;
            }
        }

        assert_eq!(core.hart_state().registers.read(5), 5);
        assert_eq!(core.hart_state().registers.read(6), 9);
        assert_eq!(core.stats().translation_barrier_flushes, 3);
    }

    #[test]
    fn sfence_vma_flushes_stale_translation() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x3000 >> 12));

        for _ in 0..8 {
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

        for _ in 0..12 {
            core.step_cycle(&mut bus)
                .expect("sfence.vma flow should observe remapped page");
        }

        assert_eq!(core.hart_state().registers.read(3), 9);
        assert_eq!(core.stats().translation_barrier_flushes, 1);
    }

    #[test]
    fn sfence_vma_can_flush_one_virtual_address_only() {
        let mut bus = TestBus::new(0x10_000);
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

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));

        for _ in 0..10 {
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

        for _ in 0..12 {
            core.step_cycle(&mut bus)
                .expect("selective sfence.vma should execute");
        }

        assert_eq!(core.hart_state().registers.read(5), 11);
        assert_eq!(core.hart_state().registers.read(6), 7);
        assert_eq!(core.stats().translation_barrier_flushes, 1);
    }

    #[test]
    fn traps_on_instruction_page_fault_during_sv32_fetch() {
        let mut bus = TestBus::new(0x10_000);
        bus.store_words(0x0080, &[encode_addi(10, 0, 1), encode_jal(0, 0)]);

        let mut core = PipelineCore::new(0x4000);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Satp, SATP_MODE_SV32 | (0x2000 >> 12));
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x80);

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("instruction page fault should trap through pipeline");
        }

        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::Machine
        );
        assert_eq!(core.hart_state().registers.read(10), 1);
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
        assert_eq!(core.stats().trap_count, 1);
    }

    #[test]
    fn sret_restores_execution_from_sepc() {
        let mut bus = TestBus::new(128);
        bus.load_program(&[
            encode_sret(),
            encode_addi(1, 0, 99),
            encode_addi(2, 0, 7),
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sepc, 8);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Sstatus, 1 << 5);

        for _ in 0..10 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
        }

        assert_eq!(core.hart_state().registers.read(1), 0);
        assert_eq!(core.hart_state().registers.read(2), 7);
        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::User
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Sstatus) & (1 << 1),
            1 << 1
        );
        assert_eq!(core.stats().return_flushes, 1);
    }

    #[test]
    fn delegates_user_ecall_to_supervisor_handler_and_returns_with_sret() {
        let mut bus = TestBus::new(128);
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

        let mut core = PipelineCore::new(0);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::User;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Medeleg, 1 << 8);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Stvec, 0x20);

        for _ in 0..16 {
            core.step_cycle(&mut bus)
                .expect("pipeline delegated supervisor trap flow should execute");
        }

        assert_eq!(core.hart_state().registers.read(1), 1);
        assert_eq!(core.hart_state().registers.read(2), 7);
        assert_eq!(
            core.hart_state().privilege,
            crate::state::PrivilegeMode::User
        );
        assert_eq!(
            core.hart_state().csrs.read(rvsim_isa::CsrAddress::Scause),
            8
        );
        assert_eq!(core.hart_state().csrs.read(rvsim_isa::CsrAddress::Sepc), 4);
        assert_eq!(core.stats().trap_count, 1);
        assert_eq!(core.stats().trap_flushes, 1);
        assert_eq!(core.stats().return_flushes, 1);
    }

    #[test]
    fn user_mode_machine_csr_access_traps_as_illegal_instruction() {
        let instruction = encode_csrrwi(1, rvsim_isa::CsrAddress::Mstatus as u16, 1);
        let mut bus = TestBus::new(64);
        bus.load_program(&[
            instruction,
            encode_jal(0, 0),
            0,
            0,
            0,
            0,
            0,
            0,
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::User;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("illegal csr access should trap through pipeline");
        }

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
        assert_eq!(core.stats().trap_count, 1);
    }

    #[test]
    fn supervisor_satp_access_traps_when_tvm_is_set() {
        let instruction = encode_csrrw(1, rvsim_isa::CsrAddress::Satp as u16, 0);
        let mut bus = TestBus::new(0x10_000);
        bus.load_program(&[
            instruction,
            encode_jal(0, 0),
            0,
            0,
            0,
            0,
            0,
            0,
            encode_jal(0, 0),
        ]);
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x0000,
            PTE_R | PTE_X | PTE_A,
        );

        let mut core = PipelineCore::new(0x4000);
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

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("tvm should trap supervisor satp access through pipeline");
        }

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
        assert_eq!(core.stats().trap_count, 1);
        assert_eq!(core.stats().translation_barrier_flushes, 0);
    }

    #[test]
    fn supervisor_sfence_vma_traps_when_tvm_is_set() {
        let instruction = encode_sfence_vma(0, 0);
        let mut bus = TestBus::new(64);
        bus.load_program(&[
            instruction,
            encode_jal(0, 0),
            0,
            0,
            0,
            0,
            0,
            0,
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        core.hart_state_mut().privilege = crate::state::PrivilegeMode::Supervisor;
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 0x20);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mstatus, MSTATUS_TVM);

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("tvm should trap supervisor sfence.vma through pipeline");
        }

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
        assert_eq!(core.stats().trap_count, 1);
        assert_eq!(core.stats().translation_barrier_flushes, 0);
    }

    #[test]
    fn supervisor_sret_traps_when_tsr_is_set() {
        let instruction = encode_sret();
        let mut bus = TestBus::new(64);
        bus.load_program(&[
            instruction,
            encode_jal(0, 0),
            0,
            0,
            0,
            0,
            0,
            0,
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
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

        for _ in 0..8 {
            core.step_cycle(&mut bus)
                .expect("tsr should trap supervisor sret through pipeline");
        }

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
        assert_eq!(core.stats().trap_count, 1);
        assert_eq!(core.stats().return_flushes, 0);
    }

    #[test]
    fn serializes_back_to_back_csr_instructions() {
        let mut bus = TestBus::new(128);
        bus.load_program(&[
            encode_csrrwi(0, rvsim_isa::CsrAddress::Mtvec as u16, 4),
            encode_csrrs(1, rvsim_isa::CsrAddress::Mtvec as u16, 0),
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        let mut observed_csr_stall = false;
        for _ in 0..10 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
            observed_csr_stall |= core.last_trace().note == "csr stall";
        }

        assert!(observed_csr_stall);
        assert_eq!(core.hart_state().registers.read(1), 4);
        assert_eq!(core.hart_state().csrs.read(rvsim_isa::CsrAddress::Mtvec), 4);
    }

    #[test]
    fn commit_view_reports_csr_writeback() {
        let mut bus = TestBus::new(128);
        bus.load_program(&[
            encode_csrrwi(1, rvsim_isa::CsrAddress::Mstatus as u16, 3),
            encode_jal(0, 0),
        ]);

        let mut core = PipelineCore::new(0);
        let mut csr_commit = None;
        for _ in 0..6 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
            if let Some(commit) = core
                .last_commit()
                .filter(|commit| commit.csr_write.is_some())
            {
                csr_commit = Some(commit);
            }
        }

        let commit = csr_commit.expect("csr instruction should commit");
        assert_eq!(commit.destination, Some(1));
        assert_eq!(commit.value, Some(0));
        let csr_write = commit.csr_write.expect("csr write should be reported");
        assert_eq!(csr_write.address, rvsim_isa::CsrAddress::Mstatus);
        assert_eq!(csr_write.value, 3);
    }

    #[test]
    fn exposes_last_commit_and_cumulative_stats() {
        let mut bus = TestBus::new(64);
        bus.load_program(&[encode_addi(1, 0, 5), encode_jal(0, 0)]);

        let mut core = PipelineCore::new(0);
        for _ in 0..5 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
        }

        let commit = core.last_commit().expect("one instruction should commit");
        assert_eq!(commit.pc, 0);
        assert_eq!(commit.next_pc, 4);
        assert_eq!(commit.destination, Some(1));
        assert_eq!(commit.value, Some(5));
        assert_eq!(core.stats().cycles, 5);
        assert_eq!(core.stats().retired_instructions, 1);
    }

    #[test]
    fn tracks_trap_flushes_in_trace_and_stats() {
        let mut bus = TestBus::new(64);
        bus.load_program(&[0xffff_ffff, encode_jal(0, 0)]);

        let mut core = PipelineCore::new(0);
        core.hart_state_mut()
            .csrs
            .write(rvsim_isa::CsrAddress::Mtvec, 4);
        let mut observed_trap_flush = false;
        let mut observed_trap_trace = false;
        for _ in 0..4 {
            core.step_cycle(&mut bus)
                .expect("pipeline cycle should work");
            observed_trap_flush |= core.last_trace().flush_reason == Some(FlushReason::Trap);
            observed_trap_trace |= core.last_trace().trap.is_some();
        }

        assert!(observed_trap_flush);
        assert_eq!(core.stats().trap_count, 1);
        assert_eq!(core.stats().trap_flushes, 1);
        assert!(observed_trap_trace);
    }

    #[test]
    fn takes_precise_machine_timer_interrupt_after_older_commit() {
        const TIMER_BASE: u64 = 0x3000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(Rom::from_words(
                0,
                &[
                    encode_addi(1, 0, 5),
                    encode_addi(2, 0, 9),
                    encode_jal(0, 0),
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

        let mut machine = Machine::new(PipelineCore::new(0), memory);
        machine
            .bus_mut()
            .store32(TIMER_BASE + 8, 5)
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

        for _ in 0..12 {
            machine
                .step_cycle()
                .expect("pipeline cycle should work through interrupt");
        }

        assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
        assert_eq!(machine.cpu().hart_state().registers.read(2), 0);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Mepc),
            4
        );
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 7
        );
        assert_eq!(machine.cpu().stats().trap_count, 1);
        assert_eq!(machine.cpu().stats().trap_flushes, 1);
    }

    #[test]
    fn takes_machine_external_interrupt_from_controller_device() {
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
                    encode_addi(10, 0, 2),
                    encode_jal(0, 0),
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(InterruptController::new(CONTROLLER_BASE))
            .expect("controller should map");

        let mut machine = Machine::new(PipelineCore::new(0), memory);
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

        for _ in 0..5 {
            machine
                .step_cycle()
                .expect("pipeline warmup cycle should work");
        }

        machine
            .bus_mut()
            .store32(CONTROLLER_BASE + 4, 1)
            .expect("enable register should write");
        machine
            .bus_mut()
            .store32(CONTROLLER_BASE + 8, 1)
            .expect("set-pending register should write");

        for _ in 0..8 {
            machine
                .step_cycle()
                .expect("pipeline external interrupt cycle should work");
        }

        assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 2);
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 11
        );
        assert_eq!(machine.cpu().stats().trap_count, 1);
        assert_eq!(machine.cpu().stats().trap_flushes, 1);
    }

    #[test]
    fn takes_machine_software_interrupt_from_msip_device() {
        const MSIP_BASE: u64 = 0x5000_0000;

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
                    encode_addi(10, 0, 3),
                    encode_jal(0, 0),
                ],
            ))
            .expect("rom should map");
        memory
            .map_device(MachineSoftwareInterrupt::new(MSIP_BASE))
            .expect("msip device should map");

        let mut machine = Machine::new(PipelineCore::new(0), memory);
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

        for _ in 0..5 {
            machine
                .step_cycle()
                .expect("pipeline warmup cycle should work");
        }

        machine
            .bus_mut()
            .store32(MSIP_BASE, 1)
            .expect("msip register should write");

        for _ in 0..8 {
            machine
                .step_cycle()
                .expect("pipeline software interrupt cycle should work");
        }

        assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
        assert_eq!(machine.cpu().hart_state().registers.read(10), 3);
        assert_eq!(
            machine
                .cpu()
                .hart_state()
                .csrs
                .read(rvsim_isa::CsrAddress::Mcause),
            (1_u32 << 31) | 3
        );
        assert_eq!(machine.cpu().stats().trap_count, 1);
        assert_eq!(machine.cpu().stats().trap_flushes, 1);
    }

    #[test]
    fn takes_delegated_supervisor_software_interrupt_from_ssip_device() {
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

        let mut machine = Machine::new(PipelineCore::new(0), memory);
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

        for _ in 0..14 {
            machine
                .step_cycle()
                .expect("pipeline supervisor interrupt cycle should work");
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
        assert_eq!(machine.cpu().stats().trap_count, 1);
        assert_eq!(machine.cpu().stats().trap_flushes, 1);
        assert_eq!(machine.cpu().stats().return_flushes, 1);
    }

    #[test]
    fn stalls_memory_stage_until_ram_latency_completes() {
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

        let mut machine = Machine::new(PipelineCore::new(0), memory);
        let mut observed_memory_wait = false;
        for _ in 0..18 {
            machine
                .step_cycle()
                .expect("pipeline cycle should work through ram latency");
            observed_memory_wait |= machine.cpu().last_trace().note == "memory wait";
        }

        assert!(observed_memory_wait);
        assert_eq!(machine.cpu().hart_state().registers.read(3), 9);
    }

    #[test]
    fn split_l1_cache_feeds_pipeline_front_end_and_data_path_separately() {
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

        let mut machine = Machine::new(PipelineCore::new(0), cache);
        for _ in 0..12 {
            machine
                .step_cycle()
                .expect("pipeline cycle should work through split cache");
        }

        assert_eq!(machine.cpu().hart_state().registers.read(3), 9);
        assert_eq!(machine.cpu().hart_state().registers.read(4), 9);

        let stats = machine.bus().stats();
        assert!(stats.instruction.refills >= 2);
        assert!(stats.instruction.read_hits >= 4);
        assert_eq!(stats.data.refills, 1);
        assert!(stats.data.read_hits >= 1);
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

    fn encode_ecall() -> u32 {
        0x0000_0073
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

    fn encode_csrrs(rd: u8, csr: u16, rs1: u8) -> u32 {
        ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b010 << 12) | ((rd as u32) << 7) | 0b1110011
    }

    fn encode_csrrw(rd: u8, csr: u16, rs1: u8) -> u32 {
        ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b001 << 12) | ((rd as u32) << 7) | 0b1110011
    }

    fn encode_csrrwi(rd: u8, csr: u16, zimm: u8) -> u32 {
        ((csr as u32) << 20)
            | ((zimm as u32) << 15)
            | (0b101 << 12)
            | ((rd as u32) << 7)
            | 0b1110011
    }

    fn encode_sret() -> u32 {
        0x1020_0073
    }

    fn encode_sfence_vma(rs1: u8, rs2: u8) -> u32 {
        0x1200_0073 | ((rs1 as u32) << 15) | ((rs2 as u32) << 20)
    }

    fn install_sv32_mapping(
        bus: &mut TestBus,
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
        bus: &mut TestBus,
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
