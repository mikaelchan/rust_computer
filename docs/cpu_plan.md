# CPU Plan

## Phase 1

- Build a trusted `ReferenceCore` that executes one instruction at a time.
- Model architectural state explicitly: `pc`, integer registers, machine CSRs, privilege mode.
- Support enough RV32I instructions to bootstrap tiny programs and regression tests.

## Phase 2

- Expand the `PipelineCore` from a scaffold into a real 5-stage in-order pipeline.
- Add forwarding, load-use stalls, branch flushes, and unified-memory structural hazards.
- Start recording cycle traces for each stage.

## Phase 3

- Replace the static predictor with a 2-bit bimodal table.
- Add trap-vector behavior and more complete CSR/system instruction support.
- Introduce machine software, timer, and external interrupt sources, then route reusable external sources through an interrupt controller.

## Phase 4

- Add cache and bus timing.
- Prepare interfaces for LSQ, ROB, RAT, and reservation stations.
- Introduce a separate out-of-order core instead of overloading the in-order design.
