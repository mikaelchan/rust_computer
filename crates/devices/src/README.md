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

## Execution Model

- Every device is primarily an `Addressable` MMIO endpoint.
- Some devices are purely reactive register blocks.
- Some devices also consume time through `tick`, such as the block device and machine timer.
- DMA is the notable hybrid: it is both an MMIO device and a `BusMaster`, so software starts it through registers and the transfer then competes for the shared bus like a real master.

That split is intentional. It keeps software-visible control separate from data movement through the memory system.

## Interrupt Pattern

- Devices raise interrupts through the shared `InterruptLine` model from `rvsim_system`.
- Several peripherals expose a route register so completion can target either machine external or supervisor external delivery.
- Software-visible `done` and `error` bits are usually latched until software clears them, which makes completion handling easy to test and reason about.

## Key Device Flows

- `dram.rs`
  Models an open-row memory. The first access to a row pays row-miss latency, subsequent accesses to the same row can pay row-hit latency, and sequential bursts can get the cheapest burst latency.
- `dma_controller.rs`
  Starts from MMIO registers, then issues bounded read-ahead bursts and queued write bursts as an autonomous master. This is the main path for studying bus contention and cache-maintenance effects.
- `interrupt_controller.rs`
  Implements a small claim/complete controller with 32 software-visible sources and a simple lowest-source-ID priority rule. Pending, enabled, claimed, and completed states are kept distinct.
- `block_device.rs`
  Exposes a one-block staging window. Software fills or drains that window, then launches a fixed-latency read or write command. This keeps the storage model small while still forcing explicit command and completion handling.
- `machine_timer.rs`, `machine_software_interrupt.rs`, `supervisor_software_interrupt.rs`
  Provide the basic asynchronous interrupt sources needed for privilege and trap experiments.
- `latency_adapter.rs`
  Wraps an existing device when you want extra delay without inventing a whole new peripheral.

## Design Boundaries

- Device-local register semantics stay here.
- Shared transport, arbitration, cache behavior, and interrupt aggregation stay in `rvsim_system`.
- CPU-visible trap and privilege interpretation stay in `rvsim_cpu`.

That separation keeps devices small and composable. A peripheral should describe "what state does this MMIO block implement?" rather than "how does the whole machine react to it?"

## Extension Pattern

- Start with a compact register map and explicit status bits.
- Use `tick` for time-based progression instead of hidden background mutation.
- Use route bits and shared interrupt lines instead of device-specific CPU hooks.
- If the device originates memory traffic, implement `BusMaster` rather than bypassing the fabric.

## How To Validate

- `cargo test -p rvsim-devices`
  Runs device-local regressions for timers, interrupt routing, DMA, DRAM, and storage.
- `cargo test`
  Recommended when a device change also affects CPU-visible interrupt or memory behavior.

## Extension Notes

- New devices should follow the existing MMIO register style and use the shared interrupt lines where possible.
- If a device needs extra latency but no new behavior, prefer wrapping it in `LatencyAdapter` rather than cloning the device implementation.

## Related Reading

- [repository index](../../../README.md)
- [system crate](../../../crates/system/src/README.md)
- [architecture overview](../../../docs/architecture.md)
- [memory map](../../../docs/memory_map.md)
- [memory experiments guide](../../../docs/memory_experiments.md)
