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
