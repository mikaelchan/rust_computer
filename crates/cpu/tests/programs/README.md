# regression program images

This directory holds versioned architectural regression programs for `rvsim-cpu`.

## Program Image Format

- One 32-bit RV32 machine word per line.
- Hex words use the `0x12345678` form.
- Blank lines are ignored.
- `#` starts a comment until end of line.

The integration test harness parses these files directly so the same program images can be reused across CPU models and future experiment runners.

## Why These Files Are Versioned

- They provide a stable cross-core regression surface that does not depend on one specific unit-test fixture.
- They are small enough to inspect manually when bring-up or interrupt sequencing breaks.
- They can later be reused by the computer application, benchmark runners, or external loaders without inventing a second test-program format.

Current checked-in examples now cover:

- simple load/store behavior
- simple counted control flow
- machine software interrupt delivery
- delegated supervisor software interrupt delivery with `sret` return
- delegated supervisor external interrupt delivery with controller claim/complete
- delegated supervisor external interrupt delivery from block-device completion
- machine `ecall` delivery into a machine handler with `mret` return
- delegated user-ecall delivery into a supervisor handler with `sret` return
- `Sv32` ASID namespace switching
- pagetable remap visibility across `sfence.vma`
- `Sv32` root-leaf superpage data access
- `Sv32` global-mapping survival across ASID-specific `sfence.vma`

## Extension Notes

- Keep programs short and purpose-specific so failures remain easy to localize.
- Prefer one behavioral axis per program, such as load/store ordering, simple control flow, or one interrupt delivery path.
- When a new program captures an architectural rule, add or update a matching integration test in `program_suite.rs`.

## How To Validate

- `cargo test -p rvsim-cpu --test program_suite`
  Replays every checked-in program image against both the reference and pipelined cores.
- `cargo test -p rvsim-cpu --test program_suite reference_core_runs_msip_interrupt_program`
  Narrow reference-core validation for the software-interrupt program image.
- `cargo test -p rvsim-cpu --test program_suite pipeline_core_runs_store_load_program`
  Narrow pipeline-core validation for the basic load/store program image.
- `cargo test -p rvsim-cpu --test program_suite pipeline_core_runs_sv32_asid_switch_program`
  Narrow pipeline-core validation for the `Sv32` ASID namespace program image.

## Related Reading

- [cpu integration tests](../README.md)
- [cpu crate](../../src/README.md)
- [cpu core models](../../src/core/README.md)
- [computer app](../../../../apps/computer/src/README.md)
- [architecture overview](../../../../docs/architecture.md)
