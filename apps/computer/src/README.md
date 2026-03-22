# computer app

This directory contains the runnable application layer for the project.

## Responsibility

- Assemble ready-to-run experiments on top of the reusable crates.
- Expose small binaries that exercise the simulated computer from a user-facing entry point.
- Keep benchmarking and orchestration logic out of the lower-level CPU, device, ISA, and system crates.

## Files

- `lib.rs`: library surface for the application package.
- `main.rs`: the default `computer` binary entry point.
- `microbench.rs`: memory-system, translation, and interrupt-latency microbench harnesses.
- `bin/`: extra standalone binaries.

## Design Notes

- This layer should prefer composition over reimplementation. It wires together `rvsim_cpu`, `rvsim_devices`, and `rvsim_system`.
- Microbench code lives here because it is experiment-oriented rather than part of the reusable hardware model.
- Future demos, workload runners, and scripted experiments should enter here before they become their own application packages.

## Two Roles In This Directory

- `main.rs` is the full-machine demonstration path. It wires caches, memory, DMA, interrupts, and a CPU into one concrete example.
- `microbench.rs` is the focused experiment harness. It builds small-purpose machines and returns compact reports instead of interactive output.

That distinction is useful because a demo and an experiment runner have different jobs:

- the demo shows the pieces working together end to end
- the microbench harness isolates one subsystem well enough to compare behaviors

## Host-Driven Experiment Style

- This application layer often acts as a host controller outside the simulated CPU.
- It can prefill memory, program MMIO registers, advance the machine, and inspect outcomes from the outside.
- That is why code here may call helper routines that look more like lab automation than like guest software.

This is intentional. The app layer is where the project turns reusable machine parts into reproducible experiments.

## Main Example Structure

- Build a memory map with ROM, main memory, UART, interrupt sources, DMA, and block storage.
- Wrap the memory map in arbitration and cache hierarchy layers.
- Create a CPU and machine wrapper.
- Seed initial CSR state or device state from the host side.
- Step until observable milestones are reached, then print the results.

This makes `main.rs` closer to a system integration notebook than to a minimal CLI.

## Microbenchmark Structure

- Each benchmark builds only the machine pieces needed for one phenomenon.
- Reports are typed structs rather than ad hoc strings, so tests can validate them and binaries can print them.
- The benchmark helpers measure stall cycles by repeatedly advancing the underlying bus or machine until the target action completes.
- The current suite covers cache conflicts, line refills, dirty write-back pressure, interrupt latency, and translation-caching behavior across ASID switches and `sfence.vma`.

This keeps the measurement logic explicit and reusable.

## How To Validate

- `cargo run -p rvsim-computer`
  Runs the end-to-end example machine.
- `cargo run -p rvsim-computer --bin memory_microbench`
  Runs the focused cache and interrupt microbenchmark suite.
- `cargo test -p rvsim-computer`
  Runs application-layer regression tests around the benchmark harness.

## Extension Pattern

- Put reusable experiment helpers in `microbench.rs` or sibling modules.
- Keep binaries thin wrappers around those helpers.
- If a new experiment starts requiring significant configuration or output handling, add a dedicated module before adding another large `main.rs` branch.

## Related Reading

- [repository index](../../../README.md)
- [docs index](../../../docs/README.md)
- [architecture overview](../../../docs/architecture.md)
- [memory experiments guide](../../../docs/memory_experiments.md)
- [app binaries](../../../apps/computer/src/bin/README.md)
