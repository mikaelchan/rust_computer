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

    Ok(())
}
