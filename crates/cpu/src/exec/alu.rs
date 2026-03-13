use rvsim_isa::AluOp;

/// Execute one RV32I ALU operation.
#[must_use]
pub fn execute_alu(op: AluOp, lhs: u32, rhs: u32) -> u32 {
    match op {
        AluOp::Add => lhs.wrapping_add(rhs),
        AluOp::Sub => lhs.wrapping_sub(rhs),
        AluOp::And => lhs & rhs,
        AluOp::Or => lhs | rhs,
        AluOp::Xor => lhs ^ rhs,
        AluOp::Sll => lhs.wrapping_shl(rhs & 0x1f),
        AluOp::Slt => ((lhs as i32) < (rhs as i32)) as u32,
        AluOp::Sltu => (lhs < rhs) as u32,
        AluOp::Srl => lhs.wrapping_shr(rhs & 0x1f),
        AluOp::Sra => ((lhs as i32) >> (rhs & 0x1f)) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::execute_alu;
    use rvsim_isa::AluOp;

    #[test]
    fn arithmetic_shift_right_preserves_sign() {
        let value = execute_alu(AluOp::Sra, 0xffff_ff00, 4);
        assert_eq!(value, 0xffff_fff0);
    }
}
