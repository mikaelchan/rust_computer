# isa crate

This crate defines the ISA-level language understood by the CPU models.

## Responsibilities

- Represent decoded instructions and raw instruction words.
- Define opcodes, ALU ops, branch kinds, load/store kinds, system kinds, and CSR addresses.
- Decode 32-bit instruction words into semantic forms.
- Represent exceptions, interrupts, and traps in a core-independent way.

## Files

- `instruction.rs`: decoded instruction representation.
- `opcode.rs`: instruction-kind enumerations.
- `decode.rs`: instruction decoder.
- `csr.rs`: CSR address definitions and metadata.
- `exception.rs`: exception, interrupt, and trap types.

## Why This Crate Is Separate

- The ISA description should not depend on a specific core implementation.
- Tests, assemblers, microbench code, and future tools can all reuse the same decode and trap vocabulary.

## Extension Notes

- New architectural features usually start here before they become executable behavior in `rvsim_cpu`.
- Keep this layer descriptive. Architectural timing belongs elsewhere.

## Related Reading

- [repository index](../../../README.md)
- [cpu crate](../../../crates/cpu/src/README.md)
- [execution helpers](../../../crates/cpu/src/exec/README.md)
- [architecture overview](../../../docs/architecture.md)
