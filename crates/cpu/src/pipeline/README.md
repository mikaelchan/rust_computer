# pipeline stages

This module contains the building blocks of the in-order pipeline.

## Files

- `if_stage.rs`: instruction fetch and branch-prediction handoff.
- `id_stage.rs`: decode-stage wrapper around ISA decode.
- `ex_stage.rs`: execute-stage orchestration and execute-result packaging.
- `mem_stage.rs`: load/store access and memory-event handling.
- `wb_stage.rs`: architectural write-back.
- `latches.rs`: the structures that move state between stages.

## Data Flow

1. `if_stage` fetches raw instruction words and produces the next PC.
2. `id_stage` decodes the raw instruction into semantic form.
3. `ex_stage` performs ALU, branch, CSR, and trap preparation work.
4. `mem_stage` resolves data-memory access and memory-side stalls.
5. `wb_stage` commits register results into architectural state.

## Design Intent

- The stage helpers are intentionally narrow. They make stage-local behavior testable without collapsing the whole pipeline into one function.
- Pipeline latches are treated as first-class state, which is important for precise exceptions, forwarding, and replay.
- Branch prediction and hazard logic live outside this directory so the stages stay focused on per-stage work rather than global policy.

## Related Reading

- [cpu core models](/Users/michael/Workspace/rust_computer/crates/cpu/src/core/README.md)
- [hazard logic](/Users/michael/Workspace/rust_computer/crates/cpu/src/hazard/README.md)
- [branch prediction](/Users/michael/Workspace/rust_computer/crates/cpu/src/predictor/README.md)
- [architectural state](/Users/michael/Workspace/rust_computer/crates/cpu/src/state/README.md)
- [CPU plan](/Users/michael/Workspace/rust_computer/docs/cpu_plan.md)
