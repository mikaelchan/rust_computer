# CPU Plan

This document explains how CPU work is staged now that the first reference and in-order pipelined cores already exist.

## Current Baseline

The CPU project has already completed the original bring-up phases:

- `ReferenceCore` exists as the architectural baseline for new ISA and privilege features.
- `PipelineCore` exists as a five-stage in-order core with explicit latches, hazards, traces, and branch prediction.
- shared execution helpers define most instruction meaning once for both cores
- machine mode plus a substantial supervisor-mode privilege slice are already modeled
- `Sv32` translation, page walking, `sfence.vma`, A/D-bit management, and basic namespace behavior are already present
- machine and supervisor interrupt delivery paths already run end to end through real devices and the interrupt controller

That means this plan is no longer about first boot. It is about finishing the architectural CPU surface and then deciding how much more timing fidelity to add before starting a separate out-of-order core.

## Phase 5: Finish The Privileged Baseline

The next CPU-facing work should tighten the architectural contract before adding more aggressive microarchitecture:

- broaden privileged CSR and exception coverage where behavior is still intentionally partial
- keep extending interrupt, trap, and nested-handler regressions when new privilege behavior is introduced
- expand architectural program coverage so privilege transitions and fault paths are exercised by reusable program images, not only hand-built unit tests

The main rule for this phase is simple: if software-visible privilege behavior changes, the reference core should become correct first and the pipeline core should inherit that behavior second.

## Phase 6: Deepen Virtual Memory Experiments

The current MMU slice is already useful, but there is still room to widen the study surface:

- broaden ASID and global-mapping experiments
- add more page-table and permission-shape regressions beyond the current focused set
- keep translation, fence, and cache interactions observable enough for later CPU and DMA studies

This phase is less about adding features for their own sake and more about making the address-translation model strong enough that later timing work rests on a defensible baseline.

## Phase 7: Refine Timing And Pipeline Fidelity

Once the architectural surface is stable, the in-order CPU can absorb more timing detail without turning into a second architecture project:

- widen pipeline observability when new replay, flush, or arbitration behavior is added
- consider more explicit clock-phase or sub-cycle modeling only where it changes the machine contract in a meaningful way
- keep timing refinements expressed through explicit state machines and traces rather than through implicit host-side scheduling

The project can eventually model finer-grained clock behavior, but only if the added detail improves experiments or exposes real architectural or microarchitectural effects. Signal-level waveforms are not the immediate goal.

## Phase 8: Keep Out-Of-Order As A Separate Track

An out-of-order CPU remains a later milestone, but it should not be layered into the current in-order pipeline gradually.

- keep the in-order pipeline readable and testable as its own machine
- introduce a separate OoO subsystem when the current privilege, memory, and validation baselines are strong enough
- reuse shared decode, trap vocabulary, memory interfaces, and regression programs rather than duplicating architectural meaning

That separation is described in more detail in [future_ooo.md](./future_ooo.md).

## Related Reading

- [architecture.md](./architecture.md)
- [future_ooo.md](./future_ooo.md)
- [../crates/cpu/src/README.md](../crates/cpu/src/README.md)
- [../crates/cpu/src/core/README.md](../crates/cpu/src/core/README.md)
- [../crates/cpu/src/pipeline/README.md](../crates/cpu/src/pipeline/README.md)
- [../TODO.md](../TODO.md)

## Validation Entry Points

- `cargo test -p rvsim-cpu`
  Runs the CPU unit and integration suite across state, MMU, cores, and program images.
- `cargo test -p rvsim-cpu --test program_suite`
  Replays the shared architectural program images against both current core models.
- `cargo run -p rvsim-computer --bin memory_microbench`
  Provides a system-level check for memory timing and interrupt behavior that future CPU changes must not invalidate accidentally.
