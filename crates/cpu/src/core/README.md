# cpu core models

This module owns the top-level CPU implementations.

## Files

- `reference.rs`: a simple instruction-at-a-time model used as the architectural baseline.
- `pipeline.rs`: a staged in-order pipeline with hazards, tracing, branch prediction, and precise trap behavior.
- `trait.rs`: the common `CpuModel` interface and shared error type.

## Why Two Cores Exist

- The reference core is easier to reason about when adding new architectural features.
- The pipeline core exposes timing-sensitive behavior such as stalls, flushes, forwarding, and interrupt commit boundaries.
- Keeping both cores alive gives the project a built-in differential check: one model for clarity, one for realism.

## Reference Core Execution Style

- Fetch, decode, execute, and retire are conceptually collapsed into one architectural step.
- A `pending_decoded` slot keeps one instruction alive across bus stalls, so a stalled fetch or memory access can retry without re-decoding unrelated state.
- Interrupts are sampled before a fresh fetch when the bus is not busy, which keeps the model easy to reason about while still honoring visible timing constraints from memory and page walking.

This model is the first place to land new semantics because it is easier to prove correctness when there are fewer in-flight states.

## Pipeline Core Execution Style

- The pipeline core holds explicit stage latches, a separate `front_end_pc`, branch prediction state, translation-barrier state, trace output, and cumulative stats.
- Architectural retirement happens from write-back, not from fetch or execute.
- Flush causes are classified, so branch redirects, traps, trap returns, and translation barriers can be distinguished in both traces and tests.

The key design point is that this core models "when is an effect committed?" rather than only "what is the effect?"

## Commit Boundary Rules

- Register writes, CSR writes, retired-instruction counters, and visible next-PC updates are only architectural once the core reaches its commit point.
- Interrupt timing is therefore precise relative to committed state rather than relative to speculative front-end fetch.
- This is why some nested-interrupt or precise-trap tests intentionally expect different intermediate `epc` values between the reference core and the pipelined core.

## Where Core Logic Stops

- Trap state bits such as `MPP`, `SPP`, `MPIE`, and `SPIE` are owned by `state::CsrFile`.
- Instruction meaning is owned by `exec/`.
- Page-table traversal is owned by `mmu.rs`.

The core implementations should orchestrate those subsystems, not duplicate their logic.

## Expected Differences

- Both cores should agree on architecturally committed state.
- They may differ on transient internal timing, such as when an interrupt becomes precise relative to in-flight instructions.
- Tests should encode those timing differences explicitly rather than forcing the pipeline model to mimic the reference core cycle-for-cycle.

## Extension Notes

- Add shared semantics outside this directory when possible.
- Keep core-local logic focused on scheduling, trapping boundaries, and interaction with the bus/MMU over time.

## How To Validate

- `cargo test -p rvsim-cpu core::reference::tests`
  Exercises the architectural baseline core, including privilege, MMU, and interrupt flows.
- `cargo test -p rvsim-cpu core::pipeline::tests`
  Exercises the in-order pipelined core, including stalls, flushes, prediction, and precise interrupts.
- `cargo test -p rvsim-cpu --test program_suite`
  Replays the shared hex-program regressions against both core models.

## Related Reading

- [cpu crate](../../../../crates/cpu/src/README.md)
- [pipeline stages](../../../../crates/cpu/src/pipeline/README.md)
- [hazard logic](../../../../crates/cpu/src/hazard/README.md)
- [architectural state](../../../../crates/cpu/src/state/README.md)
- [CPU plan](../../../../docs/cpu_plan.md)
