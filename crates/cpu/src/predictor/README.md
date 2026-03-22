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

## Extension Notes

- New predictors should implement `BranchPredictor` and keep update semantics explicit.
- Predictor state should stay local to the predictor implementation rather than leaking into the pipeline core.
