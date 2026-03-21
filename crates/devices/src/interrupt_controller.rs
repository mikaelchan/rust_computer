use rvsim_system::{Address, AddressRange, Addressable, BusError, InterruptLine, InterruptSet};

const PENDING_OFFSET: Address = 0;
const ENABLE_OFFSET: Address = 4;
const SET_PENDING_OFFSET: Address = 8;
const CLAIM_COMPLETE_OFFSET: Address = 12;
const ROUTE_OFFSET: Address = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterruptRoute {
    MachineExternal,
    SupervisorExternal,
}

impl InterruptRoute {
    const fn from_register(value: u32) -> Self {
        if (value & 1) != 0 {
            Self::SupervisorExternal
        } else {
            Self::MachineExternal
        }
    }

    const fn register_bits(self) -> u32 {
        match self {
            Self::MachineExternal => 0,
            Self::SupervisorExternal => 1,
        }
    }

    const fn interrupt_line(self) -> InterruptLine {
        match self {
            Self::MachineExternal => InterruptLine::MachineExternal,
            Self::SupervisorExternal => InterruptLine::SupervisorExternal,
        }
    }
}

/// A minimal external interrupt controller with 32 software-visible sources.
///
/// The `claim/complete` register is modeled as a 32-bit MMIO word even though the
/// bus currently issues byte accesses. Reads therefore latch one claimed source
/// ID on the first byte and serve the remaining bytes from that snapshot.
#[derive(Debug, Clone)]
pub struct InterruptController {
    range: AddressRange,
    pending: u32,
    enable: u32,
    claimed: u32,
    route: InterruptRoute,
    claim_latch: Option<u32>,
    complete_staging: [u8; 4],
}

impl InterruptController {
    #[must_use]
    pub fn new(base: Address) -> Self {
        Self {
            range: AddressRange::new(base, 20),
            pending: 0,
            enable: 0,
            claimed: 0,
            route: InterruptRoute::MachineExternal,
            claim_latch: None,
            complete_staging: [0; 4],
        }
    }

    fn offset(&self, addr: Address) -> Result<Address, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }

        Ok(addr - self.range.start)
    }

    fn enabled_pending(&self) -> u32 {
        self.pending & self.enable
    }

    fn source_mask(source_id: u32) -> u32 {
        match source_id {
            1..=32 => 1_u32 << (source_id - 1),
            _ => 0,
        }
    }

    fn highest_priority_pending_source(&self) -> u32 {
        let enabled_pending = self.enabled_pending();
        if enabled_pending == 0 {
            0
        } else {
            enabled_pending.trailing_zeros() + 1
        }
    }

    fn claim_next(&mut self) -> u32 {
        let source_id = self.highest_priority_pending_source();
        if source_id != 0 {
            let mask = Self::source_mask(source_id);
            self.pending &= !mask;
            self.claimed |= mask;
        }

        source_id
    }

    fn complete_source(&mut self, source_id: u32) {
        self.claimed &= !Self::source_mask(source_id);
    }

    fn read_register_byte(&self, offset: Address) -> u8 {
        let (value, byte_offset) = match offset {
            PENDING_OFFSET..=3 => (self.pending, offset as usize),
            ENABLE_OFFSET..=7 => (self.enable, (offset - 4) as usize),
            SET_PENDING_OFFSET..=11 => (0, 0),
            CLAIM_COMPLETE_OFFSET..=15 => {
                (self.claim_latch.unwrap_or_default(), (offset - 12) as usize)
            }
            ROUTE_OFFSET..=19 => (self.route.register_bits(), (offset - 16) as usize),
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
        self.claimed = 0;
        self.route = InterruptRoute::MachineExternal;
        self.claim_latch = None;
        self.complete_staging = [0; 4];
    }

    fn pending_interrupts(&self) -> InterruptSet {
        if self.enabled_pending() != 0 {
            InterruptSet::from(self.route.interrupt_line())
        } else {
            InterruptSet::empty()
        }
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let offset = self.offset(addr)?;
        if offset == CLAIM_COMPLETE_OFFSET {
            self.claim_latch = Some(self.claim_next());
        }

        let value = self.read_register_byte(offset);
        if offset == CLAIM_COMPLETE_OFFSET + 3 {
            self.claim_latch = None;
        }

        Ok(value)
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
                self.pending |= mask & !self.claimed;
                Ok(())
            }
            CLAIM_COMPLETE_OFFSET..=15 => {
                self.complete_staging[(offset - 12) as usize] = value;
                if offset == CLAIM_COMPLETE_OFFSET + 3 {
                    let source_id = u32::from_le_bytes(self.complete_staging);
                    self.complete_source(source_id);
                    self.complete_staging = [0; 4];
                }
                Ok(())
            }
            ROUTE_OFFSET..=19 => {
                let mut bytes = self.route.register_bits().to_le_bytes();
                bytes[(offset - 16) as usize] = value;
                self.route = InterruptRoute::from_register(u32::from_le_bytes(bytes));
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
    fn claim_register_selects_highest_priority_pending_source() {
        let mut controller = InterruptController::new(CONTROLLER_BASE);

        write_u32(&mut controller, CONTROLLER_BASE + 4, 0b0101);
        write_u32(&mut controller, CONTROLLER_BASE + 8, 0b0101);

        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE + 12), 1);
        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE), 0b0100);
        assert_eq!(
            controller.pending_interrupt(),
            Some(InterruptLine::MachineExternal)
        );

        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE + 12), 3);
        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE), 0);
        assert_eq!(controller.pending_interrupt(), None);
    }

    #[test]
    fn completion_allows_source_to_be_raised_again() {
        let mut controller = InterruptController::new(CONTROLLER_BASE);

        write_u32(&mut controller, CONTROLLER_BASE + 4, 0b0001);
        write_u32(&mut controller, CONTROLLER_BASE + 8, 0b0001);

        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE + 12), 1);
        assert_eq!(controller.pending_interrupt(), None);

        write_u32(&mut controller, CONTROLLER_BASE + 8, 0b0001);
        assert_eq!(controller.pending_interrupt(), None);

        write_u32(&mut controller, CONTROLLER_BASE + 12, 1);
        write_u32(&mut controller, CONTROLLER_BASE + 8, 0b0001);
        assert_eq!(
            controller.pending_interrupt(),
            Some(InterruptLine::MachineExternal)
        );
    }

    #[test]
    fn route_register_can_raise_supervisor_external_interrupts() {
        let mut controller = InterruptController::new(CONTROLLER_BASE);

        write_u32(&mut controller, CONTROLLER_BASE + 4, 0b0001);
        write_u32(&mut controller, CONTROLLER_BASE + 16, 1);
        write_u32(&mut controller, CONTROLLER_BASE + 8, 0b0001);

        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE + 16), 1);
        assert_eq!(
            controller.pending_interrupt(),
            Some(InterruptLine::SupervisorExternal)
        );
    }

    #[test]
    fn route_register_masks_reserved_bits() {
        let mut controller = InterruptController::new(CONTROLLER_BASE);

        write_u32(&mut controller, CONTROLLER_BASE + 16, u32::MAX);

        assert_eq!(read_u32(&mut controller, CONTROLLER_BASE + 16), 1);
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
