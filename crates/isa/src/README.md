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

## Layering Inside The Crate

- `opcode.rs` defines the semantic categories of instructions once they have been recognized.
- `instruction.rs` packages one decoded instruction together with the fields the microarchitecture needs later.
- `decode.rs` is the translation boundary from raw 32-bit word to `DecodedInstruction`.
- `csr.rs` gives CSR addresses metadata such as privilege level, read-only encoding, and counter visibility rules.
- `exception.rs` is the shared vocabulary for synchronous exceptions, asynchronous interrupts, and the combined `Trap` type.

That layering means the decoder does not need to know about CPU pipelines, and the CPU does not need to remember raw encoding details once decode is complete.

## Decode Contract

- `decode(raw, pc)` either returns a fully classified `DecodedInstruction` or a `DecodeError`.
- The decoder is intentionally strict: unsupported encodings are rejected immediately instead of producing partial instruction objects.
- Immediate extraction and operand-field slicing happen here, so later stages can work with a normalized semantic representation.

This keeps the rest of the machine from depending on bit-twiddling logic scattered through execution code.

## CSR Metadata Contract

- `CsrAddress` is more than an enum of constants.
- It also answers:
  - what minimum privilege level is required
  - whether the CSR is read-only by encoding convention
  - whether it participates in `counteren` gating

That is why CSR privilege checks belong partly here and partly in `rvsim_cpu::state::CsrFile`: this crate describes the encoded contract, while the CPU crate owns the actual stored state and behavior.

## Trap Vocabulary Role

- `Trap::cause_code`, `cause_bits`, and `tval` make the CPU models independent from hard-coded trap-number tables.
- Interrupt and exception classification stays at the ISA boundary, while privilege delegation and trap entry stay in the CPU state layer.

This separation is especially valuable once both reference and pipeline cores need to agree on trap meaning.

## Extension Pattern

- Add new encodings and categories here first.
- Keep this crate descriptive and mostly side-effect free.
- Only put behavior here when it is part of the ISA description itself rather than part of a particular CPU implementation choice.

## Extension Notes

- New architectural features usually start here before they become executable behavior in `rvsim_cpu`.
- Keep this layer descriptive. Architectural timing belongs elsewhere.

## Related Reading

- [repository index](../../../README.md)
- [cpu crate](../../../crates/cpu/src/README.md)
- [execution helpers](../../../crates/cpu/src/exec/README.md)
- [architecture overview](../../../docs/architecture.md)
