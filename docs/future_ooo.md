# Future OoO

This document explains why an out-of-order CPU should remain a separate implementation track instead of being folded gradually into the current in-order pipeline.

## Why Keep It Separate

- The current `PipelineCore` is an in-order machine with explicit stage latches, hazards, and precise commit rules that are still useful on their own.
- Mixing rename, speculative wakeup, and reorder-buffer behavior into that design would blur the distinction between "in-order timing model" and "future OoO experiment."
- The project already has a cleaner differential structure: a reference core for architectural clarity and a pipelined core for in-order timing behavior.

Keeping OoO separate preserves that structure instead of replacing it with one hybrid core that is harder to reason about and harder to validate.

## Shared Infrastructure To Reuse

The future OoO track should still reuse as much of the existing project as possible:

- `rvsim-isa` for decode, trap vocabulary, and CSR metadata
- `rvsim-system` for bus, cache, arbitration, and machine wrapper interfaces
- architectural program images under `crates/cpu/tests/programs`
- privileged-state and trap semantics that already define what committed architectural state should mean

The new work should change scheduling and speculation policy, not redefine ISA meaning or invent a separate memory-system contract.

## Likely Module Boundaries

When this track starts, it should grow as its own subsystem rather than as scattered conditionals inside the in-order pipeline:

- add dedicated `ooo/` modules under `rvsim-cpu`
- keep rename, issue, execute, memory-ordering, and commit structures explicit
- introduce dedicated building blocks such as a register-alias table, physical register file, reorder buffer, reservation stations, and load-store queue

One reasonable end state is a separate `OoOCore` that plugs into the same top-level CPU trait while consuming the same ISA and system crates below it.

## Entry Criteria

The project should not start this track until the current baseline is strong enough:

- privileged behavior should already be documented and regression-tested at the architectural level
- virtual-memory behavior should be stable enough that OoO work is not debugging the MMU and speculation model simultaneously
- cache, DMA, and interrupt experiments should already expose the memory-system behavior the OoO core must tolerate
- the shared program suite should be broad enough to catch architectural drift between core models

This keeps the first OoO experiments focused on reordering, speculation, and memory ordering instead of on unfinished baseline functionality.

## Related Reading

- [architecture.md](./architecture.md)
- [cpu_plan.md](./cpu_plan.md)
- [../crates/cpu/src/README.md](../crates/cpu/src/README.md)
- [../crates/cpu/src/core/README.md](../crates/cpu/src/core/README.md)
- [../crates/cpu/src/pipeline/README.md](../crates/cpu/src/pipeline/README.md)
- [../TODO.md](../TODO.md)

## Validation Entry Points

There is no OoO implementation yet, so the current validation goal is to preserve a trustworthy baseline:

- `cargo test -p rvsim-cpu core::pipeline::tests`
  Validates the current in-order timing model that future OoO work should complement rather than destabilize.
- `cargo test -p rvsim-cpu --test program_suite`
  Validates the shared architectural program-image surface that a future OoO core should also pass.
- `cargo run -p rvsim-computer --bin memory_microbench`
  Captures system-level memory pressure and interrupt behavior that should inform any future OoO experiment design.
