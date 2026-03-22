use rvsim_computer::microbench::run_memory_microbenchmarks;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report = run_memory_microbenchmarks()?;

    println!("memory microbenchmarks");
    println!(
        "conflict_miss: accesses={} hot_stall_cycles={} thrash_stall_cycles={} hot_hits={} hot_misses={} thrash_hits={} thrash_misses={}",
        report.conflict_miss.accesses,
        report.conflict_miss.hot_stall_cycles,
        report.conflict_miss.thrash_stall_cycles,
        report.conflict_miss.hot_stats.read_hits,
        report.conflict_miss.hot_stats.read_misses,
        report.conflict_miss.thrash_stats.read_hits,
        report.conflict_miss.thrash_stats.read_misses
    );
    println!(
        "line_refill_cost: first_line_stall_cycles={} same_line_stall_cycles={} next_line_stall_cycles={} refills={} refill_words={}",
        report.line_refill.first_line_stall_cycles,
        report.line_refill.same_line_stall_cycles,
        report.line_refill.next_line_stall_cycles,
        report.line_refill.stats.refills,
        report.line_refill.stats.refill_words
    );
    println!(
        "write_back_pressure: stores={} stall_cycles={} dirty_evictions={} write_back_words={} evictions={}",
        report.write_back_pressure.stores,
        report.write_back_pressure.stall_cycles,
        report.write_back_pressure.stats.dirty_evictions,
        report.write_back_pressure.stats.write_back_words,
        report.write_back_pressure.stats.evictions
    );
    println!(
        "interrupt_latency: reference(idle={} loaded={}) pipeline(idle={} loaded={})",
        report.interrupt_latency.reference.idle_cycles,
        report.interrupt_latency.reference.loaded_cycles,
        report.interrupt_latency.pipeline.idle_cycles,
        report.interrupt_latency.pipeline.loaded_cycles
    );
    println!(
        "translation_caching: reference(cold={} warm={} asid_switch={} asid_return={} global_switch={} sfence_reload={}) pipeline(cold={} warm={} asid_switch={} asid_return={} global_switch={} sfence_reload={})",
        report.translation_caching.reference.cold_cycles,
        report.translation_caching.reference.warm_cycles,
        report.translation_caching.reference.switched_asid_cycles,
        report.translation_caching.reference.returned_asid_cycles,
        report
            .translation_caching
            .reference
            .global_switched_asid_cycles,
        report.translation_caching.reference.sfence_reload_cycles,
        report.translation_caching.pipeline.cold_cycles,
        report.translation_caching.pipeline.warm_cycles,
        report.translation_caching.pipeline.switched_asid_cycles,
        report.translation_caching.pipeline.returned_asid_cycles,
        report
            .translation_caching
            .pipeline
            .global_switched_asid_cycles,
        report.translation_caching.pipeline.sfence_reload_cycles
    );

    Ok(())
}
