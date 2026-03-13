use crate::{pipeline::latches::MemWbPayload, state::HartState};

/// Write one instruction result back to architectural state.
#[must_use]
pub fn write_back(state: &mut HartState, payload: MemWbPayload) -> u64 {
    if let (Some(rd), Some(value)) = (payload.decoded.rd, payload.writeback_value) {
        state.registers.write(rd, value);
    }

    1
}
