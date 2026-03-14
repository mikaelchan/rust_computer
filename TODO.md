# TODO

This file tracks the next engineering steps for `rust_computer`.

The current baseline already has:

- a cycle-driven single-core machine
- reference and pipeline CPU models
- interrupt controller, machine timer, machine software interrupt source, UART
- fixed bus wait states
- unified cache and split L1 instruction/data cache
- configurable cache line size, associativity, and replacement policy

## Next

- [x] Add `write-back` data-cache support with dirty lines.
  Dirty cache lines can now stay resident and write back on eviction instead of forcing every store through to memory immediately.

- [x] Add configurable store allocation policy.
  `CacheConfig` now supports both `write-allocate` and `no-write-allocate`.

- [x] Tighten cacheable-range boundary behavior.
  Cacheable physical ranges are now validated against cache line boundaries so partial-line ranges fail fast during cache construction.

- [x] Add CPU-level integration tests for cache behavior.
  Reference and pipeline cores now have split-L1 integration coverage so front-end fetches and data accesses stay visible as separate paths.

## Memory Hierarchy

- [x] Add an L2 cache wrapper below the L1 cache.
  The cache wrappers are now used in a stacked hierarchy, with split L1 instruction/data caches over a unified L2 in the example machine and regression tests.

- [x] Add a memory-controller / DRAM timing model.
  A simple DRAM device now models row misses, row hits, and sequential burst accesses, and the example machine uses it for main memory.

- [x] Model refill/write-back traffic explicitly.
  Cache stats now expose refill/write-back word counts, dirty evictions, and bypassed accesses so lower-level traffic is visible per cache level.

- [x] Add cache performance counters beyond hits/misses.
  Cache levels now report refill words, write-back words, dirty evictions, bypassed reads/writes, and per-level write-back traffic in addition to hits/misses.

## Bus And Devices

- [x] Add bus arbitration for multiple bus masters.
  A new round-robin arbiter now sits in front of the memory map so autonomous devices can compete with the CPU for lower-bus cycles.

- [x] Add a DMA-capable external device.
  A simple MMIO-programmable DMA engine now issues memory-to-memory transfers through the shared arbiter and can raise a machine external interrupt on completion or fault.

- [ ] Add cache maintenance or coherent DMA behavior.
  The current DMA path intentionally uses non-cacheable buffers; future work should model either software-managed cache maintenance or hardware coherence.

- [ ] Expand MMIO device coverage.
  Good candidates: block device, display/framebuffer, keyboard/input source, programmable timer variants, and storage-oriented test devices.

## CPU And Privilege

- [ ] Extend privilege support beyond machine mode.
  Supervisor mode, trap delegation, and a cleaner privilege model are natural next steps once the memory system is more mature.

- [ ] Add virtual memory support.
  A future MMU/TLB implementation should come after the cache/memory hierarchy is stable enough to study address translation interactions cleanly.

- [ ] Keep the out-of-order path separate until the memory system is ready.
  OOO remains a future milestone, but it should not move ahead of the remaining cache/bus foundations.

## Tooling And Validation

- [ ] Add focused microbenchmarks for cache and memory behavior.
  Examples: conflict misses, line refill cost, write-back pressure, interrupt latency under memory pressure.

- [x] Add a small architecture test program suite.
  Versioned RV32 program images now live under `crates/cpu/tests/programs`, and integration tests run them against both the reference and pipeline cores.

- [ ] Add more user-facing documentation for memory experiments.
  Document recommended machine configurations, cache policies, and expected timing behavior for common research scenarios.
