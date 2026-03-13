use rvsim_system::{Address, AddressRange, Addressable, BusError, InterruptLine, InterruptSet};

const PENDING_OFFSET: Address = 0;
const ENABLE_OFFSET: Address = 4;
const SET_PENDING_OFFSET: Address = 8;
const CLEAR_PENDING_OFFSET: Address = 12;

/// A minimal external interrupt controller with 32 software-visible sources.
#[derive(Debug, Clone)]
pub struct InterruptController {
    range: AddressRange,
    pending: u32,
    enable: u32,
}

impl InterruptController {
    #[must_use]
    pub fn new(base: Address) -> Self {
        Self {
            range: AddressRange::new(base, 16),
            pending: 0,
            enable: 0,
        }
    }

    fn offset(&self, addr: Address) -> Result<Address, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }

        Ok(addr - self.range.start)
    }

    fn read_register_byte(&self, offset: Address) -> u8 {
        let (value, byte_offset) = match offset {
            PENDING_OFFSET..=3 => (self.pending, offset as usize),
            ENABLE_OFFSET..=7 => (self.enable, (offset - 4) as usize),
            SET_PENDING_OFFSET..=11 | CLEAR_PENDING_OFFSET..=15 => (0, 0),
            _ => (0, 0),
        };

        value.to_le_bytes()[byte_offset]
    }
}

impl Addressable for InterruptController {
    fn name(&self) -> &'static str {
        "interrupt-controller"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.pending = 0;
        self.enable = 0;
    }

    fn pending_interrupts(&self) -> InterruptSet {
        if (self.pending & self.enable) != 0 {
            InterruptSet::from(InterruptLine::MachineExternal)
        } else {
            InterruptSet::empty()
        }
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let offset = self.offset(addr)?;
        Ok(self.read_register_byte(offset))
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        let offset = self.offset(addr)?;
        let bit_shift = ((offset & 0b11) * 8) as u32;
        let mask = u32::from(value) << bit_shift;

        match offset {
            PENDING_OFFSET..=3 => Err(BusError::ReadOnlyAddress { addr }),
            ENABLE_OFFSET..=7 => {
                let mut bytes = self.enable.to_le_bytes();
                bytes[(offset - 4) as usize] = value;
                self.enable = u32::from_le_bytes(bytes);
                Ok(())
            }
            SET_PENDING_OFFSET..=11 => {
                self.pending |= mask;
                Ok(())
            }
            CLEAR_PENDING_OFFSET..=15 => {
                self.pending &= !mask;
                Ok(())
            }
            _ => Err(BusError::UnmappedAddress { addr }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InterruptController;
    use rvsim_system::{Addressable, BusError, InterruptLine};

    const CONTROLLER_BASE: u64 = 0x4000_0000;

    #[test]
    fn raises_machine_external_interrupt_when_enabled_pending_source_exists() {
        let mut controller = InterruptController::new(CONTROLLER_BASE);

        write_u32(&mut controller, CONTROLLER_BASE + 4, 0b0010);
        write_u32(&mut controller, CONTROLLER_BASE + 8, 0b0010);

        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE), 0b0010);
        assert_eq!(
            controller.pending_interrupt(),
            Some(InterruptLine::MachineExternal)
        );
    }

    #[test]
    fn clear_register_acknowledges_pending_sources() {
        let mut controller = InterruptController::new(CONTROLLER_BASE);

        write_u32(&mut controller, CONTROLLER_BASE + 4, 0b0101);
        write_u32(&mut controller, CONTROLLER_BASE + 8, 0b0101);
        write_u32(&mut controller, CONTROLLER_BASE + 12, 0b0001);

        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE), 0b0100);
        assert_eq!(
            controller.pending_interrupt(),
            Some(InterruptLine::MachineExternal)
        );

        write_u32(&mut controller, CONTROLLER_BASE + 12, 0b0100);
        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE), 0);
        assert_eq!(controller.pending_interrupt(), None);
    }

    #[test]
    fn rejects_direct_writes_to_pending_register() {
        let mut controller = InterruptController::new(CONTROLLER_BASE);

        let error = controller
            .store8(CONTROLLER_BASE, 1)
            .expect_err("pending register should be read-only");
        assert_eq!(
            error,
            BusError::ReadOnlyAddress {
                addr: CONTROLLER_BASE
            }
        );
    }

    fn read_u32(controller: &mut InterruptController, addr: u64) -> u32 {
        let mut bytes = [0; 4];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = controller
                .load8(addr + offset as u64)
                .expect("controller register read should succeed");
        }
        u32::from_le_bytes(bytes)
    }

    fn write_u32(controller: &mut InterruptController, addr: u64, value: u32) {
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            controller
                .store8(addr + offset as u64, byte)
                .expect("controller register write should succeed");
        }
    }
}
