use rvsim_system::{Address, AddressRange, Addressable, BusError, InterruptLine, InterruptSet};

const MTIME_LOW_OFFSET: Address = 0;
const MTIME_HIGH_OFFSET: Address = 4;
const MTIMECMP_LOW_OFFSET: Address = 8;
const MTIMECMP_HIGH_OFFSET: Address = 12;

/// A minimal memory-mapped machine timer with `mtime` and `mtimecmp`.
#[derive(Debug, Clone)]
pub struct MachineTimer {
    range: AddressRange,
    mtime: u64,
    mtimecmp: u64,
}

impl MachineTimer {
    #[must_use]
    pub fn new(base: Address) -> Self {
        Self {
            range: AddressRange::new(base, 16),
            mtime: 0,
            mtimecmp: u64::MAX,
        }
    }

    #[must_use]
    pub const fn mtime(&self) -> u64 {
        self.mtime
    }

    fn offset(&self, addr: Address) -> Result<Address, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }
        Ok(addr - self.range.start)
    }

    fn read_register_byte(&self, offset: Address) -> u8 {
        let (value, byte_offset) = match offset {
            MTIME_LOW_OFFSET..=3 => (self.mtime as u32, offset as usize),
            MTIME_HIGH_OFFSET..=7 => ((self.mtime >> 32) as u32, (offset - 4) as usize),
            MTIMECMP_LOW_OFFSET..=11 => (self.mtimecmp as u32, (offset - 8) as usize),
            MTIMECMP_HIGH_OFFSET..=15 => ((self.mtimecmp >> 32) as u32, (offset - 12) as usize),
            _ => (0, 0),
        };

        value.to_le_bytes()[byte_offset]
    }

    fn write_register_byte(&mut self, offset: Address, value: u8) {
        match offset {
            MTIME_LOW_OFFSET..=3 => {
                let mut bytes = (self.mtime as u32).to_le_bytes();
                bytes[offset as usize] = value;
                self.mtime = (self.mtime & !0xffff_ffff) | u64::from(u32::from_le_bytes(bytes));
            }
            MTIME_HIGH_OFFSET..=7 => {
                let mut bytes = ((self.mtime >> 32) as u32).to_le_bytes();
                bytes[(offset - 4) as usize] = value;
                self.mtime =
                    (u64::from(u32::from_le_bytes(bytes)) << 32) | (self.mtime & 0xffff_ffff);
            }
            MTIMECMP_LOW_OFFSET..=11 => {
                let mut bytes = (self.mtimecmp as u32).to_le_bytes();
                bytes[(offset - 8) as usize] = value;
                self.mtimecmp =
                    (self.mtimecmp & !0xffff_ffff) | u64::from(u32::from_le_bytes(bytes));
            }
            MTIMECMP_HIGH_OFFSET..=15 => {
                let mut bytes = ((self.mtimecmp >> 32) as u32).to_le_bytes();
                bytes[(offset - 12) as usize] = value;
                self.mtimecmp =
                    (u64::from(u32::from_le_bytes(bytes)) << 32) | (self.mtimecmp & 0xffff_ffff);
            }
            _ => {}
        }
    }
}

impl Addressable for MachineTimer {
    fn name(&self) -> &'static str {
        "machine-timer"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.mtime = 0;
        self.mtimecmp = u64::MAX;
    }

    fn tick(&mut self) {
        self.mtime = self.mtime.wrapping_add(1);
    }

    fn machine_time(&self) -> Option<u64> {
        Some(self.mtime)
    }

    fn pending_interrupts(&self) -> InterruptSet {
        if self.mtime >= self.mtimecmp {
            InterruptSet::from(InterruptLine::MachineTimer)
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
        self.write_register_byte(offset, value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MachineTimer;
    use rvsim_system::{Addressable, InterruptLine};

    const TIMER_BASE: u64 = 0x3000_0000;

    #[test]
    fn ticks_until_compare_and_raises_machine_timer_interrupt() {
        let mut timer = MachineTimer::new(TIMER_BASE);

        write_u32(&mut timer, TIMER_BASE + 8, 3);
        write_u32(&mut timer, TIMER_BASE + 12, 0);

        assert_eq!(read_u32(&mut timer, TIMER_BASE), 0);
        assert_eq!(timer.pending_interrupt(), None);

        timer.tick();
        timer.tick();

        assert_eq!(read_u32(&mut timer, TIMER_BASE), 2);
        assert_eq!(timer.pending_interrupt(), None);

        timer.tick();

        assert_eq!(read_u32(&mut timer, TIMER_BASE), 3);
        assert_eq!(timer.pending_interrupt(), Some(InterruptLine::MachineTimer));
    }

    #[test]
    fn reset_restores_counter_and_disarms_compare() {
        let mut timer = MachineTimer::new(TIMER_BASE);

        write_u32(&mut timer, TIMER_BASE + 8, 1);
        write_u32(&mut timer, TIMER_BASE + 12, 0);
        timer.tick();
        assert_eq!(timer.pending_interrupt(), Some(InterruptLine::MachineTimer));

        timer.reset();

        assert_eq!(read_u32(&mut timer, TIMER_BASE), 0);
        assert_eq!(read_u32(&mut timer, TIMER_BASE + 4), 0);
        assert_eq!(read_u32(&mut timer, TIMER_BASE + 8), u32::MAX);
        assert_eq!(read_u32(&mut timer, TIMER_BASE + 12), u32::MAX);
        assert_eq!(timer.pending_interrupt(), None);
    }

    #[test]
    fn exposes_current_mtime_as_machine_time_source() {
        let mut timer = MachineTimer::new(TIMER_BASE);

        assert_eq!(timer.machine_time(), Some(0));

        write_u32(&mut timer, TIMER_BASE, 0x1234_5678);
        write_u32(&mut timer, TIMER_BASE + 4, 1);

        assert_eq!(timer.machine_time(), Some(0x0000_0001_1234_5678));

        timer.tick();

        assert_eq!(timer.machine_time(), Some(0x0000_0001_1234_5679));
        assert_eq!(timer.mtime(), 0x0000_0001_1234_5679);
    }

    fn read_u32(timer: &mut MachineTimer, addr: u64) -> u32 {
        let mut bytes = [0; 4];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = timer
                .load8(addr + offset as u64)
                .expect("timer register read should succeed");
        }
        u32::from_le_bytes(bytes)
    }

    fn write_u32(timer: &mut MachineTimer, addr: u64, value: u32) {
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            timer
                .store8(addr + offset as u64, byte)
                .expect("timer register write should succeed");
        }
    }
}
