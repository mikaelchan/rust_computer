# Memory Experiments

This guide summarizes the current memory-system experiments that are already supported in-tree.

The goal is not cycle accuracy against a real SoC. The goal is controlled, repeatable comparisons inside one simulator configuration, so cache, DRAM, DMA, and interrupt interactions stay easy to study.

## Baseline

The current baseline is:

- split L1 instruction/data caches over a unified L2 in the `computer` example
- configurable cache line size, associativity, replacement policy, write policy, and store-allocation policy
- DRAM timing with row-miss, row-hit, and sequential-burst behavior
- a single shared lower bus with round-robin arbitration for autonomous masters such as DMA
- explicit cache traffic counters for refills, dirty evictions, write-backs, and bypassed accesses
- a translation microbenchmark path for TLB warm hits, ASID namespace switches, global mappings, and `sfence.vma` reload cost

Two built-in entry points are useful for memory work:

- `cargo run -p rvsim-computer`
  Runs the main example machine with split L1, unified L2, DRAM, interrupts, and DMA.
- `cargo run -p rvsim-computer --bin memory_microbench`
  Runs focused memory microbenchmarks and prints compact per-scenario summaries.

For architecture-level regressions, keep using:

- `cargo test -p rvsim-cpu --test program_suite`

## Recommended Experiments

### Conflict Misses

Use this when you want to study set conflicts and associativity pressure.

Recommended shape:

- keep the cache small
- use direct-mapped or very low associativity
- place two hot addresses in different lines that map to the same set
- keep backing memory latency visible so misses are easy to distinguish from hits

The current `memory_microbench` conflict-miss case does exactly that. It compares repeated access to one hot line against alternating accesses between two conflicting lines.

Expected behavior:

- the alternating pattern should show more `read_misses`
- total stall cycles should rise sharply versus the single-hot-line pattern
- `read_hits` may still remain non-zero because a refill immediately serves the triggering access

Current sample output:

```text
conflict_miss: accesses=4 hot_stall_cycles=24 thrash_stall_cycles=96 hot_hits=4 hot_misses=1 thrash_hits=4 thrash_misses=4
```

Interpretation:

- the hot case pays one miss, then stays resident
- the thrashing case keeps evicting and refilling, so all four accesses pay miss latency

### Line Refill Cost

Use this when you want to separate refill latency from same-line hits and from later misses to other lines.

Recommended shape:

- use DRAM as the backing store
- make the cache line wider than one word
- read one word from a cold line, then another word in the same line, then a word from a different line

The built-in benchmark uses a 16-byte cache line over DRAM.

Expected behavior:

- the first access to a cold line should incur refill latency
- another access inside the same line should be effectively free at the cache level
- an access to the next line should miss again, but may be cheaper than the first miss if the DRAM model benefits from an open row

Current sample output:

```text
line_refill_cost: first_line_stall_cycles=9 same_line_stall_cycles=0 next_line_stall_cycles=4 refills=2 refill_words=8
```

Interpretation:

- `same_line_stall_cycles=0` confirms the line refill brought in neighboring words
- the second miss is still visible, but smaller because the DRAM row stays favorable

### Write-Back Pressure

Use this when you want to study dirty eviction traffic rather than pure read miss behavior.

Recommended shape:

- use `write-back` plus `write-allocate`
- keep the cache very small
- write to multiple lines that alias onto the same cache set
- make backing memory latency non-zero so write-back traffic is visible as stalls

The current benchmark uses a one-line cache and four stores to four distinct lines.

Expected behavior:

- `dirty_evictions` should rise once the working set exceeds cache capacity
- `write_back_words` should track how much modified data was pushed to lower memory
- total store stall cycles should grow with each extra eviction

Current sample output:

```text
write_back_pressure: stores=4 stall_cycles=112 dirty_evictions=3 write_back_words=12 evictions=3
```

Interpretation:

- after the first resident line, each new line forces an eviction
- each dirty victim writes back a full 16-byte line, which is why `write_back_words=12`

### Interrupt Latency Under Memory Pressure

Use this when you want to see how memory stalls delay asynchronous event handling.

Recommended shape:

- compare an idle loop against a loop that repeatedly loads from DRAM
- inject a machine software interrupt at a known point
- measure cycles from interrupt assertion until the handler visibly updates architectural state
- run the same scenario on both the reference core and the pipeline core

The built-in benchmark reports both cores.

Expected behavior:

- loaded latency should be higher than idle latency
- the pipeline core should usually have a higher absolute latency than the reference core because it has more in-flight state to drain or flush

Current sample output:

```text
interrupt_latency: reference(idle=2 loaded=8) pipeline(idle=6 loaded=10)
```

Interpretation:

- memory pressure delays when the interrupt can be observed precisely
- the pipeline incurs extra control overhead even in the idle case

### Translation Caching And ASID Switching

Use this when you want to study how much page-table walking still leaks through after the current TLB, namespace, and fence logic.

Recommended shape:

- keep instruction fetch physical so the benchmark isolates data translation rather than front-end fetch effects
- preload two address spaces that map the same virtual address differently
- compare a cold translated load, a warm translated load, an ASID switch, a return to a previously cached ASID, and a full `sfence.vma` reload
- run a second variant with `PTE.G` set so the same mapping can be reused across address spaces

The built-in benchmark uses machine-mode `MPRV` loads so instruction fetch remains simple while the data path still exercises `Sv32`.

Expected behavior:

- a cold translated load should cost more than a warm translated load
- switching to a different non-global ASID should cost more than returning to a previously cached ASID
- switching across a global mapping should be closer to the warm case than to the non-global ASID-switch case
- a full `sfence.vma` should force the next translated load back onto the page-walk path

Current sample output:

```text
translation_caching: reference(cold=14 warm=6 asid_switch=15 asid_return=7 global_switch=7 sfence_reload=15) pipeline(cold=18 warm=7 asid_switch=19 asid_return=10 global_switch=11 sfence_reload=18)
```

Interpretation:

- the warm path shows the current TLB is serving repeated loads without rewalking
- returning to the original ASID stays cheaper than first entering a new non-global namespace
- global mappings remain reusable across ASIDs
- `sfence.vma` restores the page-walk cost on the next translated access

## Cache Policy Recommendations

Use these policy combinations depending on what you want to learn:

- capacity and conflict studies:
  prefer `write-through` or read-only loads first, so dirty eviction traffic does not dominate the picture
- dirty data traffic studies:
  use `write-back + write-allocate`
- MMIO or DMA-safe regions:
  either keep them uncached or pair cached buffers with explicit cache maintenance
- sequential locality studies:
  increase line size and keep addresses within one DRAM row
- associativity studies:
  hold everything else fixed and vary only associativity or replacement policy

## Current Limits

A few important caveats matter for experiments:

- DMA is not coherent with caches yet.
  If you place DMA buffers in cached regions, issue `write_back_range` before DMA reads them and `invalidate_range` before the CPU consumes DMA-written data.
- Cacheable ranges must align to cache-line boundaries.
  Partial-line cacheable windows are rejected at construction time.
- The bus is still single-progress at the lower level.
  Arbitration decides who advances next, but only one lower-level access makes progress at a time.
- Cycle counts are configuration-dependent.
  Treat them as relative comparisons inside one setup, not absolute hardware claims.

## Practical Workflow

A good default workflow for new memory experiments is:

1. Run `cargo run -p rvsim-computer --bin memory_microbench` and record the baseline output.
2. Change one variable at a time in [microbench.rs](../apps/computer/src/microbench.rs): line size, associativity, write policy, store-allocation policy, DRAM timing, or translation scenario setup.
3. Re-run the benchmark and compare both stall cycles and cache traffic counters.
4. If the change is intended to preserve behavior, run `cargo test` and `cargo test -p rvsim-cpu --test program_suite`.
5. If the change touches DMA-visible memory, verify that the region is either uncached or paired with explicit cache maintenance, and document that assumption.

## Related Reading

- [architecture.md](./architecture.md)
- [memory_map.md](./memory_map.md)
- [../crates/system/src/README.md](../crates/system/src/README.md)
- [../crates/devices/src/README.md](../crates/devices/src/README.md)
- [../apps/computer/src/README.md](../apps/computer/src/README.md)
- [../apps/computer/src/bin/README.md](../apps/computer/src/bin/README.md)

## Validation Entry Points

- `cargo run -p rvsim-computer --bin memory_microbench`
  Primary entry point for the experiments described here.
- `cargo test -p rvsim-system`
  Useful when changing cache, burst, arbitration, or memory-map behavior.
- `cargo test -p rvsim-devices`
  Useful when changing DRAM, DMA, or interrupt-capable devices.
- `cargo test -p rvsim-cpu --test program_suite`
  Useful when a memory-system change can alter guest-visible CPU behavior.
