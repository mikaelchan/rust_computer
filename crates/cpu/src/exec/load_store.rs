/// Compute an effective address from a base register and signed offset.
#[must_use]
pub fn effective_address(base: u32, offset: i32) -> u32 {
    base.wrapping_add_signed(offset)
}

/// Sign-extend an 8-bit load into a 32-bit register result.
#[must_use]
pub fn sign_extend_byte(value: u8) -> u32 {
    (value as i8 as i32) as u32
}

/// Sign-extend a 16-bit load into a 32-bit register result.
#[must_use]
pub fn sign_extend_half(value: u16) -> u32 {
    (value as i16 as i32) as u32
}
