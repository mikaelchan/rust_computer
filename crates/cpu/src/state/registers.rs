/// Integer register file for RV32I.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterFile {
    regs: [u32; Self::NUM_REGISTERS],
}

impl RegisterFile {
    pub const NUM_REGISTERS: usize = 32;

    #[must_use]
    pub fn read(&self, index: u8) -> u32 {
        self.regs[index as usize]
    }

    pub fn write(&mut self, index: u8, value: u32) {
        if index != 0 {
            self.regs[index as usize] = value;
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[u32; Self::NUM_REGISTERS] {
        &self.regs
    }
}

impl Default for RegisterFile {
    fn default() -> Self {
        Self {
            regs: [0; Self::NUM_REGISTERS],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RegisterFile;

    #[test]
    fn x0_is_hardwired_to_zero() {
        let mut registers = RegisterFile::default();
        registers.write(0, 42);
        registers.write(1, 7);

        assert_eq!(registers.read(0), 0);
        assert_eq!(registers.read(1), 7);
    }
}
