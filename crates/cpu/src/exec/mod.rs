//! Shared execution helpers for the first milestone cores.

pub mod alu;
pub mod branch;
pub mod csr;
pub mod load_store;

use rvsim_isa::{
    DecodedInstruction, Exception, LoadKind, StoreKind, SystemKind, Trap, opcode::InstructionKind,
};
use rvsim_system::{Bus, BusError};

use crate::{
    core::CpuError,
    state::{HartState, PrivilegeMode},
};

/// Minimal post-execution status shared across the core implementations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecutionResult {
    pub retired: u64,
    pub trap: Option<Trap>,
    pub memory_access: bool,
}

/// Execute one decoded instruction against the current hart state.
pub fn execute_decoded(
    state: &mut HartState,
    bus: &mut dyn Bus,
    decoded: DecodedInstruction,
) -> Result<ExecutionResult, CpuError> {
    let current_pc = decoded.pc;
    let next_pc = current_pc.wrapping_add(4);

    let rs1_value = decoded
        .rs1
        .map(|register| state.registers.read(register))
        .unwrap_or(0);
    let rs2_value = decoded
        .rs2
        .map(|register| state.registers.read(register))
        .unwrap_or(0);

    match decoded.kind {
        InstructionKind::Lui => {
            write_rd(state, decoded.rd, decoded.imm as u32);
            state.pc = next_pc;
        }
        InstructionKind::Auipc => {
            write_rd(
                state,
                decoded.rd,
                current_pc.wrapping_add_signed(decoded.imm),
            );
            state.pc = next_pc;
        }
        InstructionKind::Jal => {
            write_rd(state, decoded.rd, next_pc);
            state.pc = branch::branch_target(current_pc, decoded.imm);
        }
        InstructionKind::Jalr => {
            write_rd(state, decoded.rd, next_pc);
            state.pc = load_store::effective_address(rs1_value, decoded.imm) & !1;
        }
        InstructionKind::Branch(branch_kind) => {
            if branch::branch_taken(branch_kind, rs1_value, rs2_value) {
                state.pc = branch::branch_target(current_pc, decoded.imm);
            } else {
                state.pc = next_pc;
            }
        }
        InstructionKind::Load(load_kind) => {
            let address = u64::from(load_store::effective_address(rs1_value, decoded.imm));
            match load_value(bus, load_kind, address) {
                Ok(value) => {
                    write_rd(state, decoded.rd, value);
                    state.pc = next_pc;
                    return Ok(ExecutionResult {
                        retired: 1,
                        trap: None,
                        memory_access: true,
                    });
                }
                Err(error) => {
                    if matches!(error, BusError::Busy { .. }) {
                        return Ok(ExecutionResult {
                            retired: 0,
                            trap: None,
                            memory_access: true,
                        });
                    }
                    if let Some(trap) = map_memory_error(error.clone(), address as u32, true) {
                        let mut result = apply_trap(state, trap, current_pc);
                        result.memory_access = true;
                        return Ok(result);
                    }
                    return Err(CpuError::Bus(error));
                }
            }
        }
        InstructionKind::Store(store_kind) => {
            let address = u64::from(load_store::effective_address(rs1_value, decoded.imm));
            match store_value(bus, store_kind, address, rs2_value) {
                Ok(()) => {
                    state.pc = next_pc;
                    return Ok(ExecutionResult {
                        retired: 1,
                        trap: None,
                        memory_access: true,
                    });
                }
                Err(error) => {
                    if matches!(error, BusError::Busy { .. }) {
                        return Ok(ExecutionResult {
                            retired: 0,
                            trap: None,
                            memory_access: true,
                        });
                    }
                    if let Some(trap) = map_memory_error(error.clone(), address as u32, false) {
                        let mut result = apply_trap(state, trap, current_pc);
                        result.memory_access = true;
                        return Ok(result);
                    }
                    return Err(CpuError::Bus(error));
                }
            }
        }
        InstructionKind::OpImm(op) => {
            let rhs = if matches!(
                op,
                rvsim_isa::AluOp::Sll | rvsim_isa::AluOp::Srl | rvsim_isa::AluOp::Sra
            ) {
                decoded.raw.0 >> 20
            } else {
                decoded.imm as u32
            };
            let result = alu::execute_alu(op, rs1_value, rhs);
            write_rd(state, decoded.rd, result);
            state.pc = next_pc;
        }
        InstructionKind::Op(op) => {
            let result = alu::execute_alu(op, rs1_value, rs2_value);
            write_rd(state, decoded.rd, result);
            state.pc = next_pc;
        }
        InstructionKind::Csr(_op) => {
            let outcome = csr::execute(decoded, &state.csrs, rs1_value)
                .expect("csr instruction should provide an address");
            write_rd(state, decoded.rd, outcome.read_value);
            if let Some(write) = outcome.write {
                state.csrs.write(write.address, write.value);
            }
            state.pc = next_pc;
        }
        InstructionKind::System(SystemKind::Ecall) => {
            return Ok(apply_trap(
                state,
                Trap::Exception(Exception::EnvironmentCallFromMMode),
                current_pc,
            ));
        }
        InstructionKind::System(SystemKind::Ebreak) => {
            return Ok(apply_trap(
                state,
                Trap::Exception(Exception::Breakpoint),
                current_pc,
            ));
        }
        InstructionKind::System(SystemKind::Mret) => {
            return Ok(return_from_trap(state));
        }
    }

    Ok(ExecutionResult {
        retired: 1,
        trap: None,
        memory_access: false,
    })
}

/// Apply a trap to machine CSRs and redirect the hart to `mtvec`.
pub fn apply_trap(state: &mut HartState, trap: Trap, current_pc: u32) -> ExecutionResult {
    let trap_vector = state.csrs.enter_trap(trap, current_pc, state.privilege);
    state.privilege = PrivilegeMode::Machine;
    state.pc = trap_vector;

    ExecutionResult {
        retired: 0,
        trap: Some(trap),
        memory_access: false,
    }
}

/// Return from the current machine-mode trap context using `mepc`.
pub fn return_from_trap(state: &mut HartState) -> ExecutionResult {
    let (privilege, next_pc) = state.csrs.return_from_trap();
    state.privilege = privilege;
    state.pc = next_pc;

    ExecutionResult {
        retired: 1,
        trap: None,
        memory_access: false,
    }
}

fn write_rd(state: &mut HartState, rd: Option<u8>, value: u32) {
    if let Some(rd) = rd {
        state.registers.write(rd, value);
    }
}

fn load_value(bus: &mut dyn Bus, kind: LoadKind, address: u64) -> Result<u32, BusError> {
    match kind {
        LoadKind::Byte => Ok(load_store::sign_extend_byte(bus.load8(address)?)),
        LoadKind::Half => Ok(load_store::sign_extend_half(bus.load16(address)?)),
        LoadKind::Word => bus.load32(address),
        LoadKind::ByteUnsigned => Ok(bus.load8(address)? as u32),
        LoadKind::HalfUnsigned => Ok(bus.load16(address)? as u32),
    }
}

fn store_value(
    bus: &mut dyn Bus,
    kind: StoreKind,
    address: u64,
    value: u32,
) -> Result<(), BusError> {
    match kind {
        StoreKind::Byte => bus.store8(address, value as u8),
        StoreKind::Half => bus.store16(address, value as u16),
        StoreKind::Word => bus.store32(address, value),
    }
}

fn map_memory_error(error: BusError, address: u32, is_load: bool) -> Option<Trap> {
    match error {
        BusError::MisalignedAccess { .. } => Some(Trap::Exception(if is_load {
            Exception::LoadAddressMisaligned { addr: address }
        } else {
            Exception::StoreAddressMisaligned { addr: address }
        })),
        BusError::Busy { .. } => None,
        BusError::UnmappedAddress { .. }
        | BusError::ReadOnlyAddress { .. }
        | BusError::DeviceFault { .. } => None,
    }
}
