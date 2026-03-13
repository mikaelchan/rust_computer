//! Address-decoded bus implementation for memory-mapped devices.

use core::fmt;

use crate::bus::{Address, AddressRange, Addressable, Bus, BusError, InterruptSet};

struct DeviceSlot {
    range: AddressRange,
    name: &'static str,
    device: Box<dyn Addressable>,
}

/// A simple bus that routes accesses to the first mapped device that contains the address.
#[derive(Default)]
pub struct MemoryMap {
    devices: Vec<DeviceSlot>,
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
        for slot in &mut self.devices {
            slot.device.reset();
        }
    }

    fn find_device_mut(&mut self, addr: Address) -> Option<&mut DeviceSlot> {
        self.devices
            .iter_mut()
            .find(|slot| slot.range.contains(addr))
    }
}

impl Bus for MemoryMap {
    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        self.find_device_mut(addr)
            .ok_or(BusError::UnmappedAddress { addr })?
            .device
            .load8(addr)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        self.find_device_mut(addr)
            .ok_or(BusError::UnmappedAddress { addr })?
            .device
            .store8(addr, value)
    }

    fn tick(&mut self) {
        for slot in &mut self.devices {
            slot.device.tick();
        }
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
        debug.field("devices", &devices).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryMap;
    use crate::{AddressRange, Addressable, Bus, BusError, InterruptLine, InterruptSet};

    #[derive(Debug)]
    struct CounterDevice {
        range: AddressRange,
        value: u8,
        interrupts: InterruptSet,
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
    }

    #[test]
    fn routes_to_mapped_device() {
        let mut map = MemoryMap::new();
        map.map_device(CounterDevice {
            range: AddressRange::new(0x1000, 4),
            value: 0,
            interrupts: InterruptSet::empty(),
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
        })
        .expect("timer-like device should map");
        map.map_device(CounterDevice {
            range: AddressRange::new(0x2000, 4),
            value: 0,
            interrupts: InterruptSet::from(InterruptLine::MachineExternal),
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
}
