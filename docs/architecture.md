# Architecture

This project starts with a single-core, cycle-driven von Neumann machine.

- `rvsim-isa`: instruction formats, decode, traps, CSR addresses.
- `rvsim-system`: clock, processor trait, unified bus, memory map, machine wrapper.
- `rvsim-cpu`: reference core, pipeline scaffold, execution helpers, hazard and predictor modules.
- `rvsim-devices`: RAM, ROM, a minimal UART device, a machine timer MMIO device, a machine software interrupt source, and an interrupt controller with claim/complete semantics.
- `rvsim-computer`: a tiny executable that wires the pieces together.

The current milestone targets `RV32I + machine mode CSRs + unified memory + machine software/timer/external interrupt delivery`. The directory layout already leaves space for branch prediction, richer traps, more devices, and a later out-of-order core.
