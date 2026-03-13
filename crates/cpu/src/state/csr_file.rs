use rvsim_isa::CsrAddress;

/// Machine-mode CSR values required by the first milestone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MachineCsrs {
    pub mstatus: u32,
    pub mtvec: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
}

/// Storage wrapper for CSR reads and writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CsrFile {
    machine: MachineCsrs,
}

impl CsrFile {
    #[must_use]
    pub fn read(&self, address: CsrAddress) -> u32 {
        match address {
            CsrAddress::Mstatus => self.machine.mstatus,
            CsrAddress::Mtvec => self.machine.mtvec,
            CsrAddress::Mepc => self.machine.mepc,
            CsrAddress::Mcause => self.machine.mcause,
            CsrAddress::Mtval => self.machine.mtval,
        }
    }

    pub fn write(&mut self, address: CsrAddress, value: u32) {
        match address {
            CsrAddress::Mstatus => self.machine.mstatus = value,
            CsrAddress::Mtvec => self.machine.mtvec = value,
            CsrAddress::Mepc => self.machine.mepc = value,
            CsrAddress::Mcause => self.machine.mcause = value,
            CsrAddress::Mtval => self.machine.mtval = value,
        }
    }

    #[must_use]
    pub fn machine(&self) -> &MachineCsrs {
        &self.machine
    }
}
