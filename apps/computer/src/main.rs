use rvsim_cpu::{CpuModel, ReferenceCore};
use rvsim_devices::{InterruptController, MachineSoftwareInterrupt, Ram, Rom, SimpleUart};
use rvsim_isa::CsrAddress;
use rvsim_system::{
    AddressRange, Bus, CacheConfig, DirectMappedCache, Machine, MemoryMap, ReplacementPolicy,
    SplitL1Cache, StoreAllocationPolicy, WritePolicy,
};

const RESET_VECTOR: u32 = 0x0000_0000;
const RAM_BASE: u64 = 0x1000_0000;
const UART_BASE: u64 = 0x2000_0000;
const INTERRUPT_CONTROLLER_BASE: u64 = 0x4000_0000;
const MSIP_BASE: u64 = 0x5000_0000;

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

    let mut memory = MemoryMap::new();
    memory.map_device(Rom::from_words(RESET_VECTOR as u64, &program))?;
    memory.map_device(Ram::new(RAM_BASE, 0x1000))?;
    memory.map_device(SimpleUart::new(UART_BASE))?;
    memory.map_device(InterruptController::new(INTERRUPT_CONTROLLER_BASE))?;
    memory.map_device(MachineSoftwareInterrupt::new(MSIP_BASE))?;

    let l2 = DirectMappedCache::new(
        memory,
        CacheConfig::new(
            128,
            vec![
                AddressRange::new(RESET_VECTOR as u64, 0x1000),
                AddressRange::new(RAM_BASE, 0x1000),
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
        CacheConfig::new(64, vec![AddressRange::new(RAM_BASE, 0x1000)])
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

    for _ in 0..6 {
        let report = machine.step_cycle()?;
        println!(
            "cycle={} retired={} pc=0x{:08x}",
            machine.clock().current(),
            report.retired_instructions,
            machine.cpu().hart_state().pc
        );
    }

    machine
        .bus_mut()
        .store32(INTERRUPT_CONTROLLER_BASE + 4, 1)?;
    machine
        .bus_mut()
        .store32(INTERRUPT_CONTROLLER_BASE + 8, 1)?;

    for _ in 0..3 {
        let report = machine.step_cycle()?;
        println!(
            "cycle={} retired={} pc=0x{:08x}",
            machine.clock().current(),
            report.retired_instructions,
            machine.cpu().hart_state().pc
        );
    }

    let external_mcause = machine.cpu().hart_state().csrs.read(CsrAddress::Mcause);
    let claimed_source = machine.bus_mut().load32(INTERRUPT_CONTROLLER_BASE + 12)?;
    machine
        .bus_mut()
        .store32(INTERRUPT_CONTROLLER_BASE + 12, claimed_source)?;

    machine.bus_mut().store32(MSIP_BASE, 1)?;

    for _ in 0..3 {
        let report = machine.step_cycle()?;
        println!(
            "cycle={} retired={} pc=0x{:08x}",
            machine.clock().current(),
            report.retired_instructions,
            machine.cpu().hart_state().pc
        );
    }

    let software_mcause = machine.cpu().hart_state().csrs.read(CsrAddress::Mcause);
    machine.bus_mut().store32(MSIP_BASE, 0)?;

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
        "cache stats: icache(hits={} misses={} refills={} evictions={} invalidations={}) dcache(hits={} misses={} refills={} evictions={} write_backs={} invalidations={})",
        machine.bus().stats().instruction.read_hits,
        machine.bus().stats().instruction.read_misses,
        machine.bus().stats().instruction.refills,
        machine.bus().stats().instruction.evictions,
        machine.bus().stats().instruction.invalidations,
        machine.bus().stats().data.read_hits,
        machine.bus().stats().data.read_misses,
        machine.bus().stats().data.refills,
        machine.bus().stats().data.evictions,
        machine.bus().stats().data.write_backs,
        machine.bus().stats().data.invalidations
    );
    println!(
        "l2 stats: hits={} misses={} refills={} evictions={} write_backs={} invalidations={}",
        machine.bus().inner().stats().read_hits,
        machine.bus().inner().stats().read_misses,
        machine.bus().inner().stats().refills,
        machine.bus().inner().stats().evictions,
        machine.bus().inner().stats().write_backs,
        machine.bus().inner().stats().invalidations
    );

    println!(
        "computer ready: ReferenceCore now demonstrates split L1 caches over a unified L2 plus external and software interrupts"
    );

    Ok(())
}
