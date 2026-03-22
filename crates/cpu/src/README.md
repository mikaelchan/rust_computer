# cpu crate

This crate contains the architectural and microarchitectural CPU model.

## Major Responsibilities

- Provide two executable core models: a reference core and a pipelined core.
- Share instruction semantics, privilege state, hazard logic, branch prediction, tracing, and MMU support across those cores.
- Model architecturally visible behavior closely enough for privilege, interrupt, and memory-system experiments.

## Module Map

- `core/`: top-level core implementations and the common `CpuModel` trait.
- `exec/`: decoded instruction execution helpers shared by both cores.
- `hazard/`: control, data, and structural hazard logic for the pipeline path.
- `pipeline/`: per-stage helpers and latch definitions for the in-order pipeline.
- `predictor/`: branch predictor abstraction and implementations.
- `state/`: hart architectural state, including registers, CSRs, and privilege level.
- `trace.rs`: commit and flush tracing for the pipelined core.
- `mmu.rs`: Sv32 translation, page walking, and TLB-like caching.

## Mental Model

- `state` defines what is architecturally visible.
- `exec` defines what an instruction means.
- `core` decides when that meaning is applied.
- `pipeline`, `hazard`, and `predictor` refine timing and speculation for the pipelined core.
- `mmu` and `trace` add system-facing realism around address translation and observability.

## Extension Notes

- Add new ISA behavior in `exec` and decode support in `rvsim_isa` before touching core-specific control flow.
- If a feature changes only timing, prefer keeping semantics in `exec` and localizing timing logic to `pipeline` or `hazard`.
- Privileged behavior should usually be expressed through `state::CsrFile` so both cores inherit the same rules.

## Related Reading

- [repository index](/Users/michael/Workspace/rust_computer/README.md)
- [architecture overview](/Users/michael/Workspace/rust_computer/docs/architecture.md)
- [CPU plan](/Users/michael/Workspace/rust_computer/docs/cpu_plan.md)
- [future out-of-order track](/Users/michael/Workspace/rust_computer/docs/future_ooo.md)
- [cpu core models](/Users/michael/Workspace/rust_computer/crates/cpu/src/core/README.md)
- [architectural state](/Users/michael/Workspace/rust_computer/crates/cpu/src/state/README.md)
