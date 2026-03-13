use rvsim_isa::{DecodeError, DecodedInstruction, decode};

/// Decode one fetched instruction word.
pub fn decode_stage(raw: u32, pc: u32) -> Result<DecodedInstruction, DecodeError> {
    decode(raw, pc)
}
