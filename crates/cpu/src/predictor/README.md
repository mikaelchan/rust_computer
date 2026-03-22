# branch prediction

This module defines the branch-prediction surface used by the pipelined core.

## Components

- `BranchPredictor`: shared trait used by the front end.
- `BranchPrediction`: prediction payload containing direction and target.
- `bimodal_2bit.rs`: a simple dynamic predictor with saturating counters.
- `static_predictor.rs`: an always-not-taken baseline.

## Purpose

- Keep predictor policy swappable without rewriting the fetch stage.
- Allow experiments that compare a trivial predictor against a stateful one.

## Interface Contract

- `predict(pc, fallthrough, target)` returns both a direction and the chosen next PC.
- `update(pc, taken)` is deliberately small: the predictor learns only from resolved branch outcome, not from a large pipeline-specific callback surface.

That contract keeps the predictor reusable and prevents front-end policy from leaking deep into the predictor implementation.

## Current Predictors

- `AlwaysNotTaken` is the baseline. It is useful both as a first-boot implementation and as a control case for experiments.
- `BimodalPredictor` is a direct-mapped table of 2-bit saturating counters indexed by `pc >> 2`.

The bimodal predictor is intentionally simple:

- it aliases branches that map to the same table entry
- it learns only local taken/not-taken tendency
- it has no target cache and no global history

That simplicity is useful because it gives visible branch-prediction behavior without forcing the rest of the pipeline into a more complex speculative design too early.

## Update Timing

- Predictions are consumed in fetch.
- Updates happen after the branch outcome is known later in the pipeline.
- Because updates occur after resolution, mispredictions naturally show up as flushes in `PipelineTrace` and `PipelineStats` rather than as hidden predictor-side corrections.

## Extension Pattern

- Keep new predictors behind the same `BranchPredictor` trait.
- If a predictor needs extra metadata, prefer storing it inside the predictor rather than extending stage payloads unless the front end truly needs new externally visible information.
- Only widen the interface when the fetch stage genuinely cannot express the new prediction policy through `pc`, fallthrough, target, and resolved direction.

## Extension Notes

- New predictors should implement `BranchPredictor` and keep update semantics explicit.
- Predictor state should stay local to the predictor implementation rather than leaking into the pipeline core.

## Related Reading

- [pipeline stages](../../../../crates/cpu/src/pipeline/README.md)
- [cpu core models](../../../../crates/cpu/src/core/README.md)
- [CPU plan](../../../../docs/cpu_plan.md)
- [future out-of-order track](../../../../docs/future_ooo.md)
