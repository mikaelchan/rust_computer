# Architecture

This project starts with a single-core, cycle-driven von Neumann machine.

- `rvsim-isa`: instruction formats, decode, traps, CSR addresses.
- `rvsim-system`: clock, processor trait, unified bus, memory map, machine wrapper, and configurable cache wrappers.
- `rvsim-cpu`: reference core, pipeline scaffold, execution helpers, hazard and predictor modules.
- `rvsim-devices`: RAM, DRAM, ROM, a minimal UART device, a machine timer MMIO device, a machine software interrupt source, an interrupt controller with claim/complete semantics, and latency wrappers for timing experiments.
- `rvsim-computer`: a tiny executable that wires the pieces together.

The current milestone targets `RV32I + machine mode CSRs + unified memory + machine software/timer/external interrupt delivery`. The first timing model is now in place as a single shared bus with fixed per-device wait states; loads, stores, and instruction fetches retry until the bus becomes ready again. A unified cache or split L1 instruction/data cache can now wrap that bus with configurable line sizes, set associativity, round-robin or LRU replacement, and explicit write policies (`write-through` or `write-back`) plus store allocation policies. Misses refill an entire cache line from the backing bus, dirty write-back evictions push modified lines back through the same bus path, and the current `computer` example now composes split L1 caches over a unified L2. Below the cache hierarchy, a round-robin arbiter can now grant the lower bus to autonomous masters such as the DMA controller, making bus contention observable without changing the CPU-facing bus trait. Cache statistics now expose traffic-oriented counters such as refill words, write-back words, dirty evictions, and bypassed accesses so each cache level's lower-level pressure is observable, while DMA currently targets explicitly non-cacheable buffers until a future coherence or cache-maintenance model is added. The directory layout already leaves space for branch prediction, richer traps, more devices, deeper cache hierarchies, and a later out-of-order core.
