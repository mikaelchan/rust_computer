use core::cell::Cell;

use rvsim_system::{AccessKind, Address, AddressRange, Addressable, BusError};

/// Timing parameters for the simple DRAM model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DramConfig {
    row_size: usize,
    row_miss_latency: u32,
    row_hit_latency: u32,
    burst_latency: u32,
}

impl DramConfig {
    #[must_use]
    pub fn new(
        row_size: usize,
        row_miss_latency: u32,
        row_hit_latency: u32,
        burst_latency: u32,
    ) -> Self {
        assert!(row_size > 0, "dram row size must be non-zero");
        assert!(
            row_size.is_power_of_two() && row_size % 4 == 0,
            "dram row size must be a power-of-two multiple of four bytes"
        );
        assert!(
            row_miss_latency >= row_hit_latency,
            "row miss latency must not be lower than row hit latency"
        );
        assert!(
            row_hit_latency >= burst_latency,
            "row hit latency must not be lower than burst latency"
        );

        Self {
            row_size,
            row_miss_latency,
            row_hit_latency,
            burst_latency,
        }
    }

    #[must_use]
    pub fn row_size(&self) -> usize {
        self.row_size
    }

    #[must_use]
    pub fn row_miss_latency(&self) -> u32 {
        self.row_miss_latency
    }

    #[must_use]
    pub fn row_hit_latency(&self) -> u32 {
        self.row_hit_latency
    }

    #[must_use]
    pub fn burst_latency(&self) -> u32 {
        self.burst_latency
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingAccess {
    start: Address,
    row_index: u64,
    next_burst_addr: Option<Address>,
}

/// DRAM-like memory with an open-row model and cheaper sequential bursts.
#[derive(Debug, Clone)]
pub struct Dram {
    range: AddressRange,
    data: Vec<u8>,
    config: DramConfig,
    open_row: Cell<Option<u64>>,
    next_burst_addr: Cell<Option<Address>>,
    pending_access: Cell<Option<PendingAccess>>,
}

impl Dram {
    #[must_use]
    pub fn new(base: Address, size: usize, config: DramConfig) -> Self {
        Self {
            range: AddressRange::new(base, size as u64),
            data: vec![0; size],
            config,
            open_row: Cell::new(None),
            next_burst_addr: Cell::new(None),
            pending_access: Cell::new(None),
        }
    }

    #[must_use]
    pub fn config(&self) -> DramConfig {
        self.config
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

    fn row_index(&self, addr: Address) -> u64 {
        (addr - self.range.start) / self.config.row_size as u64
    }

    fn next_burst_addr(&self, addr: Address, width: usize, row_index: u64) -> Option<Address> {
        let next_addr = addr.checked_add(width as u64)?;
        (self.row_index(next_addr.saturating_sub(1)) == row_index).then_some(next_addr)
    }

    fn commit_pending_access(&self, addr: Address) {
        let Some(pending) = self.pending_access.get() else {
            return;
        };

        if pending.start == addr {
            self.open_row.set(Some(pending.row_index));
            self.next_burst_addr.set(pending.next_burst_addr);
            self.pending_access.set(None);
        }
    }
}

impl Addressable for Dram {
    fn name(&self) -> &'static str {
        "dram"
    }

    fn address_range(&self) -> AddressRange {
        self.range
    }

    fn reset(&mut self) {
        self.data.fill(0);
        self.open_row.set(None);
        self.next_burst_addr.set(None);
        self.pending_access.set(None);
    }

    fn access_latency(&self, addr: Address, _kind: AccessKind, width: usize) -> u32 {
        let row_index = self.row_index(addr);
        let latency = if self.open_row.get() == Some(row_index) {
            if self.next_burst_addr.get() == Some(addr) {
                self.config.burst_latency
            } else {
                self.config.row_hit_latency
            }
        } else {
            self.config.row_miss_latency
        };

        self.pending_access.set(Some(PendingAccess {
            start: addr,
            row_index,
            next_burst_addr: self.next_burst_addr(addr, width, row_index),
        }));

        latency
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        self.commit_pending_access(addr);
        let offset = self.offset(addr)?;
        self.data
            .get(offset)
            .copied()
            .ok_or(BusError::UnmappedAddress { addr })
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        self.commit_pending_access(addr);
        let offset = self.offset(addr)?;
        let byte = self
            .data
            .get_mut(offset)
            .ok_or(BusError::UnmappedAddress { addr })?;
        *byte = value;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Dram, DramConfig};
    use rvsim_system::{Bus, BusError, MemoryMap};

    fn run_load32(map: &mut MemoryMap, addr: u64) -> (u32, u32) {
        let mut stalled_cycles = 0;
        loop {
            match map.load32(addr) {
                Ok(word) => return (word, stalled_cycles),
                Err(BusError::Busy { .. }) => {
                    stalled_cycles += 1;
                    map.tick();
                }
                Err(error) => panic!("unexpected dram load error: {error}"),
            }
        }
    }

    #[test]
    fn row_miss_and_row_hit_have_distinct_latencies() {
        let mut map = MemoryMap::new();
        map.map_device(Dram::new(0, 256, DramConfig::new(16, 5, 2, 1)))
            .expect("dram should map");

        assert_eq!(run_load32(&mut map, 0).1, 5);
        assert_eq!(run_load32(&mut map, 12).1, 2);
        assert_eq!(run_load32(&mut map, 16).1, 5);
    }

    #[test]
    fn sequential_accesses_use_burst_latency() {
        let mut map = MemoryMap::new();
        map.map_device(Dram::new(0, 256, DramConfig::new(32, 6, 3, 1)))
            .expect("dram should map");

        assert_eq!(run_load32(&mut map, 0).1, 6);
        assert_eq!(run_load32(&mut map, 4).1, 1);
        assert_eq!(run_load32(&mut map, 8).1, 1);
        assert_eq!(run_load32(&mut map, 20).1, 3);
    }

    #[test]
    fn stores_update_memory_contents() {
        let mut map = MemoryMap::new();
        map.map_device(Dram::new(0, 256, DramConfig::new(32, 4, 2, 1)))
            .expect("dram should map");

        loop {
            match map.store32(0, 0x1234_5678) {
                Ok(()) => break,
                Err(BusError::Busy { .. }) => map.tick(),
                Err(error) => panic!("unexpected dram store error: {error}"),
            }
        }

        assert_eq!(run_load32(&mut map, 0).0, 0x1234_5678);
    }
}
