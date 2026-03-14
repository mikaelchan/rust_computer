//! System-level simulation primitives shared by CPU cores and devices.

pub mod arbiter;
pub mod bus;
pub mod cache;
pub mod clock;
pub mod component;
pub mod machine;
pub mod memory_map;

pub use arbiter::{ArbiterBus, ArbiterStats};
pub use bus::{
    AccessKind, Address, AddressRange, Addressable, Bus, BusError, BusMaster, BusMasterRequest,
    BusMasterResponse, InterruptLine, InterruptSet, TransactionPhase, TransactionRequest,
    TransactionResponse,
};
pub use cache::{
    CacheConfig, CacheStats, DirectMappedCache, ReplacementPolicy, SplitCacheStats, SplitL1Cache,
    StoreAllocationPolicy, WritePolicy,
};
pub use clock::Clock;
pub use component::{CpuCycle, Processor, SimComponent};
pub use machine::Machine;
pub use memory_map::MemoryMap;
