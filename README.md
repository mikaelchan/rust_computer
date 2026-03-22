# rust_computer

A Rust-based computer simulation project for studying a von Neumann machine from the CPU outward.

The codebase currently focuses on a cycle-driven `RV32I` machine with:

- a reference core and an in-order pipelined core
- machine mode plus a substantial supervisor-mode privilege slice
- `Sv32` address translation and page walking
- bus timing, explicit transactions and bursts, and a cache hierarchy
- interrupt sources, an interrupt controller, DMA, DRAM, and a RAM-backed block device
- microbenchmarks for cache behavior and interrupt latency under load

## Workspace Layout

- `apps/computer/`
  Application entry points and microbench binaries.
- `crates/cpu/`
  CPU models, execution semantics, privilege state, MMU, tracing, and pipeline support.
- `crates/devices/`
  MMIO device models such as DRAM, DMA, interrupt sources, and storage.
- `crates/isa/`
  ISA definitions, decode, traps, CSR addresses, and instruction vocabulary.
- `crates/system/`
  Bus, memory map, cache hierarchy, arbitration, and machine wrapper infrastructure.
- `docs/`
  Higher-level design notes, plans, and experiment guides.
- `TODO.md`
  Active milestone tracker.

## Code Documentation Map

Directory-backed modules now carry their own `README.md` files:

- [computer app](./apps/computer/src/README.md)
- [app binaries](./apps/computer/src/bin/README.md)
- [cpu crate](./crates/cpu/src/README.md)
- [cpu core models](./crates/cpu/src/core/README.md)
- [execution helpers](./crates/cpu/src/exec/README.md)
- [hazard logic](./crates/cpu/src/hazard/README.md)
- [pipeline stages](./crates/cpu/src/pipeline/README.md)
- [branch prediction](./crates/cpu/src/predictor/README.md)
- [architectural state](./crates/cpu/src/state/README.md)
- [devices crate](./crates/devices/src/README.md)
- [isa crate](./crates/isa/src/README.md)
- [system crate](./crates/system/src/README.md)

Single-file top-level modules are documented from their parent module indexes:

- `crates/cpu/src/mmu.rs` and `crates/cpu/src/trace.rs` are covered in the CPU crate README.
- `apps/computer/src/microbench.rs` is covered in the computer app README.

## Design Documents

The higher-level design and experiment notes live under [`docs/`](./docs):

- [docs index](./docs/README.md)
- [architecture overview](./docs/architecture.md)
- [CPU plan](./docs/cpu_plan.md)
- [future out-of-order track](./docs/future_ooo.md)
- [memory experiments guide](./docs/memory_experiments.md)
- [memory map](./docs/memory_map.md)

## Suggested Reading Paths

- CPU architecture work:
  [architecture overview](./docs/architecture.md) ->
  [cpu crate](./crates/cpu/src/README.md) ->
  [architectural state](./crates/cpu/src/state/README.md) ->
  [cpu core models](./crates/cpu/src/core/README.md) ->
  [pipeline stages](./crates/cpu/src/pipeline/README.md)
- Memory-system work:
  [architecture overview](./docs/architecture.md) ->
  [system crate](./crates/system/src/README.md) ->
  [devices crate](./crates/devices/src/README.md) ->
  [memory experiments guide](./docs/memory_experiments.md)
- Tooling and runnable experiments:
  [computer app](./apps/computer/src/README.md) ->
  [app binaries](./apps/computer/src/bin/README.md) ->
  [docs index](./docs/README.md)

## Typical Commands

- `cargo test`
  Run the full workspace test suite.
- `cargo test -p rvsim-cpu`
  Run CPU-focused regressions.
- `cargo run -p rvsim-computer`
  Run the main example machine.
- `cargo run -p rvsim-computer --bin memory_microbench`
  Run the memory and interrupt microbenchmark suite.

## Current Documentation Convention

- Put module-level knowledge in the nearest directory `README.md`.
- Keep broader architecture and roadmap material in `docs/`.
- Keep milestone tracking in `TODO.md`.

That split keeps code-adjacent explanations near the implementation while preserving a smaller set of higher-level design documents.
