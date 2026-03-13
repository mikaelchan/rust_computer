use rvsim_isa::BranchKind;

/// Return whether a conditional branch should be taken.
#[must_use]
pub fn branch_taken(kind: BranchKind, lhs: u32, rhs: u32) -> bool {
    match kind {
        BranchKind::Beq => lhs == rhs,
        BranchKind::Bne => lhs != rhs,
        BranchKind::Blt => (lhs as i32) < (rhs as i32),
        BranchKind::Bge => (lhs as i32) >= (rhs as i32),
        BranchKind::Bltu => lhs < rhs,
        BranchKind::Bgeu => lhs >= rhs,
    }
}

/// Compute the branch destination relative to the current program counter.
#[must_use]
pub fn branch_target(pc: u32, imm: i32) -> u32 {
    pc.wrapping_add_signed(imm)
}
