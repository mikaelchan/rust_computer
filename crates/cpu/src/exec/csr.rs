use rvsim_isa::{CsrAddress, CsrOp, DecodedInstruction, opcode::InstructionKind};

use crate::state::CsrFile;

/// A deferred CSR side effect that should be applied at commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsrWrite {
    pub address: CsrAddress,
    pub value: u32,
}

/// Read result plus optional write for one CSR instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsrOutcome {
    pub read_value: u32,
    pub write: Option<CsrWrite>,
}

/// Evaluate one CSR instruction using the committed CSR file.
#[must_use]
pub fn execute(decoded: DecodedInstruction, csrs: &CsrFile, rs1_value: u32) -> Option<CsrOutcome> {
    let InstructionKind::Csr(op) = decoded.kind else {
        return None;
    };

    let address = decoded.csr?;
    let current_value = csrs.read(address);
    let source = match op {
        CsrOp::ReadWriteImmediate | CsrOp::ReadSetImmediate | CsrOp::ReadClearImmediate => {
            decoded.rs1.unwrap_or_default() as u32
        }
        CsrOp::ReadWrite | CsrOp::ReadSet | CsrOp::ReadClear => rs1_value,
    };

    let write = write_value(op, address, current_value, source);
    Some(CsrOutcome {
        read_value: current_value,
        write,
    })
}

fn write_value(
    op: CsrOp,
    address: CsrAddress,
    current_value: u32,
    source: u32,
) -> Option<CsrWrite> {
    let value = match op {
        CsrOp::ReadWrite | CsrOp::ReadWriteImmediate => Some(source),
        CsrOp::ReadSet | CsrOp::ReadSetImmediate => {
            if source == 0 {
                None
            } else {
                Some(current_value | source)
            }
        }
        CsrOp::ReadClear | CsrOp::ReadClearImmediate => {
            if source == 0 {
                None
            } else {
                Some(current_value & !source)
            }
        }
    }?;

    Some(CsrWrite { address, value })
}

#[cfg(test)]
mod tests {
    use rvsim_isa::{CsrAddress, CsrOp, DecodedInstruction, InstructionKind};

    use super::execute;
    use crate::state::CsrFile;

    #[test]
    fn csr_set_with_zero_source_reads_without_writing() {
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Mstatus, 0x12);

        let decoded = DecodedInstruction::new(
            0,
            0,
            InstructionKind::Csr(CsrOp::ReadSet),
            Some(1),
            Some(0),
            None,
            0,
            Some(CsrAddress::Mstatus),
        );

        let outcome = execute(decoded, &csrs, 0).expect("csr outcome should exist");
        assert_eq!(outcome.read_value, 0x12);
        assert_eq!(outcome.write, None);
    }
}
