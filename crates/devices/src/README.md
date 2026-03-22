# devices crate

This crate contains memory-mapped device models that plug into the shared bus.

## Device Families

- Memory devices: `ram.rs`, `rom.rs`, `dram.rs`.
- Interrupt sources: `machine_timer.rs`, `machine_software_interrupt.rs`, `supervisor_software_interrupt.rs`.
- System peripherals: `interrupt_controller.rs`, `dma_controller.rs`, `block_device.rs`, `simple_uart.rs`.
- Adapters: `latency_adapter.rs`.

## Common Design Pattern

- Devices expose a register or memory window through the `Addressable` interface from `rvsim_system`.
- Time-dependent devices advance through repeated bus interactions rather than instant side effects.
- Interrupt-capable devices surface their signals through the shared interrupt-line model instead of bespoke callbacks.

## Modeling Philosophy

- The devices are simplified enough to be tractable in tests.
- They are still stateful enough to study ordering, latency, interrupts, DMA completion, and cache-maintenance effects.

## Extension Notes

- New devices should follow the existing MMIO register style and use the shared interrupt lines where possible.
- If a device needs extra latency but no new behavior, prefer wrapping it in `LatencyAdapter` rather than cloning the device implementation.

## Related Reading

- [repository index](../../../README.md)
- [system crate](../../../crates/system/src/README.md)
- [architecture overview](../../../docs/architecture.md)
- [memory map](../../../docs/memory_map.md)
- [memory experiments guide](../../../docs/memory_experiments.md)
