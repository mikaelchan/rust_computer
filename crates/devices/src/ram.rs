use rvsim_system::{Address, AddressRange, Addressable, BusError};

/// Read-write memory backed by a byte vector.
#[derive(Debug, Clone)]
pub struct Ram {
    range: AddressRange,
    data: Vec<u8>,
}

impl Ram {
    #[must_use]
    pub fn new(base: Address, size: usize) -> Self {
        Self {
            range: AddressRange::new(base, size as u64),
            data: vec![0; size],
        }
    }

    #[must_use]
    pub fn read_word(&self, address: Address) -> Option<u32> {
        let offset = self.offset(address).ok()?;
        let bytes: [u8; 4] = self.data.get(offset..offset + 4)?.try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    fn offset(&self, addr: Address) -> Result<usize, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }
        Ok((addr - self.range.start) as usize)
    }
}

impl Addressable for Ram {
    fn name(&self) -> &'static str {
        "ram"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.data.fill(0);
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let offset = self.offset(addr)?;
        self.data
            .get(offset)
            .copied()
            .ok_or(BusError::UnmappedAddress { addr })
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        let offset = self.offset(addr)?;
        let byte = self
            .data
            .get_mut(offset)
            .ok_or(BusError::UnmappedAddress { addr })?;
        *byte = value;
        Ok(())
    }
}
