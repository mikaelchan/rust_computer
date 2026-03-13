use rvsim_system::{Address, AddressRange, Addressable, BusError};

const TX_OFFSET: Address = 0;
const STATUS_OFFSET: Address = 4;

/// A tiny UART model with one transmit register and one status register.
#[derive(Debug, Clone)]
pub struct SimpleUart {
    range: AddressRange,
    tx_buffer: Vec<u8>,
}

impl SimpleUart {
    #[must_use]
    pub fn new(base: Address) -> Self {
        Self {
            range: AddressRange::new(base, 8),
            tx_buffer: Vec::new(),
        }
    }

    #[must_use]
    pub fn drained_output(&self) -> String {
        String::from_utf8_lossy(&self.tx_buffer).into_owned()
    }

    fn offset(&self, addr: Address) -> Result<Address, BusError> {
        if !self.range.contains(addr) {
            return Err(BusError::UnmappedAddress { addr });
        }
        Ok(addr - self.range.start)
    }
}

impl Addressable for SimpleUart {
    fn name(&self) -> &'static str {
        "simple-uart"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.tx_buffer.clear();
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        match self.offset(addr)? {
            TX_OFFSET => Ok(0),
            STATUS_OFFSET => Ok(1),
            _ => Ok(0),
        }
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        match self.offset(addr)? {
            TX_OFFSET => {
                self.tx_buffer.push(value);
                Ok(())
            }
            STATUS_OFFSET => Ok(()),
            _ => Err(BusError::UnmappedAddress { addr }),
        }
    }
}
