# cpu integration tests

This directory holds integration-oriented CPU regressions that execute the public CPU models against small whole-machine fixtures.

## Files

- `program_suite.rs`: boots both core models with the same versioned program images and checks shared architectural outcomes.
- `programs/`: hex-encoded regression programs used by `program_suite.rs`.

## Why This Directory Exists

- Unit tests inside `src/` are good for narrow semantic boundaries such as CSR masking, MMU permission checks, or pipeline replay rules.
- Integration tests here validate the exported CPU models as complete processors connected to a machine wrapper and memory map.
- Keeping this layer separate makes it easier to add more reusable architectural programs without bloating one source file inside `src/`.

## Current Test Strategy

- The integration harness builds a minimal machine with ROM, RAM, and a machine software interrupt source.
- Each program image is executed on both `ReferenceCore` and `PipelineCore`.
- Some programs also use host-side setup to preload page tables, CSR state, or register inputs before execution starts.
- Stop conditions check architectural state such as registers, PC, or trap CSRs rather than insisting on identical cycle timing.

That split is intentional. Unit tests prove local invariants; integration tests prove that those invariants still compose into the same visible machine behavior.

## Extension Pattern

- Add new architectural program images under `programs/` when a behavior should be shared across core models.
- Keep host-side fixture setup in `program_suite.rs` small and explicit so each program remains easy to reason about.
- Prefer adding one focused behavior per program before building larger end-to-end workloads.

## How To Validate

- `cargo test -p rvsim-cpu --test program_suite`
  Runs the current cross-core architectural program suite.
- `cargo test -p rvsim-cpu --test program_suite reference_core_runs_msip_interrupt_program`
  Narrows validation to the reference core's software-interrupt program path.
- `cargo test -p rvsim-cpu --test program_suite pipeline_core_runs_count_loop_program`
  Narrows validation to the pipelined core's simple control-flow program path.
- `cargo test -p rvsim-cpu --test program_suite reference_core_runs_sv32_sfence_remap_program`
  Narrows validation to the reference core's pagetable-remap plus `sfence.vma` program path.

## Related Reading

- [cpu crate](../src/README.md)
- [cpu core models](../src/core/README.md)
- [regression program images](./programs/README.md)
- [architecture overview](../../../docs/architecture.md)
