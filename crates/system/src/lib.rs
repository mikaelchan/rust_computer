//! System-level simulation primitives shared by CPU cores and devices.

pub mod bus;
pub mod clock;
pub mod component;
pub mod machine;
pub mod memory_map;

pub use bus::{Address, AddressRange, Addressable, Bus, BusError};
pub use clock::Clock;
pub use component::{CpuCycle, Processor, SimComponent};
pub use machine::Machine;
pub use memory_map::MemoryMap;
