# TODO

This file tracks the next engineering steps for `rust_computer`.

The current baseline already has:

- a cycle-driven single-core machine
- reference and pipeline CPU models
- interrupt controller, machine and supervisor software interrupt sources, machine timer, UART
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

- [x] Upgrade the bus timing model from single transactions to beat/burst-aware transfers.
  The memory map now exposes explicit single-outstanding burst requests with beat-level `Accepted`/`InFlight`/`Ready` phases in addition to single-beat transactions.

- [x] Move cache line refill/write-back paths onto burst transfers.
  Cache refill and dirty write-back paths now submit explicit lower-bus bursts, and unified L2 caches can serve burst refills from split L1s without falling back to per-word compatibility accesses.

- [x] Add multi-outstanding transaction support.
  The memory map now accepts overlapping explicit requests, the arbiter allows each master to keep a configurable request window in flight while preserving in-order responses per master, and the DMA engine uses bounded read-ahead to overlap multiple burst requests.

- [x] Add bus arbitration for multiple bus masters.
  A new round-robin arbiter now sits in front of the memory map so autonomous devices can compete with the CPU for lower-bus cycles.

- [x] Add a DMA-capable external device.
  A simple MMIO-programmable DMA engine now issues memory-to-memory transfers through the shared arbiter and can raise a machine external interrupt on completion or fault.

- [x] Add cache maintenance or coherent DMA behavior.
  Cache wrappers now expose software-managed `write_back_range` and `invalidate_range` hooks that recurse through the cache hierarchy, so cached DMA buffers can be synchronized explicitly even though the DMA path is still non-coherent.

- [x] Expand MMIO device coverage.
  `rvsim-devices` now includes a RAM-backed MMIO block device with fixed command latency, a one-block staging window, and optional completion interrupts for storage-oriented experiments. Future display and input devices can still build on the same MMIO patterns.

## CPU And Privilege

- [x] Add the first supervisor-mode privilege slice.
  The CPU now models supervisor trap CSRs, `sret`, synchronous exception delegation through `medeleg`, and CSR privilege checks across both the reference and pipeline cores.

- [x] Add delegated supervisor interrupt delivery.
  The bus and CPU now model supervisor software/timer/external interrupt lines, privilege-aware interrupt arbitration through `mideleg`, `mie/mip`, `sie/sip`, and `mstatus`/`sstatus`, plus end-to-end supervisor software-interrupt handling in both CPU models.

- [ ] Extend privilege support beyond the first supervisor slice.
  Supervisor trap/interrupt state, `satp`-driven translation, machine-mode `MPRV` data accesses with `SUM`/`MXR` coverage, supervisor-routable external interrupt delivery from the interrupt controller, DMA completion path, and block-device completion path, `WFI` sleep/wakeup handling, supervisor gating through `mstatus.TVM`/`mstatus.TSR`/`mstatus.TW`, 64-bit counter exposure through low/high-half `cycle/time/instret` and `mcycle/minstret` CSRs with `mcounteren/scounteren` control, an independent `time/timeh` counter domain instead of aliasing `mcycle`, software-pending CSR injection through `mip/sip` for machine/supervisor software interrupts, delegation and interrupt-enable CSR masking for currently modeled causes, architectural instruction/load/store access-fault traps for direct bus failures and page-table-walk physical faults, and basic cross-mode plus same-mode nested interrupt behavior with software-managed trap-state save/restore are now in place. Additional supervisor-capable devices, fuller privileged CSR semantics, and broader nesting policy experiments remain future work.

- [x] Add the first virtual-memory slice.
  The CPU now performs `Sv32` instruction and data address translation from `satp`, walks page tables across cycles, and raises instruction/load/store page faults through the existing trap machinery in both CPU models.

- [ ] Extend virtual memory beyond the first slice.
  TLBs, selective `sfence.vma`, hardware-managed A/D bits, the first `SUM`/`MXR` supervisor permission controls, `satp`-scoped TLB namespaces, basic global (`PTE.G`) mappings, end-to-end superpage coverage, a user-facing translation-caching microbenchmark for cold/warm loads, ASID switches, global mappings, and `sfence.vma` reloads, plus versioned program-suite coverage for ASID namespace switching, pagetable remap visibility, superpage data access, and global-mapping survival across ASID-specific `sfence.vma` are now in place. Broader ASID experiments and wider page-table studies remain future work.

- [ ] Keep the out-of-order path separate until the memory system is ready.
  OOO remains a future milestone, but it should not move ahead of the remaining cache/bus foundations.

## Tooling And Validation

- [x] Add focused microbenchmarks for cache and memory behavior.
  The `memory_microbench` binary now reports conflict-miss pressure, line refill cost, write-back pressure, interrupt latency under load, and translation-caching behavior across ASID switches and `sfence.vma`, with matching regression tests for the qualitative behavior.

- [x] Add a small architecture test program suite.
  Versioned RV32 program images now live under `crates/cpu/tests/programs`, and integration tests run them against both the reference and pipeline cores.

- [x] Add more user-facing documentation for memory experiments.
  `docs/memory_experiments.md` now documents recommended configurations, cache policies, expected timing behavior, and the `memory_microbench` workflow.

- [ ] Defer the remaining architectural test backlog into smaller follow-on batches.
  The current program suite already covers 90+ shared regressions, so the remaining work can wait while implementation focus moves back to non-test surfaces. Likely future additions are supervisor/user counter-visibility paths beyond the current `cycleh` and `instret` slices, more malformed-page and permission-shape regressions around superpages plus ASID-targeted fences, and narrower privilege/trap precision corners that are still only covered by unit-style core tests.
