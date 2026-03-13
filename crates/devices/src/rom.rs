use rvsim_system::{Address, AddressRange, Addressable, BusError};

/// Immutable program memory backed by bytes.
#[derive(Debug, Clone)]
pub struct Rom {
    range: AddressRange,
    data: Vec<u8>,
}

impl Rom {
    #[must_use]
    pub fn from_words(base: Address, words: &[u32]) -> Self {
        let mut data = Vec::with_capacity(words.len() * 4);
        for word in words {
            data.extend_from_slice(&word.to_le_bytes());
        }

        Self {
            range: AddressRange::new(base, data.len() as u64),
            data,
        }
    }

    fn offset(&self, addr: Address) -> Result<usize, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }
        Ok((addr - self.range.start) as usize)
    }
}

impl Addressable for Rom {
    fn name(&self) -> &'static str {
        "rom"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let offset = self.offset(addr)?;
        self.data
            .get(offset)
            .copied()
            .ok_or(BusError::UnmappedAddress { addr })
    }

    fn store8(&mut self, addr: Address, _value: u8) -> Result<(), BusError> {
        Err(BusError::ReadOnlyAddress { addr })
    }
}
