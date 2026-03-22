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

## Extension Notes

- If a new feature changes how requests move, it probably belongs here.
- If it changes what an endpoint does once addressed, it probably belongs in `rvsim_devices`.

## Related Reading

- [repository index](/Users/michael/Workspace/rust_computer/README.md)
- [devices crate](/Users/michael/Workspace/rust_computer/crates/devices/src/README.md)
- [cpu crate](/Users/michael/Workspace/rust_computer/crates/cpu/src/README.md)
- [architecture overview](/Users/michael/Workspace/rust_computer/docs/architecture.md)
- [memory map](/Users/michael/Workspace/rust_computer/docs/memory_map.md)
- [memory experiments guide](/Users/michael/Workspace/rust_computer/docs/memory_experiments.md)
