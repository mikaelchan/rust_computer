use rvsim_system::{Address, AddressRange, Addressable, BusError, InterruptLine, InterruptSet};

/// A minimal supervisor software interrupt source with a single `ssip` register.
#[derive(Debug, Clone)]
pub struct SupervisorSoftwareInterrupt {
    range: AddressRange,
    ssip: u32,
}

impl SupervisorSoftwareInterrupt {
    #[must_use]
    pub fn new(base: Address) -> Self {
        Self {
            range: AddressRange::new(base, 4),
            ssip: 0,
        }
    }

    fn offset(&self, addr: Address) -> Result<Address, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }

        Ok(addr - self.range.start)
    }
}

impl Addressable for SupervisorSoftwareInterrupt {
    fn name(&self) -> &'static str {
        "supervisor-software-interrupt"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.ssip = 0;
    }

    fn pending_interrupts(&self) -> InterruptSet {
        if (self.ssip & 1) != 0 {
            InterruptSet::from(InterruptLine::SupervisorSoftware)
        } else {
            InterruptSet::empty()
        }
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let offset = self.offset(addr)?;
        Ok(self.ssip.to_le_bytes()[offset as usize])
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        let offset = self.offset(addr)?;
        let mut bytes = self.ssip.to_le_bytes();
        bytes[offset as usize] = value;
        self.ssip = u32::from_le_bytes(bytes) & 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SupervisorSoftwareInterrupt;
    use rvsim_system::{Addressable, InterruptLine};

    const SSIP_BASE: u64 = 0x6000_0000;

    #[test]
    fn raises_supervisor_software_interrupt_when_ssip_is_set() {
        let mut device = SupervisorSoftwareInterrupt::new(SSIP_BASE);

        write_u32(&mut device, SSIP_BASE, 1);

        assert_eq!(read_u32(&mut device, SSIP_BASE), 1);
        assert_eq!(
            device.pending_interrupt(),
            Some(InterruptLine::SupervisorSoftware)
        );
    }

    #[test]
    fn clears_supervisor_software_interrupt_when_ssip_is_reset() {
        let mut device = SupervisorSoftwareInterrupt::new(SSIP_BASE);

        write_u32(&mut device, SSIP_BASE, 1);
        write_u32(&mut device, SSIP_BASE, 0);

        assert_eq!(read_u32(&mut device, SSIP_BASE), 0);
        assert_eq!(device.pending_interrupt(), None);
    }

    fn read_u32(device: &mut SupervisorSoftwareInterrupt, addr: u64) -> u32 {
        let mut bytes = [0; 4];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = device
                .load8(addr + offset as u64)
                .expect("ssip register read should succeed");
        }
        u32::from_le_bytes(bytes)
    }

    fn write_u32(device: &mut SupervisorSoftwareInterrupt, addr: u64, value: u32) {
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            device
                .store8(addr + offset as u64, byte)
                .expect("ssip register write should succeed");
        }
    }
}
