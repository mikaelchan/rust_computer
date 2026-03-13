use rvsim_system::{Address, AddressRange, Addressable, BusError, InterruptLine, InterruptSet};

/// A minimal machine software interrupt source with a single `msip` register.
#[derive(Debug, Clone)]
pub struct MachineSoftwareInterrupt {
    range: AddressRange,
    msip: u32,
}

impl MachineSoftwareInterrupt {
    #[must_use]
    pub fn new(base: Address) -> Self {
        Self {
            range: AddressRange::new(base, 4),
            msip: 0,
        }
    }

    fn offset(&self, addr: Address) -> Result<Address, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }

        Ok(addr - self.range.start)
    }
}

impl Addressable for MachineSoftwareInterrupt {
    fn name(&self) -> &'static str {
        "machine-software-interrupt"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.msip = 0;
    }

    fn pending_interrupts(&self) -> InterruptSet {
        if (self.msip & 1) != 0 {
            InterruptSet::from(InterruptLine::MachineSoftware)
        } else {
            InterruptSet::empty()
        }
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let offset = self.offset(addr)?;
        Ok(self.msip.to_le_bytes()[offset as usize])
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        let offset = self.offset(addr)?;
        let mut bytes = self.msip.to_le_bytes();
        bytes[offset as usize] = value;
        self.msip = u32::from_le_bytes(bytes) & 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MachineSoftwareInterrupt;
    use rvsim_system::{Addressable, InterruptLine};

    const MSIP_BASE: u64 = 0x5000_0000;

    #[test]
    fn raises_machine_software_interrupt_when_msip_is_set() {
        let mut device = MachineSoftwareInterrupt::new(MSIP_BASE);

        write_u32(&mut device, MSIP_BASE, 1);

        assert_eq!(read_u32(&mut device, MSIP_BASE), 1);
        assert_eq!(
            device.pending_interrupt(),
            Some(InterruptLine::MachineSoftware)
        );
    }

    #[test]
    fn clears_machine_software_interrupt_when_msip_is_reset() {
        let mut device = MachineSoftwareInterrupt::new(MSIP_BASE);

        write_u32(&mut device, MSIP_BASE, 1);
        write_u32(&mut device, MSIP_BASE, 0);

        assert_eq!(read_u32(&mut device, MSIP_BASE), 0);
        assert_eq!(device.pending_interrupt(), None);
    }

    fn read_u32(device: &mut MachineSoftwareInterrupt, addr: u64) -> u32 {
        let mut bytes = [0; 4];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = device
                .load8(addr + offset as u64)
                .expect("msip register read should succeed");
        }
        u32::from_le_bytes(bytes)
    }

    fn write_u32(device: &mut MachineSoftwareInterrupt, addr: u64, value: u32) {
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            device
                .store8(addr + offset as u64, byte)
                .expect("msip register write should succeed");
        }
    }
}
