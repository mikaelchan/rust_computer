# system crate

This crate provides the shared machine-level plumbing around CPUs and devices.

## Major Pieces

- `bus.rs`: core bus traits, requests, responses, address ranges, and interrupt lines.
- `memory_map.rs`: address decoding and multiplexing across mapped devices.
- `cache.rs`: direct-mapped and split-L1 cache models with refill, write-back, maintenance, and replacement policy support.
- `arbiter.rs`: multi-master bus arbitration for overlapping request sources.
- `component.rs`: simulation component traits and per-cycle CPU summaries.
- `clock.rs`: simple time source.
- `machine.rs`: top-level container that advances a processor against a bus fabric.

## Design Goals

- Separate transport concerns from device behavior.
- Make latency explicit instead of hiding it behind blocking calls.
- Allow CPUs, DMA engines, and other masters to share the same bus vocabulary.

## Mental Model

- `bus` defines the protocol.
- `memory_map` connects protocol traffic to devices.
- `cache` and `arbiter` shape timing and contention.
- `machine` drives the system forward one cycle at a time.

## Protocol Layers

- `Bus` is the CPU-friendly compatibility surface. Callers issue loads, stores, and fetches and may receive `BusError::Busy` until the access completes.
- `TransactionBus` exposes explicit request IDs and phase tracking for single accesses.
- `BurstBus` does the same for multi-beat word bursts, which are used by caches and autonomous masters such as DMA.
- `Addressable` is the device-facing contract. A device implements byte loads and stores, a latency model, optional interrupts, and per-cycle `tick` behavior.

The practical split is:

- CPUs mostly consume `Bus`.
- caches, arbiters, and DMA consume `TransactionBus` and `BurstBus`.
- memory-mapped devices implement `Addressable`.

## Timing Model

- A device advertises latency through `access_latency`.
- `MemoryMap` accepts transactions and bursts, records them as active work, and advances them through explicit `Accepted`, `InFlight`, `Ready`, or failure states.
- The compatibility `Bus` path is layered on top of that machinery, so a simple CPU still sees retry-until-ready semantics while the lower fabric can keep multiple requests in motion.
- `tick` is the global progress boundary. Device internal time, transaction countdowns, and burst beat advancement all move forward there.

This means the crate is not just an address decoder. It is the place where "which byte is addressed" and "when the result becomes visible" are joined into one system model.

## Cache Data Flow

- `DirectMappedCache` and `SplitL1Cache` wrap an inner bus rather than modifying devices directly.
- A hit is resolved locally from cached line state.
- A miss allocates a refill flow that pulls a full line through lower-bus burst reads.
- Dirty victims drain through explicit write-back bursts instead of instant memory updates.
- Maintenance operations such as `write_back_range` and `invalidate_range` recurse through the hierarchy, which is why DMA experiments can stay non-coherent while still being observable and controllable from software.

The cache code is therefore doing three jobs at once:

- geometry and tag lookup
- policy decisions such as replacement and write behavior
- lower-fabric traffic generation for refill, eviction, and maintenance

## Arbitration Model

- `ArbiterBus` sits between the CPU-facing path and one or more autonomous masters.
- The CPU can reserve a cycle and stall masters for that tick.
- Masters submit either single transactions or bursts through the shared lower fabric.
- Responses are delivered back to each master in submission order even when several requests are outstanding.

This matters because DMA is modeled as a real competing master, not as a magical side effect that edits memory behind the system's back.

## Where To Extend

- Add a new transfer style or phase-visible protocol in `bus.rs`.
- Add new mapping or fabric behavior in `memory_map.rs`.
- Add hierarchy, refill, write-back, or coherence experiments in `cache.rs`.
- Add new shared-master scheduling rules in `arbiter.rs`.
- Keep `machine.rs` thin. It should orchestrate components, not absorb transport policy.

## Extension Notes

- If a new feature changes how requests move, it probably belongs here.
- If it changes what an endpoint does once addressed, it probably belongs in `rvsim_devices`.

## Related Reading

- [repository index](../../../README.md)
- [devices crate](../../../crates/devices/src/README.md)
- [cpu crate](../../../crates/cpu/src/README.md)
- [architecture overview](../../../docs/architecture.md)
- [memory map](../../../docs/memory_map.md)
- [memory experiments guide](../../../docs/memory_experiments.md)
