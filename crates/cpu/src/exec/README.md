# execution helpers

This module contains instruction semantics shared by both CPU cores.

## Responsibilities

- Evaluate decoded instructions against `HartState`.
- Perform CSR reads and writes through a common path.
- Drive MMU-assisted loads and stores.
- Convert memory faults and illegal operations into architectural traps.

## Files

- `alu.rs`: integer ALU operations.
- `branch.rs`: branch conditions and target calculation.
- `csr.rs`: CSR instruction read/modify/write behavior.
- `load_store.rs`: effective-address and sign-extension helpers.
- `mod.rs`: `execute_decoded`, the shared semantic entry point.

## Boundary

- This module answers "what does this instruction do?"
- It does not answer "in which cycle does it retire?" or "which pipeline stage owns it right now?"
- Timing-specific concerns belong in `core/`, `pipeline/`, or `hazard/`.

## Execution Contract

- `execute_decoded` consumes a decoded instruction, the committed `HartState`, the shared `PageWalker`, and a bus handle.
- It returns an `ExecutionResult` rather than mutating retirement counters or pipeline stats directly.
- `retired = 0` plus `trap = None` means "no architectural progress yet"; the caller is expected to retry later.
- `trap = Some(...)` means the architectural trap entry has already been applied to `HartState`.

This contract is what lets the same semantic function serve both the reference core and the pipelined core.

## Memory and Stall Semantics

- Loads and stores first compute a virtual address, then ask the page walker for translation.
- Translation can return a physical address, a stall, or a page fault.
- Even after translation succeeds, the backing bus can still return `Busy`, which again means "retry later without inventing new architectural state."
- Memory-side faults are mapped into architectural traps rather than leaking raw bus errors upward, unless the error is outside the modeled architectural fault set.

That split is important: the semantic layer knows how to convert a legal architectural failure into a trap, but transport-level failures that do not map to the ISA still remain real errors.

## CSR Handling Model

- `csr.rs` computes a `CsrOutcome` with a read value and an optional deferred `CsrWrite`.
- This keeps read/modify/write behavior explicit instead of scattering CSR bit logic through the cores.
- In the reference core, that write can be applied immediately after the semantic check succeeds.
- In the pipelined core, the same `CsrWrite` object can ride the pipeline until commit.

The deferred-write shape is one of the main reasons this module composes cleanly with both cores.

## System Instruction Policy

- `sfence.vma`, `wfi`, `mret`, and `sret` are handled here as architectural operations.
- Privilege gating still depends on `HartState` and `CsrFile`, so the execution layer can reject illegal use without duplicating CSR ownership.
- Translation fences are produced as semantic side effects and then consumed by the pipeline timing model where needed.

## Design Rule

- If you are adding new architectural behavior, prefer expressing it here in terms of state transition and explicit result objects.
- If you are adding a new stall source, replay rule, or precise-commit constraint, it probably belongs outside this module.

## Practical Rule

- When adding a new instruction, prefer extending this module first so both core models inherit the same architectural meaning.

## How To Validate

- `cargo test -p rvsim-cpu exec::`
  Covers the direct execution-helper unit tests such as ALU and CSR helper behavior.
- `cargo test -p rvsim-cpu core::reference::tests::executes_csr_read_write_sequence`
  Confirms the reference core consumes the shared execution results correctly.
- `cargo test -p rvsim-cpu core::pipeline::tests::delegates_user_ecall_to_supervisor_handler_and_returns_with_sret`
  Confirms trap-producing system instructions still compose with the pipelined core.

## Related Reading

- [cpu crate](../../../../crates/cpu/src/README.md)
- [isa crate](../../../../crates/isa/src/README.md)
- [architectural state](../../../../crates/cpu/src/state/README.md)
- [cpu core models](../../../../crates/cpu/src/core/README.md)
