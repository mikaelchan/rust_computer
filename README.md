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

- [computer app](/Users/michael/Workspace/rust_computer/apps/computer/src/README.md)
- [app binaries](/Users/michael/Workspace/rust_computer/apps/computer/src/bin/README.md)
- [cpu crate](/Users/michael/Workspace/rust_computer/crates/cpu/src/README.md)
- [cpu core models](/Users/michael/Workspace/rust_computer/crates/cpu/src/core/README.md)
- [execution helpers](/Users/michael/Workspace/rust_computer/crates/cpu/src/exec/README.md)
- [hazard logic](/Users/michael/Workspace/rust_computer/crates/cpu/src/hazard/README.md)
- [pipeline stages](/Users/michael/Workspace/rust_computer/crates/cpu/src/pipeline/README.md)
- [branch prediction](/Users/michael/Workspace/rust_computer/crates/cpu/src/predictor/README.md)
- [architectural state](/Users/michael/Workspace/rust_computer/crates/cpu/src/state/README.md)
- [devices crate](/Users/michael/Workspace/rust_computer/crates/devices/src/README.md)
- [isa crate](/Users/michael/Workspace/rust_computer/crates/isa/src/README.md)
- [system crate](/Users/michael/Workspace/rust_computer/crates/system/src/README.md)

Single-file top-level modules are documented from their parent module indexes:

- `crates/cpu/src/mmu.rs` and `crates/cpu/src/trace.rs` are covered in the CPU crate README.
- `apps/computer/src/microbench.rs` is covered in the computer app README.

## Design Documents

The higher-level design and experiment notes live under [`docs/`](/Users/michael/Workspace/rust_computer/docs):

- [docs index](/Users/michael/Workspace/rust_computer/docs/README.md)
- [architecture overview](/Users/michael/Workspace/rust_computer/docs/architecture.md)
- [CPU plan](/Users/michael/Workspace/rust_computer/docs/cpu_plan.md)
- [future out-of-order track](/Users/michael/Workspace/rust_computer/docs/future_ooo.md)
- [memory experiments guide](/Users/michael/Workspace/rust_computer/docs/memory_experiments.md)
- [memory map](/Users/michael/Workspace/rust_computer/docs/memory_map.md)

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
