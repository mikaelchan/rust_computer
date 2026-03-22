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

## Practical Rule

- When adding a new instruction, prefer extending this module first so both core models inherit the same architectural meaning.
