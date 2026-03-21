use rvsim_system::{Address, AddressRange, Addressable, BusError, InterruptLine, InterruptSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockCommand {
    ReadBlock,
    WriteBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveCommand {
    kind: BlockCommand,
    remaining_cycles: u32,
}

/// A simple RAM-backed MMIO block device with a one-block staging window.
///
/// Software selects a block index, reads or writes the staging window, and then
/// starts either a block read or block write command through the control
/// register. Commands complete after a fixed number of ticks and may raise a
/// machine external interrupt if enabled.
#[derive(Debug, Clone)]
pub struct BlockDevice {
    range: AddressRange,
    block_count: u32,
    block_bytes: usize,
    operation_latency: u32,
    selected_block: u32,
    irq_enabled: bool,
    done: bool,
    error: bool,
    staging: Box<[u8]>,
    storage: Vec<u8>,
    active: Option<ActiveCommand>,
}

impl BlockDevice {
    pub const BLOCK_INDEX_OFFSET: Address = 0;
    pub const CONTROL_OFFSET: Address = 4;
    pub const BLOCK_COUNT_OFFSET: Address = 8;
    pub const BLOCK_BYTES_OFFSET: Address = 12;
    pub const DATA_WINDOW_OFFSET: Address = 16;

    pub const CONTROL_START_READ: u32 = 1 << 0;
    pub const CONTROL_START_WRITE: u32 = 1 << 1;
    pub const STATUS_BUSY: u32 = 1 << 2;
    pub const STATUS_DONE: u32 = 1 << 3;
    pub const CONTROL_IRQ_ENABLE: u32 = 1 << 4;
    pub const STATUS_ERROR: u32 = 1 << 5;

    #[must_use]
    pub fn new(
        base: Address,
        block_count: u32,
        block_bytes: usize,
        operation_latency: u32,
    ) -> Self {
        assert!(block_count > 0, "block device requires at least one block");
        assert!(
            block_bytes > 0 && block_bytes % 4 == 0,
            "block device block size must be a non-zero multiple of four bytes"
        );

        let total_bytes = block_count as usize * block_bytes;
        Self {
            range: AddressRange::new(base, Self::DATA_WINDOW_OFFSET + block_bytes as u64),
            block_count,
            block_bytes,
            operation_latency,
            selected_block: 0,
            irq_enabled: false,
            done: false,
            error: false,
            staging: vec![0; block_bytes].into_boxed_slice(),
            storage: vec![0; total_bytes],
            active: None,
        }
    }

    #[must_use]
    pub const fn block_count(&self) -> u32 {
        self.block_count
    }

    #[must_use]
    pub const fn block_bytes(&self) -> usize {
        self.block_bytes
    }

    #[must_use]
    pub fn block(&self, index: u32) -> Option<&[u8]> {
        let range = self.block_range(index)?;
        Some(&self.storage[range])
    }

    pub fn write_block_contents(&mut self, index: u32, bytes: &[u8]) -> Result<(), BusError> {
        if bytes.len() != self.block_bytes {
            return Err(BusError::DeviceFault {
                addr: self.range.start + Self::DATA_WINDOW_OFFSET,
                message: format!(
                    "block image size {} does not match device block size {}",
                    bytes.len(),
                    self.block_bytes
                ),
            });
        }

        let Some(range) = self.block_range(index) else {
            return Err(BusError::DeviceFault {
                addr: self.range.start + Self::BLOCK_INDEX_OFFSET,
                message: format!("block index {index} is out of range"),
            });
        };

        self.storage[range].copy_from_slice(bytes);
        Ok(())
    }

    fn offset(&self, addr: Address) -> Result<Address, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }

        Ok(addr - self.range.start)
    }

    fn block_range(&self, index: u32) -> Option<std::ops::Range<usize>> {
        if index >= self.block_count {
            return None;
        }

        let start = index as usize * self.block_bytes;
        Some(start..start + self.block_bytes)
    }

    fn control_word(&self) -> u32 {
        let mut value = 0;
        if self.active.is_some() {
            value |= Self::STATUS_BUSY;
        }
        if self.done {
            value |= Self::STATUS_DONE;
        }
        if self.irq_enabled {
            value |= Self::CONTROL_IRQ_ENABLE;
        }
        if self.error {
            value |= Self::STATUS_ERROR;
        }
        value
    }

    fn finish_command(&mut self, kind: BlockCommand) {
        let Some(range) = self.block_range(self.selected_block) else {
            self.error = true;
            self.done = false;
            return;
        };

        match kind {
            BlockCommand::ReadBlock => {
                self.staging.copy_from_slice(&self.storage[range]);
            }
            BlockCommand::WriteBlock => {
                self.storage[range].copy_from_slice(&self.staging);
            }
        }

        self.done = true;
    }

    fn start_command(&mut self, kind: BlockCommand) {
        self.done = false;
        self.error = false;

        if self.active.is_some() {
            self.error = true;
            return;
        }

        if self.operation_latency == 0 {
            self.finish_command(kind);
        } else {
            self.active = Some(ActiveCommand {
                kind,
                remaining_cycles: self.operation_latency,
            });
        }
    }

    fn apply_control_byte(&mut self, value: u8) {
        let control = u32::from(value);
        self.irq_enabled = (control & Self::CONTROL_IRQ_ENABLE) != 0;

        if (control & Self::STATUS_DONE) != 0 {
            self.done = false;
        }
        if (control & Self::STATUS_ERROR) != 0 {
            self.error = false;
        }

        let start_read = (control & Self::CONTROL_START_READ) != 0;
        let start_write = (control & Self::CONTROL_START_WRITE) != 0;
        match (start_read, start_write) {
            (true, false) => self.start_command(BlockCommand::ReadBlock),
            (false, true) => self.start_command(BlockCommand::WriteBlock),
            (true, true) => {
                self.done = false;
                self.error = true;
            }
            (false, false) => {}
        }
    }

    fn read_register_byte(&self, offset: Address) -> Result<u8, BusError> {
        let byte = match offset {
            Self::BLOCK_INDEX_OFFSET..=3 => self.selected_block.to_le_bytes()[offset as usize],
            Self::CONTROL_OFFSET..=7 => {
                self.control_word().to_le_bytes()[(offset - Self::CONTROL_OFFSET) as usize]
            }
            Self::BLOCK_COUNT_OFFSET..=11 => {
                self.block_count.to_le_bytes()[(offset - Self::BLOCK_COUNT_OFFSET) as usize]
            }
            Self::BLOCK_BYTES_OFFSET..=15 => (self.block_bytes as u32).to_le_bytes()
                [(offset - Self::BLOCK_BYTES_OFFSET) as usize],
            Self::DATA_WINDOW_OFFSET.. => {
                let index = (offset - Self::DATA_WINDOW_OFFSET) as usize;
                *self.staging.get(index).ok_or(BusError::UnmappedAddress {
                    addr: self.range.start + offset,
                })?
            }
        };
        Ok(byte)
    }
}

impl Addressable for BlockDevice {
    fn name(&self) -> &'static str {
        "block-device"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.selected_block = 0;
        self.irq_enabled = false;
        self.done = false;
        self.error = false;
        self.staging.fill(0);
        self.storage.fill(0);
        self.active = None;
    }

    fn tick(&mut self) {
        let Some(active) = self.active else {
            return;
        };

        if active.remaining_cycles > 1 {
            self.active = Some(ActiveCommand {
                remaining_cycles: active.remaining_cycles - 1,
                ..active
            });
            return;
        }

        self.active = None;
        self.finish_command(active.kind);
    }

    fn pending_interrupts(&self) -> InterruptSet {
        if self.irq_enabled && (self.done || self.error) {
            InterruptSet::from(InterruptLine::MachineExternal)
        } else {
            InterruptSet::empty()
        }
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let offset = self.offset(addr)?;
        self.read_register_byte(offset)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        let offset = self.offset(addr)?;
        match offset {
            Self::BLOCK_INDEX_OFFSET..=3 => {
                let mut bytes = self.selected_block.to_le_bytes();
                bytes[offset as usize] = value;
                self.selected_block = u32::from_le_bytes(bytes);
                Ok(())
            }
            Self::CONTROL_OFFSET..=7 => {
                if offset == Self::CONTROL_OFFSET {
                    self.apply_control_byte(value);
                }
                Ok(())
            }
            Self::BLOCK_COUNT_OFFSET..=15 => Err(BusError::ReadOnlyAddress { addr }),
            Self::DATA_WINDOW_OFFSET.. => {
                let index = (offset - Self::DATA_WINDOW_OFFSET) as usize;
                let byte = self
                    .staging
                    .get_mut(index)
                    .ok_or(BusError::UnmappedAddress {
                        addr: self.range.start + offset,
                    })?;
                *byte = value;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rvsim_system::{Addressable, BusError, InterruptLine};

    use super::BlockDevice;

    const BLOCK_BASE: u64 = 0x7000_0000;
    const BLOCK_BYTES: usize = 16;

    #[test]
    fn read_command_copies_block_into_staging_window() {
        let mut device = BlockDevice::new(BLOCK_BASE, 4, BLOCK_BYTES, 2);
        device
            .write_block_contents(2, &block_bytes([11, 22, 33, 44]))
            .expect("block image should load");

        write_u32(&mut device, BLOCK_BASE + BlockDevice::BLOCK_INDEX_OFFSET, 2);
        write_u32(
            &mut device,
            BLOCK_BASE + BlockDevice::CONTROL_OFFSET,
            BlockDevice::CONTROL_START_READ | BlockDevice::CONTROL_IRQ_ENABLE,
        );

        assert_eq!(
            read_u32(&mut device, BLOCK_BASE + BlockDevice::CONTROL_OFFSET)
                & BlockDevice::STATUS_BUSY,
            BlockDevice::STATUS_BUSY
        );
        assert_eq!(device.pending_interrupt(), None);

        device.tick();
        assert_eq!(device.pending_interrupt(), None);
        device.tick();

        assert_eq!(
            read_u32(&mut device, BLOCK_BASE + BlockDevice::DATA_WINDOW_OFFSET),
            11
        );
        assert_eq!(
            read_u32(
                &mut device,
                BLOCK_BASE + BlockDevice::DATA_WINDOW_OFFSET + 4
            ),
            22
        );
        assert_eq!(
            device.pending_interrupt(),
            Some(InterruptLine::MachineExternal)
        );
        assert_eq!(
            read_u32(&mut device, BLOCK_BASE + BlockDevice::CONTROL_OFFSET)
                & BlockDevice::STATUS_DONE,
            BlockDevice::STATUS_DONE
        );
    }

    #[test]
    fn write_command_copies_staging_window_into_backing_store() {
        let mut device = BlockDevice::new(BLOCK_BASE, 4, BLOCK_BYTES, 1);

        write_u32(&mut device, BLOCK_BASE + BlockDevice::BLOCK_INDEX_OFFSET, 1);
        write_u32(
            &mut device,
            BLOCK_BASE + BlockDevice::DATA_WINDOW_OFFSET,
            0x1122_3344,
        );
        write_u32(
            &mut device,
            BLOCK_BASE + BlockDevice::DATA_WINDOW_OFFSET + 4,
            0x5566_7788,
        );
        write_u32(
            &mut device,
            BLOCK_BASE + BlockDevice::CONTROL_OFFSET,
            BlockDevice::CONTROL_START_WRITE,
        );

        device.tick();

        let block = device.block(1).expect("block 1 should exist");
        assert_eq!(
            &block[..8],
            &[0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55]
        );
        assert_eq!(
            read_u32(&mut device, BLOCK_BASE + BlockDevice::CONTROL_OFFSET)
                & BlockDevice::STATUS_DONE,
            BlockDevice::STATUS_DONE
        );
    }

    #[test]
    fn invalid_block_index_sets_error_and_interrupt() {
        let mut device = BlockDevice::new(BLOCK_BASE, 2, BLOCK_BYTES, 0);

        write_u32(&mut device, BLOCK_BASE + BlockDevice::BLOCK_INDEX_OFFSET, 9);
        write_u32(
            &mut device,
            BLOCK_BASE + BlockDevice::CONTROL_OFFSET,
            BlockDevice::CONTROL_START_READ | BlockDevice::CONTROL_IRQ_ENABLE,
        );

        assert_eq!(
            read_u32(&mut device, BLOCK_BASE + BlockDevice::CONTROL_OFFSET)
                & BlockDevice::STATUS_ERROR,
            BlockDevice::STATUS_ERROR
        );
        assert_eq!(
            device.pending_interrupt(),
            Some(InterruptLine::MachineExternal)
        );
    }

    #[test]
    fn block_count_and_block_size_registers_are_read_only() {
        let mut device = BlockDevice::new(BLOCK_BASE, 2, BLOCK_BYTES, 0);

        let error = device
            .store8(BLOCK_BASE + BlockDevice::BLOCK_COUNT_OFFSET, 0)
            .expect_err("block count register should be read-only");
        assert_eq!(
            error,
            BusError::ReadOnlyAddress {
                addr: BLOCK_BASE + BlockDevice::BLOCK_COUNT_OFFSET
            }
        );

        assert_eq!(
            read_u32(&mut device, BLOCK_BASE + BlockDevice::BLOCK_COUNT_OFFSET),
            2
        );
        assert_eq!(
            read_u32(&mut device, BLOCK_BASE + BlockDevice::BLOCK_BYTES_OFFSET),
            BLOCK_BYTES as u32
        );
    }

    fn block_bytes(words: [u32; 4]) -> [u8; BLOCK_BYTES] {
        let mut bytes = [0; BLOCK_BYTES];
        for (index, word) in words.into_iter().enumerate() {
            bytes[index * 4..(index + 1) * 4].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    fn read_u32(device: &mut BlockDevice, addr: u64) -> u32 {
        let mut bytes = [0; 4];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = device
                .load8(addr + offset as u64)
                .expect("device read should succeed");
        }
        u32::from_le_bytes(bytes)
    }

    fn write_u32(device: &mut BlockDevice, addr: u64, value: u32) {
        for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
            device
                .store8(addr + offset as u64, byte)
                .expect("device write should succeed");
        }
    }
}
