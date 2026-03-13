# Architecture

This project starts with a single-core, cycle-driven von Neumann machine.

- `rvsim-isa`: instruction formats, decode, traps, CSR addresses.
- `rvsim-system`: clock, processor trait, unified bus, memory map, machine wrapper.
- `rvsim-cpu`: reference core, pipeline scaffold, execution helpers, hazard and predictor modules.
- `rvsim-devices`: RAM, ROM, and a minimal UART device.
- `rvsim-simulator`: a tiny executable that wires the pieces together.

The current milestone targets `RV32I + machine mode CSRs + unified memory`. The directory layout already leaves space for branch prediction, richer traps, more devices, and a later out-of-order core.
