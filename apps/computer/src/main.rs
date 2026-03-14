use std::{cell::RefCell, rc::Rc};

use rvsim_cpu::{CpuModel, ReferenceCore};
use rvsim_devices::{
    DmaController, Dram, DramConfig, InterruptController, MachineSoftwareInterrupt, Rom, SimpleUart,
};
use rvsim_isa::CsrAddress;
use rvsim_system::{
    AddressRange, ArbiterBus, Bus, BusError, CacheConfig, CpuCycle, DirectMappedCache, Machine,
    MemoryMap, Processor, ReplacementPolicy, SplitL1Cache, StoreAllocationPolicy, WritePolicy,
};

const RESET_VECTOR: u32 = 0x0000_0000;
const RAM_BASE: u64 = 0x1000_0000;
const CACHED_RAM_BYTES: u64 = 0x0800;
const UART_BASE: u64 = 0x2000_0000;
const INTERRUPT_CONTROLLER_BASE: u64 = 0x4000_0000;
const MSIP_BASE: u64 = 0x5000_0000;
const DMA_BASE: u64 = 0x6000_0000;
const DMA_SOURCE: u64 = RAM_BASE + 0x0800;
const DMA_DESTINATION: u64 = RAM_BASE + 0x0840;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = [
        0x0050_0093, // addi x1, x0, 5
        0x00a0_0113, // addi x2, x0, 10
        0x0020_81b3, // add x3, x1, x2
        0x1000_0237, // lui x4, 0x10000
        0x0032_2023, // sw x3, 0(x4)
        0x0002_2283, // lw x5, 0(x4)
        0x0000_006f, // jal x0, 0
        0,
        0x0013_0313, // addi x6, x6, 1
        0x3020_0073, // mret
        0,
        0,
        0,
        0,
        0,
        0,
    ];

    let dma = Rc::new(RefCell::new(DmaController::new(DMA_BASE)));
    let mut memory = MemoryMap::new();
    memory.map_device(Rom::from_words(RESET_VECTOR as u64, &program))?;
    memory.map_device(Dram::new(RAM_BASE, 0x1000, DramConfig::new(64, 6, 2, 1)))?;
    memory.map_device(SimpleUart::new(UART_BASE))?;
    memory.map_device(InterruptController::new(INTERRUPT_CONTROLLER_BASE))?;
    memory.map_device(MachineSoftwareInterrupt::new(MSIP_BASE))?;
    memory.map_shared_device(Rc::clone(&dma))?;

    let mut fabric = ArbiterBus::new(memory);
    fabric.add_shared_master(Rc::clone(&dma));

    let l2 = DirectMappedCache::new(
        fabric,
        CacheConfig::new(
            128,
            vec![
                AddressRange::new(RESET_VECTOR as u64, 0x1000),
                AddressRange::new(RAM_BASE, CACHED_RAM_BYTES),
            ],
        )
        .with_line_size(32)
        .with_associativity(4)
        .with_replacement_policy(ReplacementPolicy::LeastRecentlyUsed)
        .with_write_policy(WritePolicy::WriteBack)
        .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
    );

    let cache = SplitL1Cache::new(
        l2,
        CacheConfig::new(64, vec![AddressRange::new(RESET_VECTOR as u64, 0x1000)])
            .with_line_size(16)
            .with_associativity(2)
            .with_replacement_policy(ReplacementPolicy::LeastRecentlyUsed),
        CacheConfig::new(64, vec![AddressRange::new(RAM_BASE, CACHED_RAM_BYTES)])
            .with_line_size(16)
            .with_associativity(2)
            .with_replacement_policy(ReplacementPolicy::LeastRecentlyUsed)
            .with_write_policy(WritePolicy::WriteBack)
            .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
    );

    let cpu = ReferenceCore::new(RESET_VECTOR);
    let mut machine = Machine::new(cpu, cache);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, 1 << 3);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mie, (1 << 3) | (1 << 11));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(5) == 15
    })?;

    host_store32(&mut machine, INTERRUPT_CONTROLLER_BASE + 4, 1)?;
    host_store32(&mut machine, INTERRUPT_CONTROLLER_BASE + 8, 1)?;

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(6) >= 1 && machine.cpu().hart_state().pc == 0x18
    })?;

    let external_mcause = machine.cpu().hart_state().csrs.read(CsrAddress::Mcause);
    let claimed_source = host_load32(&mut machine, INTERRUPT_CONTROLLER_BASE + 12)?;
    host_store32(&mut machine, INTERRUPT_CONTROLLER_BASE + 12, claimed_source)?;

    host_store32(&mut machine, MSIP_BASE, 1)?;

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(6) >= 2 && machine.cpu().hart_state().pc == 0x18
    })?;

    let software_mcause = machine.cpu().hart_state().csrs.read(CsrAddress::Mcause);
    host_store32(&mut machine, MSIP_BASE, 0)?;

    host_store32(&mut machine, DMA_SOURCE, 0xcafe_babe)?;
    host_store32(&mut machine, DMA_SOURCE + 4, 0x0bad_f00d)?;
    host_store32(
        &mut machine,
        DMA_BASE + DmaController::SOURCE_OFFSET,
        DMA_SOURCE as u32,
    )?;
    host_store32(
        &mut machine,
        DMA_BASE + DmaController::DESTINATION_OFFSET,
        DMA_DESTINATION as u32,
    )?;
    host_store32(&mut machine, DMA_BASE + DmaController::LENGTH_OFFSET, 2)?;
    host_store32(
        &mut machine,
        DMA_BASE + DmaController::CONTROL_OFFSET,
        DmaController::CONTROL_START | DmaController::CONTROL_IRQ_ENABLE,
    )?;

    let dma_handle = Rc::clone(&dma);
    step_until(&mut machine, 32, move |_| dma_handle.borrow().is_done())?;

    let dma_word0 = host_load32(&mut machine, DMA_DESTINATION)?;
    let dma_word1 = host_load32(&mut machine, DMA_DESTINATION + 4)?;
    let dma_status = host_load32(&mut machine, DMA_BASE + DmaController::CONTROL_OFFSET)?;
    let dma_transferred = host_load32(&mut machine, DMA_BASE + DmaController::TRANSFERRED_OFFSET)?;
    let arbiter_stats = machine.bus().inner().inner().stats();

    println!(
        "x3={} x5={} interrupts_seen={} claimed_source={} external_mcause=0x{:08x} software_mcause=0x{:08x}",
        machine.cpu().hart_state().registers.read(3),
        machine.cpu().hart_state().registers.read(5),
        machine.cpu().hart_state().registers.read(6),
        claimed_source,
        external_mcause,
        software_mcause
    );
    println!(
        "cache stats: icache(hits={} misses={} refills={} refill_words={} bypassed_reads={} evictions={} invalidations={}) dcache(hits={} misses={} refills={} refill_words={} evictions={} dirty_evictions={} write_backs={} write_back_words={} bypassed_writes={} invalidations={})",
        machine.bus().stats().instruction.read_hits,
        machine.bus().stats().instruction.read_misses,
        machine.bus().stats().instruction.refills,
        machine.bus().stats().instruction.refill_words,
        machine.bus().stats().instruction.bypassed_reads,
        machine.bus().stats().instruction.evictions,
        machine.bus().stats().instruction.invalidations,
        machine.bus().stats().data.read_hits,
        machine.bus().stats().data.read_misses,
        machine.bus().stats().data.refills,
        machine.bus().stats().data.refill_words,
        machine.bus().stats().data.evictions,
        machine.bus().stats().data.dirty_evictions,
        machine.bus().stats().data.write_backs,
        machine.bus().stats().data.write_back_words,
        machine.bus().stats().data.bypassed_writes,
        machine.bus().stats().data.invalidations
    );
    println!(
        "l2 stats: hits={} misses={} refills={} refill_words={} evictions={} dirty_evictions={} write_backs={} write_back_words={} bypassed_reads={} bypassed_writes={} invalidations={}",
        machine.bus().inner().stats().read_hits,
        machine.bus().inner().stats().read_misses,
        machine.bus().inner().stats().refills,
        machine.bus().inner().stats().refill_words,
        machine.bus().inner().stats().evictions,
        machine.bus().inner().stats().dirty_evictions,
        machine.bus().inner().stats().write_backs,
        machine.bus().inner().stats().write_back_words,
        machine.bus().inner().stats().bypassed_reads,
        machine.bus().inner().stats().bypassed_writes,
        machine.bus().inner().stats().invalidations
    );
    println!(
        "arbiter stats: master_grants={} cpu_stalls={} dma_done={} dma_status=0x{:08x} dma_words={} dma_word0=0x{:08x} dma_word1=0x{:08x}",
        arbiter_stats.master_grants,
        arbiter_stats.cpu_stall_cycles,
        dma.borrow().is_done(),
        dma_status,
        dma_transferred,
        dma_word0,
        dma_word1
    );

    println!(
        "computer ready: ReferenceCore now demonstrates split L1 caches over a unified L2, DMA bus arbitration, and external plus software interrupts"
    );

    Ok(())
}

fn step_until<P, B, F>(
    machine: &mut Machine<P, B>,
    max_cycles: usize,
    mut predicate: F,
) -> Result<(), P::Error>
where
    P: Processor + CpuModel,
    B: Bus,
    F: FnMut(&Machine<P, B>) -> bool,
{
    for _ in 0..max_cycles {
        let _report = step_machine(machine)?;
        if predicate(machine) {
            return Ok(());
        }
    }

    Ok(())
}

fn step_machine<P, B>(machine: &mut Machine<P, B>) -> Result<CpuCycle, P::Error>
where
    P: Processor + CpuModel,
    B: Bus,
{
    let report = machine.step_cycle()?;
    println!(
        "cycle={} retired={} pc=0x{:08x}",
        machine.clock().current(),
        report.retired_instructions,
        machine.cpu().hart_state().pc
    );
    Ok(report)
}

fn host_store32<P, B>(
    machine: &mut Machine<P, B>,
    addr: u64,
    value: u32,
) -> Result<(), Box<dyn std::error::Error>>
where
    P: Processor + CpuModel,
    P::Error: std::error::Error + 'static,
    B: Bus,
{
    loop {
        match machine.bus_mut().store32(addr, value) {
            Ok(()) => return Ok(()),
            Err(BusError::Busy { .. }) => {
                step_machine(machine)
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
}

fn host_load32<P, B>(
    machine: &mut Machine<P, B>,
    addr: u64,
) -> Result<u32, Box<dyn std::error::Error>>
where
    P: Processor + CpuModel,
    P::Error: std::error::Error + 'static,
    B: Bus,
{
    loop {
        match machine.bus_mut().load32(addr) {
            Ok(value) => return Ok(value),
            Err(BusError::Busy { .. }) => {
                step_machine(machine)
                    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)?;
            }
            Err(error) => return Err(Box::new(error)),
        }
    }
}
