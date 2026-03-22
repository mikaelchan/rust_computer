# architectural state

This module owns the CPU state that exists at architectural boundaries.

## Files

- `registers.rs`: integer register file with `x0` hard-wired to zero.
- `privilege.rs`: privilege-level enumeration and helpers.
- `csr_file.rs`: machine and supervisor CSR storage plus trap and interrupt state transitions.
- `mod.rs`: `HartState`, which bundles PC, registers, CSRs, privilege mode, and halt state.

## Key Ideas

- `HartState` is the stable contract shared by both core models.
- `CsrFile` is the center of privileged behavior: delegation, enable bits, trap entry, trap return, counter exposure, and pending interrupts.
- Nested trap behavior follows architectural CSR semantics. If software wants same-mode nesting, it must save and restore trap state such as `mepc` or `sepc` together with `mstatus` or `sstatus`.

## State Layers

- `RegisterFile` is the simplest layer: architectural integer state with `x0` forced to zero.
- `PrivilegeMode` describes the active execution mode and feeds both CSR access control and MMU privilege decisions.
- `MachineCsrs` and `SupervisorCsrs` are stored separately inside `CsrFile`, but many supervisor-visible registers are masked views onto machine-owned status state.
- `HartState` packages those pieces with `pc`, reset-vector tracking, and the halted bit.

This layering keeps "raw storage layout" separate from "whole-hart state at an instruction boundary."

## Interrupt and Trap Ownership

- External interrupt lines are synchronized into `CsrFile` through `sync_interrupts`.
- `CsrFile` decides pending priority, delegation, and whether global enable state allows delivery.
- Trap entry and return update `mepc` or `sepc`, `mcause` or `scause`, and the status bits that preserve the prior privilege and interrupt-enable state.

That is why nested interrupt behavior should be tested here first. The cores decide when a trap is sampled, but `CsrFile` decides what entering or leaving that trap means.

## Counter and CSR View Rules

- Cycle and retired-instruction counters are stored as 64-bit machine-owned values and exposed through low and high CSR halves.
- `mcounteren` and `scounteren` gate visibility for lower privilege modes.
- `sstatus`, `sie`, and `sip` are not independent storage blocks; they are masked supervisor views onto a larger machine-owned state model.

This avoids duplicating privileged state while still letting the simulator express the CSR view rules that software actually sees.

## MMU Relationship

- The MMU consumes privilege level and CSR state from this module.
- Features such as `satp`, `MPRV`, `SUM`, `MXR`, and trap-delegation side effects therefore cross the state/MMU boundary directly.
- When virtual-memory behavior seems wrong, the bug is often either in `mmu.rs` translation logic or in the CSR view supplied from here.

## Why This Module Matters

- Most cross-core correctness issues reduce to state transitions here.
- Keeping privilege rules centralized prevents the reference and pipeline cores from drifting apart.

## Extension Notes

- Add new privileged features here before layering them into fetch, execute, or pipeline timing.
- Prefer precise unit tests in `csr_file.rs` whenever trap-state semantics change.

## Related Reading

- [cpu crate](../../../../crates/cpu/src/README.md)
- [execution helpers](../../../../crates/cpu/src/exec/README.md)
- [system crate](../../../../crates/system/src/README.md)
- [architecture overview](../../../../docs/architecture.md)
