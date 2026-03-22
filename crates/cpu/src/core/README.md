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

## Expected Differences

- Both cores should agree on architecturally committed state.
- They may differ on transient internal timing, such as when an interrupt becomes precise relative to in-flight instructions.
- Tests should encode those timing differences explicitly rather than forcing the pipeline model to mimic the reference core cycle-for-cycle.

## Extension Notes

- Add shared semantics outside this directory when possible.
- Keep core-local logic focused on scheduling, trapping boundaries, and interaction with the bus/MMU over time.
