# computer app

This directory contains the runnable application layer for the project.

## Responsibility

- Assemble ready-to-run experiments on top of the reusable crates.
- Expose small binaries that exercise the simulated computer from a user-facing entry point.
- Keep benchmarking and orchestration logic out of the lower-level CPU, device, ISA, and system crates.

## Files

- `lib.rs`: library surface for the application package.
- `main.rs`: the default `computer` binary entry point.
- `microbench.rs`: memory-system and interrupt-latency microbench harnesses.
- `bin/`: extra standalone binaries.

## Design Notes

- This layer should prefer composition over reimplementation. It wires together `rvsim_cpu`, `rvsim_devices`, and `rvsim_system`.
- Microbench code lives here because it is experiment-oriented rather than part of the reusable hardware model.
- Future demos, workload runners, and scripted experiments should enter here before they become their own application packages.

## Related Reading

- [repository index](/Users/michael/Workspace/rust_computer/README.md)
- [docs index](/Users/michael/Workspace/rust_computer/docs/README.md)
- [architecture overview](/Users/michael/Workspace/rust_computer/docs/architecture.md)
- [memory experiments guide](/Users/michael/Workspace/rust_computer/docs/memory_experiments.md)
- [app binaries](/Users/michael/Workspace/rust_computer/apps/computer/src/bin/README.md)
