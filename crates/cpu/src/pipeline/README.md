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

## Architectural PC vs Front-End PC

- `front_end_pc` tracks where fetch wants to go next.
- `HartState.pc` tracks the committed architectural PC.
- The two can diverge whenever the front end is ahead of retirement, which is why the pipeline can observe wrong-path fetches, redirects, and late trap precision without corrupting committed state.

This split is one of the most important differences between the pipeline core and the reference core.

## Latch Model

- `IfIdPayload` carries raw instruction bytes plus prediction metadata.
- `IdExPayload` carries decoded instruction form plus source values captured for execution.
- `ExMemPayload` carries execution results, pending CSR writes, translation fences, memory addresses, and the next PC.
- `MemWbPayload` carries the last pre-commit state into write-back.

Those payloads are intentionally narrow snapshots. They make it possible to reason about stalls and replays stage by stage rather than through one large mutable pipeline state blob.

## Stall and Flush Sources

- Fetch can stall on bus timing, page walking, or structural hazard policy.
- Decode can stall on RAW hazards or when older stages cannot accept new work.
- Execute and memory can trigger flushes on branch redirects, traps, returns from trap, and translation barriers such as `sfence.vma`-related flows.

The pipeline is therefore not only a data path. It is also a control system for deciding which in-flight work remains valid.

## Trace and Stats Model

- `PipelineTrace` records one cycle of stage occupancy, commit, trap, flush cause, and qualitative notes.
- `PipelineStats` accumulates long-run counters such as retired instructions, fetch and decode stall cycles, and different flush classes.
- `CommitEvent` captures the architecturally committed update at the end of a cycle, which is the cleanest differential-testing surface against the reference core.

When extending the pipeline, ask whether a new behavior should appear in:

- latch payloads
- trace output
- cumulative stats

If it affects timing or observability, the answer is often yes.

## Design Intent

- The stage helpers are intentionally narrow. They make stage-local behavior testable without collapsing the whole pipeline into one function.
- Pipeline latches are treated as first-class state, which is important for precise exceptions, forwarding, and replay.
- Branch prediction and hazard logic live outside this directory so the stages stay focused on per-stage work rather than global policy.

## Related Reading

- [cpu core models](../../../../crates/cpu/src/core/README.md)
- [hazard logic](../../../../crates/cpu/src/hazard/README.md)
- [branch prediction](../../../../crates/cpu/src/predictor/README.md)
- [architectural state](../../../../crates/cpu/src/state/README.md)
- [CPU plan](../../../../docs/cpu_plan.md)
