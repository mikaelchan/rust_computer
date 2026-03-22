# hazard logic

This module isolates the pipeline hazard rules.

## Responsibilities

- Detect control hazards that require redirects or flushes.
- Detect data hazards that require stalls or forwarding.
- Detect structural hazards when multiple users contend for shared resources.

## Files

- `control.rs`: branch, trap, and redirect-related hazards.
- `data.rs`: register and result dependency logic.
- `structural.rs`: resource-contention checks.

## Why This Separation Matters

- Hazard policy tends to change faster than instruction semantics.
- Keeping the rules separate makes it easier to tune the pipeline without disturbing the reference core.
- Tests can target hazard classes directly instead of inferring them from larger pipeline failures.

## Three Hazard Classes

- Control hazards answer "is the fetched or decoded path still valid?"
- Data hazards answer "does a consumer need a value that has not become usable yet?"
- Structural hazards answer "can two actions legally use the same machine resource in this cycle?"

Those questions are intentionally kept separate because they often evolve independently.

## Current Policy Shape

- `control.rs` is intentionally small: it currently decides whether predicted and actual next PCs diverge enough to require a flush.
- `data.rs` currently models the simplest RAW dependency rule, especially the load-use style case where forwarding cannot satisfy the consumer in time.
- `structural.rs` currently models the unified-memory fetch-vs-data-access conflict through `StructuralHazardPolicy`.

The key point is that the pipeline control code consumes these helpers as policy inputs. The helpers themselves are not trying to be a monolithic scoreboard.

## Why Keep It Explicit

- A hazard rule is really part of the machine contract, not just an optimization.
- Wrong hazard policy can silently corrupt architectural results even when instruction semantics are correct.
- Small predicates make it easier to prove why the pipeline stalled or flushed in a given test.

## Extension Pattern

- Add the narrowest hazard predicate that captures the new conflict.
- Keep the returned status object small and descriptive.
- Let `core/pipeline.rs` remain the place where multiple hazard signals are combined into one cycle decision.

That keeps this module readable as the pipeline grows.

## Extension Notes

- If a new feature introduces a stall or flush condition, document it here first.
- Prefer small, explicit hazard predicates over one large opaque controller.

## Related Reading

- [cpu crate](../../../../crates/cpu/src/README.md)
- [cpu core models](../../../../crates/cpu/src/core/README.md)
- [pipeline stages](../../../../crates/cpu/src/pipeline/README.md)
- [CPU plan](../../../../docs/cpu_plan.md)
