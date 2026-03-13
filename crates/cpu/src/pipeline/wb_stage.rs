use crate::{
    exec::return_from_trap, pipeline::latches::MemWbPayload, state::HartState, trace::CommitEvent,
};
use rvsim_isa::SystemKind;

/// Write one instruction result back to architectural state.
#[must_use]
pub fn write_back(state: &mut HartState, payload: MemWbPayload) -> CommitEvent {
    let next_pc = if matches!(
        payload.decoded.kind,
        rvsim_isa::InstructionKind::System(SystemKind::Mret)
    ) {
        let _result = return_from_trap(state);
        state.pc
    } else {
        if let Some(write) = payload.csr_write {
            state.csrs.write(write.address, write.value);
        }

        if let (Some(rd), Some(value)) = (payload.decoded.rd, payload.writeback_value) {
            state.registers.write(rd, value);
        }

        state.pc = payload.next_pc;
        payload.next_pc
    };

    CommitEvent {
        pc: payload.decoded.pc,
        next_pc,
        kind: payload.decoded.kind,
        destination: payload.decoded.rd,
        value: payload.writeback_value,
        csr_write: payload.csr_write,
    }
}
