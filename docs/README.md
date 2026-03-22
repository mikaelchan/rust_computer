# docs index

This directory holds project-level design notes that sit above the code-adjacent module `README.md` files.

## Documents

- [architecture.md](/Users/michael/Workspace/rust_computer/docs/architecture.md)
  High-level snapshot of the simulated machine and the current milestone scope.
- [cpu_plan.md](/Users/michael/Workspace/rust_computer/docs/cpu_plan.md)
  Phased CPU build-out plan from trusted reference core to more advanced implementations.
- [future_ooo.md](/Users/michael/Workspace/rust_computer/docs/future_ooo.md)
  Rationale for keeping a future out-of-order design as a separate track.
- [memory_experiments.md](/Users/michael/Workspace/rust_computer/docs/memory_experiments.md)
  Practical experiment guide for caches, DRAM, DMA, and interrupt-latency studies.
- [memory_map.md](/Users/michael/Workspace/rust_computer/docs/memory_map.md)
  Suggested physical address layout for the current machine.

## When To Read What

- Start with `architecture.md` if you want the shortest top-down overview.
- Read the module `README.md` files under `apps/` and `crates/` when you need implementation-oriented explanations.
- Read `memory_experiments.md` when you want to run or interpret the existing benchmark suite.
- Read `future_ooo.md` before starting any speculative work on a more aggressive core.

## Relationship To Module READMEs

- `docs/` explains the project at the design and experiment level.
- module `README.md` files explain code boundaries, responsibilities, and extension points near the implementation.

The two layers are intended to complement each other rather than duplicate the same material.

## Suggested Reading Paths

- For CPU work:
  [architecture.md](/Users/michael/Workspace/rust_computer/docs/architecture.md) ->
  [crates/cpu/src/README.md](/Users/michael/Workspace/rust_computer/crates/cpu/src/README.md) ->
  [crates/cpu/src/state/README.md](/Users/michael/Workspace/rust_computer/crates/cpu/src/state/README.md) ->
  [crates/cpu/src/core/README.md](/Users/michael/Workspace/rust_computer/crates/cpu/src/core/README.md)
- For memory and IO work:
  [memory_map.md](/Users/michael/Workspace/rust_computer/docs/memory_map.md) ->
  [crates/system/src/README.md](/Users/michael/Workspace/rust_computer/crates/system/src/README.md) ->
  [crates/devices/src/README.md](/Users/michael/Workspace/rust_computer/crates/devices/src/README.md) ->
  [memory_experiments.md](/Users/michael/Workspace/rust_computer/docs/memory_experiments.md)
- For future microarchitecture work:
  [cpu_plan.md](/Users/michael/Workspace/rust_computer/docs/cpu_plan.md) ->
  [crates/cpu/src/pipeline/README.md](/Users/michael/Workspace/rust_computer/crates/cpu/src/pipeline/README.md) ->
  [crates/cpu/src/predictor/README.md](/Users/michael/Workspace/rust_computer/crates/cpu/src/predictor/README.md) ->
  [future_ooo.md](/Users/michael/Workspace/rust_computer/docs/future_ooo.md)
