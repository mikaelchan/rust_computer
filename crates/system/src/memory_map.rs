//! Address-decoded bus implementation for memory-mapped devices.

use core::fmt;

use crate::bus::{AccessKind, Address, AddressRange, Addressable, Bus, BusError, InterruptSet};

struct DeviceSlot {
    range: AddressRange,
    name: &'static str,
    device: Box<dyn Addressable>,
}

/// A simple bus that routes accesses to the first mapped device that contains the address.
#[derive(Default)]
pub struct MemoryMap {
    devices: Vec<DeviceSlot>,
    busy_cycles: u32,
    access_ready: bool,
}

impl MemoryMap {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Map a device into the physical address space, rejecting overlapping regions.
    pub fn map_device<D>(&mut self, device: D) -> Result<(), BusError>
    where
        D: Addressable + 'static,
    {
        let range = device.address_range();
        for slot in &self.devices {
            if slot.range.overlaps(range) {
                return Err(BusError::DeviceFault {
                    addr: range.start,
                    message: format!("device {} overlaps with {}", device.name(), slot.name),
                });
            }
        }

        self.devices.push(DeviceSlot {
            range,
            name: device.name(),
            device: Box::new(device),
        });
        Ok(())
    }

    pub fn reset(&mut self) {
        self.busy_cycles = 0;
        self.access_ready = false;
        for slot in &mut self.devices {
            slot.device.reset();
        }
    }

    fn find_device_index(&self, addr: Address) -> Option<usize> {
        self.devices
            .iter()
            .position(|slot| slot.range.contains(addr))
    }

    fn begin_access(
        &mut self,
        addr: Address,
        kind: AccessKind,
        width: usize,
    ) -> Result<usize, BusError> {
        if self.busy_cycles > 0 {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_cycles,
            });
        }

        if self.access_ready {
            self.access_ready = false;
            return self
                .find_device_index(addr)
                .ok_or(BusError::UnmappedAddress { addr });
        }

        let index = self
            .find_device_index(addr)
            .ok_or(BusError::UnmappedAddress { addr })?;
        let latency = self.devices[index].device.access_latency(addr, kind, width);
        if latency > 0 {
            self.busy_cycles = latency;
            return Err(BusError::Busy {
                remaining_cycles: self.busy_cycles,
            });
        }

        Ok(index)
    }

    fn load_bytes<const N: usize>(
        &mut self,
        index: usize,
        addr: Address,
    ) -> Result<[u8; N], BusError> {
        let mut bytes = [0; N];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            *byte = self.devices[index].device.load8(addr + offset as u64)?;
        }
        Ok(bytes)
    }

    fn store_bytes<const N: usize>(
        &mut self,
        index: usize,
        addr: Address,
        bytes: [u8; N],
    ) -> Result<(), BusError> {
        for (offset, byte) in bytes.into_iter().enumerate() {
            self.devices[index]
                .device
                .store8(addr + offset as u64, byte)?;
        }
        Ok(())
    }
}

impl Bus for MemoryMap {
    fn reset(&mut self) {
        MemoryMap::reset(self);
    }

    fn fetch32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        let index = self.begin_access(addr, AccessKind::Fetch, 4)?;
        Ok(u32::from_le_bytes(self.load_bytes::<4>(index, addr)?))
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        let index = self.begin_access(addr, AccessKind::Load, 1)?;
        self.devices[index].device.load8(addr)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        let index = self.begin_access(addr, AccessKind::Store, 1)?;
        self.devices[index].device.store8(addr, value)
    }

    fn load16(&mut self, addr: Address) -> Result<u16, BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        let index = self.begin_access(addr, AccessKind::Load, 2)?;
        Ok(u16::from_le_bytes(self.load_bytes::<2>(index, addr)?))
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        let index = self.begin_access(addr, AccessKind::Load, 4)?;
        Ok(u32::from_le_bytes(self.load_bytes::<4>(index, addr)?))
    }

    fn store16(&mut self, addr: Address, value: u16) -> Result<(), BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        let index = self.begin_access(addr, AccessKind::Store, 2)?;
        self.store_bytes(index, addr, value.to_le_bytes())
    }

    fn store32(&mut self, addr: Address, value: u32) -> Result<(), BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        let index = self.begin_access(addr, AccessKind::Store, 4)?;
        self.store_bytes(index, addr, value.to_le_bytes())
    }

    fn tick(&mut self) {
        if self.busy_cycles > 0 {
            self.busy_cycles -= 1;
            if self.busy_cycles == 0 {
                self.access_ready = true;
            }
        }
        for slot in &mut self.devices {
            slot.device.tick();
        }
    }

    fn is_busy(&self) -> bool {
        self.busy_cycles > 0
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.devices
            .iter()
            .fold(InterruptSet::empty(), |interrupts, slot| {
                interrupts.union(slot.device.pending_interrupts())
            })
    }
}

impl fmt::Debug for MemoryMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("MemoryMap");
        let devices: Vec<_> = self
            .devices
            .iter()
            .map(|slot| (slot.name, slot.range.start, slot.range.size))
            .collect();
        debug
            .field("busy_cycles", &self.busy_cycles)
            .field("access_ready", &self.access_ready)
            .field("devices", &devices)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryMap;
    use crate::{
        AccessKind, AddressRange, Addressable, Bus, BusError, InterruptLine, InterruptSet,
    };

    #[derive(Debug)]
    struct CounterDevice {
        range: AddressRange,
        value: u8,
        interrupts: InterruptSet,
        latency_cycles: u32,
    }

    impl Addressable for CounterDevice {
        fn name(&self) -> &'static str {
            "counter"
        }

        fn address_range(&self) -> AddressRange {
            self.range
        }

        fn load8(&mut self, _addr: u64) -> Result<u8, BusError> {
            Ok(self.value)
        }

        fn store8(&mut self, _addr: u64, value: u8) -> Result<(), BusError> {
            self.value = value;
            Ok(())
        }

        fn pending_interrupts(&self) -> InterruptSet {
            self.interrupts
        }

        fn access_latency(&self, _addr: u64, _kind: AccessKind, _width: usize) -> u32 {
            self.latency_cycles
        }
    }

    #[test]
    fn routes_to_mapped_device() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x1000, 4),
            value: 0,
            interrupts: InterruptSet::empty(),
            latency_cycles: 0,
        })
        .expect("device should map");

        map.store8(0x1000, 7).expect("write should succeed");
        assert_eq!(map.load8(0x1000).expect("read should succeed"), 7);
    }

    #[test]
    fn aggregates_interrupts_across_devices() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x1000, 4),
            value: 0,
            interrupts: InterruptSet::from(InterruptLine::MachineTimer),
            latency_cycles: 0,
        })
        .expect("timer-like device should map");
        map.map_device(CounterDevice {
            range: AddressRange::new(0x2000, 4),
            value: 0,
            interrupts: InterruptSet::from(InterruptLine::MachineExternal),
            latency_cycles: 0,
        })
        .expect("external device should map");

        let interrupts = map.pending_interrupts();
        assert!(interrupts.contains(InterruptLine::MachineTimer));
        assert!(interrupts.contains(InterruptLine::MachineExternal));
        assert_eq!(
            interrupts.highest_priority(),
            Some(InterruptLine::MachineExternal)
        );
    }

    #[test]
    fn delays_access_until_busy_cycles_elapse() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x1000, 4),
            value: 9,
            interrupts: InterruptSet::empty(),
            latency_cycles: 2,
        })
        .expect("device should map");

        let error = map.load8(0x1000).expect_err("first access should stall");
        assert_eq!(
            error,
            BusError::Busy {
                remaining_cycles: 2
            }
        );
        assert!(map.is_busy());

        map.tick();
        let error = map
            .load8(0x1000)
            .expect_err("second access should still stall");
        assert_eq!(
            error,
            BusError::Busy {
                remaining_cycles: 1
            }
        );

        map.tick();
        assert_eq!(map.load8(0x1000).expect("access should complete"), 9);
        assert!(!map.is_busy());
    }
}
