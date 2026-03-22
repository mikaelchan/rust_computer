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

## Shared Runtime Structure

Both core models are built from the same underlying pieces:

- `HartState` carries the architectural PC, integer registers, CSRs, privilege mode, and halted bit.
- `execute_decoded` in `exec/` provides shared instruction meaning.
- `PageWalker` in `mmu.rs` handles `Sv32` translation, permission checks, and TLB-like reuse.
- `CsrFile` in `state/` centralizes interrupt pending state, delegation rules, trap entry, and trap return.

That composition is deliberate. When a new architectural feature is added, the default path is:

1. describe it in `rvsim_isa`
2. implement its state impact in `state/` or `exec/`
3. let both cores consume it through their own timing model

## The Two Single-File Top-Level Modules

- `mmu.rs`
  Holds the incremental page walker. Translation can stall across cycles, can update A/D bits through the bus, and can be fenced without forcing that logic into either core implementation.
- `trace.rs`
  Holds the pipelined core's observability surface. `CommitEvent`, `FlushReason`, `PipelineStats`, and `PipelineTrace` are kept outside `core/pipeline.rs` so the timing model can be inspected without entangling tracing with instruction semantics.

## Cross-Module Boundaries

- `core/` should own scheduling and commit timing.
- `exec/` should own semantic effects once an instruction is considered ready to execute.
- `state/` should own architectural state transitions.
- `mmu.rs` should own virtual-to-physical translation behavior and translation-local caching.
- `trace.rs` should own externally visible pipeline introspection.

If one change starts touching all of those at once, it is usually a sign that the design boundary is being crossed in the wrong place.

## Precision Model

- The reference core prioritizes clarity. It retries stalled instructions and drains page walks with as little transient state as possible.
- The pipelined core prioritizes realistic ordering. It can have a front-end PC ahead of the committed architectural PC, in-flight latches, branch prediction state, and explicit flush causes.
- Tests should compare committed behavior first. Timing-sensitive checks should explicitly document where the two models are expected to differ.

## Extension Notes

- Add new ISA behavior in `exec` and decode support in `rvsim_isa` before touching core-specific control flow.
- If a feature changes only timing, prefer keeping semantics in `exec` and localizing timing logic to `pipeline` or `hazard`.
- Privileged behavior should usually be expressed through `state::CsrFile` so both cores inherit the same rules.

## Related Reading

- [repository index](../../../README.md)
- [architecture overview](../../../docs/architecture.md)
- [CPU plan](../../../docs/cpu_plan.md)
- [future out-of-order track](../../../docs/future_ooo.md)
- [cpu core models](../../../crates/cpu/src/core/README.md)
- [architectural state](../../../crates/cpu/src/state/README.md)
