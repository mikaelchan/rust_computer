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
- `wfi` sleep and wakeup on machine software interrupt
- vectored machine software interrupt entry through `mtvec`
- vectored delegated supervisor external interrupt entry through `stvec`
- machine external interrupt delivery from the controller
- machine interrupt priority choosing external over software and timer
- machine software interrupt delivery raised by a `mip` CSR write
- delegated supervisor software interrupt delivery raised by a `sip` CSR write
- machine timer interrupt delivery from the memory-mapped timer device
- machine software interrupt preemption of a delegated supervisor external handler
- same-mode machine interrupt nesting after software re-enables `mstatus.MIE`
- same-mode supervisor interrupt nesting after software re-enables `sstatus.SIE`
- `Sv32` data-page loads and stores that force hardware `A/D` bit updates
- machine-mode translated loads through supervisor mappings under `mstatus.MPRV`
- selective `sfence.vma` invalidation for one translated virtual address
- machine trapping on an `Sv32` instruction page fault during fetch
- machine trapping on an `Sv32` malformed non-leaf PTE with reserved `A/D/U` shape bits
- `Sv32` root-leaf superpage instruction fetch plus data access through one mapping
- `satp` ASID-namespace reuse that keeps a stale translation until `sfence.vma`
- machine-mode writes and reads of the high-half `mcycleh` CSR
- user `cycleh` access with enabled counter delegation
- user `instret` access with enabled counter delegation
- illegal writes to the read-only `instret` shadow CSR
- `Sv32` translated loads from user-accessible pages
- `Sv32` translated loads from execute-only pages under `MXR`
- delegated supervisor software interrupt delivery with `sret` return
- delegated supervisor external interrupt delivery with controller claim/complete
- delegated supervisor external interrupt delivery from block-device completion
- delegated supervisor external interrupt delivery from DMA completion
- delegated user illegal-instruction delivery into a supervisor handler with `sret` return
- machine `ecall` delivery into a machine handler with `mret` return
- delegated user-ecall delivery into a supervisor handler with `sret` return
- supervisor `satp` access trapped by `mstatus.TVM` with `mret` return
- supervisor `sfence.vma` trapped by `mstatus.TVM` with `mret` return
- supervisor `sret` trapped by `mstatus.TSR` with `mret` return
- supervisor `wfi` trapped by `mstatus.TW` with `mret` return
- user machine-CSR access trapped by machine mode with `mret` return
- user `instret` access trapped by missing `scounteren` with `mret` return
- `Sv32` ASID namespace switching
- `Sv32` combined virtual-address plus ASID selective `sfence.vma`
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
