use std::{cell::RefCell, fmt, rc::Rc};

use rvsim_cpu::{CpuError, CpuModel, PipelineCore, ReferenceCore};
use rvsim_devices::{Dram, DramConfig, LatencyAdapter, MachineSoftwareInterrupt, Ram, Rom};
use rvsim_isa::CsrAddress;
use rvsim_system::{
    Address, AddressRange, Addressable, Bus, BusError, CacheConfig, CacheStats, DirectMappedCache,
    Machine, MemoryMap, Processor, StoreAllocationPolicy, WritePolicy,
};

const RESET_VECTOR: u32 = 0;
const RAM_BASE: Address = 0x1000_0000;
const RAM_BYTES: usize = 0x1000;
const VM_RAM_BYTES: usize = 0x10000;
const MSIP_BASE: Address = 0x5000_0000;
const NOP: u32 = 0x0000_0013;
const MRET: u32 = 0x3020_0073;
const PAGE_SHIFT: u32 = 12;
const SATP_MODE_SV32: u32 = 1 << 31;
const MSTATUS_MPRV: u32 = 1 << 17;
const MSTATUS_MPP_SHIFT: u32 = 11;
const PTE_V: u32 = 1 << 0;
const PTE_R: u32 = 1 << 1;
const PTE_W: u32 = 1 << 2;
const PTE_X: u32 = 1 << 3;
const PTE_G: u32 = 1 << 5;
const PTE_A: u32 = 1 << 6;
const PTE_D: u32 = 1 << 7;
const VM_ROOT_TABLE_1: Address = RAM_BASE;
const VM_LEAF_TABLE_1: Address = RAM_BASE + 0x1000;
const VM_ROOT_TABLE_2: Address = RAM_BASE + 0x2000;
const VM_LEAF_TABLE_2: Address = RAM_BASE + 0x3000;
const VM_ROOT_TABLE_3: Address = RAM_BASE + 0x6000;
const VM_PHYS_PAGE_A: Address = RAM_BASE + 0x4000;
const VM_PHYS_PAGE_B: Address = RAM_BASE + 0x5000;
const VM_PHYS_PAGE_C: Address = RAM_BASE + 0x7000;
const VM_VIRTUAL_ADDR: u32 = 0x4000;
const VM_VALUE_A: u32 = 0x1111_2222;
const VM_VALUE_B: u32 = 0x3333_4444;
const VM_VALUE_C: u32 = 0x7777_8888;
const VM_SUPERPAGE_VIRTUAL_BASE: u32 = 0x0040_0000;
const VM_SUPERPAGE_DATA_PHYSICAL_ADDR: Address = RAM_BASE + 0x8000;

#[derive(Debug)]
pub enum MicrobenchError {
    Bus(BusError),
    Cpu(CpuError),
    Timeout(&'static str),
}

impl fmt::Display for MicrobenchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus(error) => write!(f, "{error}"),
            Self::Cpu(error) => write!(f, "{error}"),
            Self::Timeout(name) => write!(f, "microbenchmark {name} timed out"),
        }
    }
}

impl std::error::Error for MicrobenchError {}

impl From<BusError> for MicrobenchError {
    fn from(value: BusError) -> Self {
        Self::Bus(value)
    }
}

impl From<CpuError> for MicrobenchError {
    fn from(value: CpuError) -> Self {
        Self::Cpu(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConflictMissReport {
    pub accesses: usize,
    pub hot_stall_cycles: u64,
    pub thrash_stall_cycles: u64,
    pub hot_stats: CacheStats,
    pub thrash_stats: CacheStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRefillReport {
    pub first_line_stall_cycles: u64,
    pub same_line_stall_cycles: u64,
    pub next_line_stall_cycles: u64,
    pub stats: CacheStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteBackPressureReport {
    pub stores: usize,
    pub stall_cycles: u64,
    pub stats: CacheStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptLatencySample {
    pub idle_cycles: u64,
    pub loaded_cycles: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptLatencyReport {
    pub reference: InterruptLatencySample,
    pub pipeline: InterruptLatencySample,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationCachingSample {
    pub cold_cycles: u64,
    pub warm_cycles: u64,
    pub switched_asid_cycles: u64,
    pub returned_asid_cycles: u64,
    pub global_switched_asid_cycles: u64,
    pub sfence_reload_cycles: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranslationCachingReport {
    pub reference: TranslationCachingSample,
    pub pipeline: TranslationCachingSample,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualMemoryPathsSample {
    pub superpage_access_cycles: u64,
    pub namespace_preserved_cycles: u64,
    pub namespace_reloaded_cycles: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualMemoryPathsReport {
    pub reference: VirtualMemoryPathsSample,
    pub pipeline: VirtualMemoryPathsSample,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryMicrobenchReport {
    pub conflict_miss: ConflictMissReport,
    pub line_refill: LineRefillReport,
    pub write_back_pressure: WriteBackPressureReport,
    pub interrupt_latency: InterruptLatencyReport,
    pub translation_caching: TranslationCachingReport,
    pub virtual_memory_paths: VirtualMemoryPathsReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AccessMeasurement {
    stall_cycles: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterruptLoad {
    Idle,
    Loaded,
}

pub fn run_memory_microbenchmarks() -> Result<MemoryMicrobenchReport, MicrobenchError> {
    Ok(MemoryMicrobenchReport {
        conflict_miss: run_conflict_miss_benchmark()?,
        line_refill: run_line_refill_benchmark()?,
        write_back_pressure: run_write_back_pressure_benchmark()?,
        interrupt_latency: run_interrupt_latency_benchmark()?,
        translation_caching: run_translation_caching_benchmark()?,
        virtual_memory_paths: run_virtual_memory_paths_benchmark()?,
    })
}

pub fn run_conflict_miss_benchmark() -> Result<ConflictMissReport, MicrobenchError> {
    const HOT_ADDR: Address = RAM_BASE;
    const CONFLICT_ADDR: Address = RAM_BASE + 0x20;

    let hot_pattern = [HOT_ADDR, HOT_ADDR, HOT_ADDR, HOT_ADDR];
    let thrash_pattern = [HOT_ADDR, CONFLICT_ADDR, HOT_ADDR, CONFLICT_ADDR];

    let (hot_stall_cycles, hot_stats) = run_alias_pattern(&hot_pattern)?;
    let (thrash_stall_cycles, thrash_stats) = run_alias_pattern(&thrash_pattern)?;

    Ok(ConflictMissReport {
        accesses: hot_pattern.len(),
        hot_stall_cycles,
        thrash_stall_cycles,
        hot_stats,
        thrash_stats,
    })
}

pub fn run_line_refill_benchmark() -> Result<LineRefillReport, MicrobenchError> {
    let mut dram = Dram::new(RAM_BASE, RAM_BYTES, DramConfig::new(64, 6, 2, 1));
    write_word(&mut dram, RAM_BASE, 0x1111_1111)?;
    write_word(&mut dram, RAM_BASE + 4, 0x2222_2222)?;
    write_word(&mut dram, RAM_BASE + 16, 0x3333_3333)?;

    let mut memory = MemoryMap::new();
    memory.map_device(dram)?;

    let mut cache = DirectMappedCache::new(
        memory,
        CacheConfig::new(8, vec![AddressRange::new(RAM_BASE, RAM_BYTES as u64)]).with_line_size(16),
    );

    let first_line = measure_load32(&mut cache, RAM_BASE)?;
    let same_line = measure_load32(&mut cache, RAM_BASE + 4)?;
    let next_line = measure_load32(&mut cache, RAM_BASE + 16)?;

    Ok(LineRefillReport {
        first_line_stall_cycles: first_line.stall_cycles,
        same_line_stall_cycles: same_line.stall_cycles,
        next_line_stall_cycles: next_line.stall_cycles,
        stats: cache.stats(),
    })
}

pub fn run_write_back_pressure_benchmark() -> Result<WriteBackPressureReport, MicrobenchError> {
    let mut ram = Ram::new(RAM_BASE, RAM_BYTES);
    for (index, addr) in [RAM_BASE, RAM_BASE + 0x10, RAM_BASE + 0x20, RAM_BASE + 0x30]
        .into_iter()
        .enumerate()
    {
        write_word(&mut ram, addr, index as u32)?;
    }

    let mut memory = MemoryMap::new();
    memory.map_device(LatencyAdapter::new(ram, 4))?;

    let mut cache = DirectMappedCache::new(
        memory,
        CacheConfig::new(1, vec![AddressRange::new(RAM_BASE, RAM_BYTES as u64)])
            .with_line_size(16)
            .with_write_policy(WritePolicy::WriteBack)
            .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
    );

    let mut stall_cycles = 0;
    for (index, addr) in [RAM_BASE, RAM_BASE + 0x10, RAM_BASE + 0x20, RAM_BASE + 0x30]
        .into_iter()
        .enumerate()
    {
        stall_cycles += measure_store32(&mut cache, addr, 0x1000 + index as u32)?.stall_cycles;
    }

    Ok(WriteBackPressureReport {
        stores: 4,
        stall_cycles,
        stats: cache.stats(),
    })
}

pub fn run_interrupt_latency_benchmark() -> Result<InterruptLatencyReport, MicrobenchError> {
    Ok(InterruptLatencyReport {
        reference: InterruptLatencySample {
            idle_cycles: measure_interrupt_latency(ReferenceCore::new, InterruptLoad::Idle)?,
            loaded_cycles: measure_interrupt_latency(ReferenceCore::new, InterruptLoad::Loaded)?,
        },
        pipeline: InterruptLatencySample {
            idle_cycles: measure_interrupt_latency(PipelineCore::new, InterruptLoad::Idle)?,
            loaded_cycles: measure_interrupt_latency(PipelineCore::new, InterruptLoad::Loaded)?,
        },
    })
}

pub fn run_translation_caching_benchmark() -> Result<TranslationCachingReport, MicrobenchError> {
    Ok(TranslationCachingReport {
        reference: measure_translation_caching(ReferenceCore::new)?,
        pipeline: measure_translation_caching(PipelineCore::new)?,
    })
}

pub fn run_virtual_memory_paths_benchmark() -> Result<VirtualMemoryPathsReport, MicrobenchError> {
    Ok(VirtualMemoryPathsReport {
        reference: measure_virtual_memory_paths(ReferenceCore::new)?,
        pipeline: measure_virtual_memory_paths(PipelineCore::new)?,
    })
}

fn run_alias_pattern(pattern: &[Address]) -> Result<(u64, CacheStats), MicrobenchError> {
    let mut ram = Ram::new(RAM_BASE, RAM_BYTES);
    write_word(&mut ram, RAM_BASE, 0xaaaa_0001)?;
    write_word(&mut ram, RAM_BASE + 0x20, 0xbbbb_0002)?;

    let mut memory = MemoryMap::new();
    memory.map_device(LatencyAdapter::new(ram, 6))?;

    let mut cache = DirectMappedCache::new(
        memory,
        CacheConfig::new(2, vec![AddressRange::new(RAM_BASE, RAM_BYTES as u64)]).with_line_size(16),
    );

    let mut stall_cycles = 0;
    for &addr in pattern {
        stall_cycles += measure_load32(&mut cache, addr)?.stall_cycles;
    }

    Ok((stall_cycles, cache.stats()))
}

fn measure_load32<B>(bus: &mut B, addr: Address) -> Result<AccessMeasurement, MicrobenchError>
where
    B: Bus,
{
    let mut stall_cycles = 0;
    loop {
        match bus.load32(addr) {
            Ok(_) => {
                return Ok(AccessMeasurement { stall_cycles });
            }
            Err(BusError::Busy { .. }) => {
                stall_cycles += 1;
                bus.tick();
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn measure_store32<B>(
    bus: &mut B,
    addr: Address,
    value: u32,
) -> Result<AccessMeasurement, MicrobenchError>
where
    B: Bus,
{
    let mut stall_cycles = 0;
    loop {
        match bus.store32(addr, value) {
            Ok(()) => {
                return Ok(AccessMeasurement { stall_cycles });
            }
            Err(BusError::Busy { .. }) => {
                stall_cycles += 1;
                bus.tick();
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn measure_interrupt_latency<P>(
    make_cpu: fn(u32) -> P,
    load: InterruptLoad,
) -> Result<u64, MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let (mut machine, msip) = build_interrupt_machine(make_cpu, load)?;

    step_until(
        &mut machine,
        16,
        "interrupt benchmark warmup",
        |machine| match load {
            InterruptLoad::Idle => machine.cpu().hart_state().pc == 0,
            InterruptLoad::Loaded => machine.cpu().hart_state().pc == 4,
        },
    )?;

    let signal_cycle = match load {
        InterruptLoad::Idle => machine.clock().current(),
        InterruptLoad::Loaded => {
            step_until(&mut machine, 32, "memory pressure trigger", |machine| {
                machine.bus().is_busy()
            })?;
            machine.clock().current()
        }
    };

    set_msip(&msip, 1)?;
    step_until(&mut machine, 64, "interrupt handler", |machine| {
        machine.cpu().hart_state().registers.read(10) >= 1
    })?;
    set_msip(&msip, 0)?;

    Ok(machine.clock().current() - signal_cycle)
}

fn measure_translation_caching<P>(
    make_cpu: fn(u32) -> P,
) -> Result<TranslationCachingSample, MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_translation_machine(make_cpu, false)?;
    let cold_cycles = measure_translation_phase(&mut machine, 1, "translation cold load")?;
    assert_eq!(machine.cpu().hart_state().registers.read(2), VM_VALUE_A);

    let warm_cycles = measure_translation_phase(&mut machine, 2, "translation warm load")?;
    assert_eq!(machine.cpu().hart_state().registers.read(2), VM_VALUE_A);

    let switched_asid_cycles =
        measure_translation_phase(&mut machine, 3, "translation switched asid load")?;
    assert_eq!(machine.cpu().hart_state().registers.read(2), VM_VALUE_B);

    let returned_asid_cycles =
        measure_translation_phase(&mut machine, 4, "translation returned asid load")?;
    assert_eq!(machine.cpu().hart_state().registers.read(2), VM_VALUE_A);

    let sfence_reload_cycles =
        measure_translation_phase(&mut machine, 5, "translation sfence reload")?;
    assert_eq!(machine.cpu().hart_state().registers.read(2), VM_VALUE_A);

    let mut global_machine = build_translation_machine(make_cpu, true)?;
    let _ = measure_translation_phase(&mut global_machine, 1, "global translation cold load")?;
    let _ = measure_translation_phase(&mut global_machine, 2, "global translation warm load")?;
    let global_switched_asid_cycles = measure_translation_phase(
        &mut global_machine,
        3,
        "global translation switched asid load",
    )?;
    assert_eq!(
        global_machine.cpu().hart_state().registers.read(2),
        VM_VALUE_A
    );

    Ok(TranslationCachingSample {
        cold_cycles,
        warm_cycles,
        switched_asid_cycles,
        returned_asid_cycles,
        global_switched_asid_cycles,
        sfence_reload_cycles,
    })
}

fn measure_virtual_memory_paths<P>(
    make_cpu: fn(u32) -> P,
) -> Result<VirtualMemoryPathsSample, MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut superpage_machine = build_superpage_access_machine(make_cpu)?;
    let superpage_start = superpage_machine.clock().current();
    step_until(
        &mut superpage_machine,
        256,
        "superpage access flow",
        |machine| machine.cpu().hart_state().registers.read(10) == 9,
    )?;
    let superpage_access_cycles = superpage_machine.clock().current() - superpage_start;

    let mut namespace_machine = build_namespace_preserve_machine(make_cpu)?;
    step_until(
        &mut namespace_machine,
        256,
        "namespace preserve warmup",
        |machine| machine.cpu().hart_state().registers.read(3) >= 2,
    )?;
    assert_eq!(
        namespace_machine.cpu().hart_state().registers.read(10),
        VM_VALUE_A
    );
    assert_eq!(
        namespace_machine.cpu().hart_state().registers.read(11),
        VM_VALUE_B
    );

    let namespace_preserve_start = namespace_machine.clock().current();
    step_until(
        &mut namespace_machine,
        256,
        "namespace preserved stale load",
        |machine| machine.cpu().hart_state().registers.read(3) >= 3,
    )?;
    assert_eq!(
        namespace_machine.cpu().hart_state().registers.read(12),
        VM_VALUE_A
    );
    let namespace_preserved_cycles = namespace_machine.clock().current() - namespace_preserve_start;

    let namespace_reload_start = namespace_machine.clock().current();
    step_until(
        &mut namespace_machine,
        256,
        "namespace reloaded load",
        |machine| machine.cpu().hart_state().registers.read(3) >= 4,
    )?;
    assert_eq!(
        namespace_machine.cpu().hart_state().registers.read(13),
        VM_VALUE_C
    );
    let namespace_reloaded_cycles = namespace_machine.clock().current() - namespace_reload_start;

    Ok(VirtualMemoryPathsSample {
        superpage_access_cycles,
        namespace_preserved_cycles,
        namespace_reloaded_cycles,
    })
}

fn build_interrupt_machine<P>(
    make_cpu: fn(u32) -> P,
    load: InterruptLoad,
) -> Result<(Machine<P, MemoryMap>, Rc<RefCell<MachineSoftwareInterrupt>>), MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let msip = Rc::new(RefCell::new(MachineSoftwareInterrupt::new(MSIP_BASE)));
    let program = match load {
        InterruptLoad::Idle => idle_interrupt_program(),
        InterruptLoad::Loaded => loaded_interrupt_program(),
    };

    let mut dram = Dram::new(RAM_BASE, RAM_BYTES, DramConfig::new(64, 6, 2, 1));
    write_word(&mut dram, RAM_BASE, 0xfeed_face)?;

    let mut memory = MemoryMap::new();
    memory.map_device(Rom::from_words(RESET_VECTOR as u64, &program))?;
    memory.map_device(dram)?;
    memory.map_shared_device(Rc::clone(&msip))?;

    let mut machine = Machine::new(make_cpu(RESET_VECTOR), memory);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, 1 << 3);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mie, 1 << 3);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);

    Ok((machine, msip))
}

fn build_translation_machine<P>(
    make_cpu: fn(u32) -> P,
    global_mapping: bool,
) -> Result<Machine<P, MemoryMap>, MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let satp_asid_1 = sv32_satp_with_asid(VM_ROOT_TABLE_1, 1);
    let satp_asid_2 = sv32_satp_with_asid(VM_ROOT_TABLE_2, 2);
    let mapping_flags = PTE_R | PTE_W | PTE_A | PTE_D | if global_mapping { PTE_G } else { 0 };

    let mut ram = Ram::new(RAM_BASE, VM_RAM_BYTES);
    write_word(&mut ram, VM_PHYS_PAGE_A, VM_VALUE_A)?;
    write_word(
        &mut ram,
        if global_mapping {
            VM_PHYS_PAGE_A
        } else {
            VM_PHYS_PAGE_B
        },
        if global_mapping {
            VM_VALUE_A
        } else {
            VM_VALUE_B
        },
    )?;
    install_sv32_mapping(
        &mut ram,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_VIRTUAL_ADDR,
        VM_PHYS_PAGE_A as u32,
        mapping_flags,
    )?;
    install_sv32_mapping(
        &mut ram,
        VM_ROOT_TABLE_2,
        VM_LEAF_TABLE_2,
        VM_VIRTUAL_ADDR,
        if global_mapping {
            VM_PHYS_PAGE_A as u32
        } else {
            VM_PHYS_PAGE_B as u32
        },
        mapping_flags,
    )?;

    let program = if global_mapping {
        global_translation_program()
    } else {
        translation_program()
    };

    let mut memory = MemoryMap::new();
    memory.map_device(Rom::from_words(RESET_VECTOR as u64, &program))?;
    memory.map_device(LatencyAdapter::new(ram, 4))?;

    let mut machine = Machine::new(make_cpu(RESET_VECTOR), memory);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_VIRTUAL_ADDR);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(4, satp_asid_1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(5, satp_asid_2);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, satp_asid_1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));

    Ok(machine)
}

fn build_superpage_access_machine<P>(
    make_cpu: fn(u32) -> P,
) -> Result<Machine<P, MemoryMap>, MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let program = superpage_access_program();
    let mut ram = Ram::new(RAM_BASE, VM_RAM_BYTES);
    write_word(&mut ram, VM_SUPERPAGE_DATA_PHYSICAL_ADDR, 9)?;
    install_sv32_superpage_mapping(
        &mut ram,
        VM_ROOT_TABLE_3,
        VM_SUPERPAGE_VIRTUAL_BASE,
        RAM_BASE as u32,
        PTE_R | PTE_W | PTE_X | PTE_A | PTE_D,
    )?;

    let mut memory = MemoryMap::new();
    memory.map_device(Rom::from_words(RESET_VECTOR as u64, &program))?;
    memory.map_device(LatencyAdapter::new(ram, 4))?;

    let mut machine = Machine::new(make_cpu(RESET_VECTOR), memory);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_3, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
    Ok(machine)
}

fn build_namespace_preserve_machine<P>(
    make_cpu: fn(u32) -> P,
) -> Result<Machine<P, MemoryMap>, MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let satp_asid_1 = sv32_satp_with_asid(VM_ROOT_TABLE_1, 1);
    let satp_asid_2 = sv32_satp_with_asid(VM_ROOT_TABLE_2, 2);

    let mut ram = Ram::new(RAM_BASE, VM_RAM_BYTES);
    write_word(&mut ram, VM_PHYS_PAGE_A, VM_VALUE_A)?;
    write_word(&mut ram, VM_PHYS_PAGE_B, VM_VALUE_B)?;
    write_word(&mut ram, VM_PHYS_PAGE_C, VM_VALUE_C)?;
    install_sv32_mapping(
        &mut ram,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_VIRTUAL_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_A | PTE_D,
    )?;
    install_sv32_mapping(
        &mut ram,
        VM_ROOT_TABLE_2,
        VM_LEAF_TABLE_2,
        VM_VIRTUAL_ADDR,
        VM_PHYS_PAGE_B as u32,
        PTE_R | PTE_A | PTE_D,
    )?;

    let mut memory = MemoryMap::new();
    memory.map_device(Rom::from_words(
        RESET_VECTOR as u64,
        &namespace_preserve_program(),
    ))?;
    memory.map_device(LatencyAdapter::new(ram, 4))?;

    let mut machine = Machine::new(make_cpu(RESET_VECTOR), memory);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_VIRTUAL_ADDR);
    machine.cpu_mut().hart_state_mut().registers.write(
        4,
        (VM_LEAF_TABLE_1 + (((VM_VIRTUAL_ADDR >> 12) & 0x3ff) as u64) * 4) as u32,
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(5, satp_asid_2);
    machine.cpu_mut().hart_state_mut().registers.write(
        6,
        PTE_V | PTE_R | PTE_A | PTE_D | ((VM_PHYS_PAGE_C as u32 >> PAGE_SHIFT) << 10),
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(7, 1 << MSTATUS_MPP_SHIFT);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(8, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(9, satp_asid_1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, satp_asid_1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));

    Ok(machine)
}

fn step_until<P, B, F>(
    machine: &mut Machine<P, B>,
    max_cycles: usize,
    benchmark: &'static str,
    mut predicate: F,
) -> Result<(), MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
    B: Bus,
    F: FnMut(&Machine<P, B>) -> bool,
{
    for _ in 0..max_cycles {
        machine.step_cycle()?;
        if predicate(machine) {
            return Ok(());
        }
    }

    Err(MicrobenchError::Timeout(benchmark))
}

fn measure_translation_phase<P>(
    machine: &mut Machine<P, MemoryMap>,
    target_load_count: u32,
    benchmark: &'static str,
) -> Result<u64, MicrobenchError>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let start = machine.clock().current();
    step_until(machine, 128, benchmark, |machine| {
        machine.cpu().hart_state().registers.read(3) >= target_load_count
    })?;
    Ok(machine.clock().current() - start)
}

fn set_msip(
    device: &Rc<RefCell<MachineSoftwareInterrupt>>,
    value: u32,
) -> Result<(), MicrobenchError> {
    let mut device = device.borrow_mut();
    for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
        device.store8(MSIP_BASE + offset as u64, byte)?;
    }
    Ok(())
}

fn write_word<D>(device: &mut D, addr: Address, value: u32) -> Result<(), MicrobenchError>
where
    D: Addressable,
{
    for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
        device.store8(addr + offset as u64, byte)?;
    }
    Ok(())
}

fn idle_interrupt_program() -> Vec<u32> {
    let mut program = vec![
        NOP,
        encode_jal(0, -4),
        NOP,
        NOP,
        NOP,
        NOP,
        NOP,
        NOP,
        encode_addi(10, 10, 1),
        MRET,
    ];
    program.extend([NOP; 4]);
    program
}

fn loaded_interrupt_program() -> Vec<u32> {
    let mut program = vec![
        encode_lui(1, (RAM_BASE >> 12) as u32),
        encode_lw(2, 1, 0),
        encode_jal(0, -4),
        NOP,
        NOP,
        NOP,
        NOP,
        NOP,
        encode_addi(10, 10, 1),
        MRET,
    ];
    program.extend([NOP; 4]);
    program
}

fn translation_program() -> Vec<u32> {
    vec![
        encode_lw(2, 1, 0),
        encode_addi(3, 3, 1),
        encode_lw(2, 1, 0),
        encode_addi(3, 3, 1),
        encode_csrrw(0, CsrAddress::Satp as u16, 5),
        encode_lw(2, 1, 0),
        encode_addi(3, 3, 1),
        encode_csrrw(0, CsrAddress::Satp as u16, 4),
        encode_lw(2, 1, 0),
        encode_addi(3, 3, 1),
        encode_sfence_vma(0, 0),
        encode_lw(2, 1, 0),
        encode_addi(3, 3, 1),
        encode_jal(0, 0),
    ]
}

fn global_translation_program() -> Vec<u32> {
    vec![
        encode_lw(2, 1, 0),
        encode_addi(3, 3, 1),
        encode_lw(2, 1, 0),
        encode_addi(3, 3, 1),
        encode_csrrw(0, CsrAddress::Satp as u16, 5),
        encode_lw(2, 1, 0),
        encode_addi(3, 3, 1),
        encode_jal(0, 0),
    ]
}

fn superpage_access_program() -> Vec<u32> {
    vec![
        encode_lui(1, VM_SUPERPAGE_VIRTUAL_BASE.wrapping_add(0x8000) >> 12),
        encode_lw(10, 1, 0),
        encode_jal(0, 0),
    ]
}

fn namespace_preserve_program() -> Vec<u32> {
    vec![
        encode_lw(10, 1, 0),
        encode_addi(3, 3, 1),
        encode_csrrw(0, CsrAddress::Satp as u16, 5),
        encode_lw(11, 1, 0),
        encode_addi(3, 3, 1),
        encode_csrrw(0, CsrAddress::Mstatus as u16, 7),
        encode_sw(6, 4, 0),
        encode_csrrw(0, CsrAddress::Mstatus as u16, 8),
        encode_csrrw(0, CsrAddress::Satp as u16, 9),
        encode_lw(12, 1, 0),
        encode_addi(3, 3, 1),
        encode_sfence_vma(0, 0),
        encode_lw(13, 1, 0),
        encode_addi(3, 3, 1),
        encode_jal(0, 0),
    ]
}

fn encode_addi(rd: u8, rs1: u8, imm: i16) -> u32 {
    encode_i(imm, rs1, 0b000, rd, 0b0010011)
}

fn encode_lw(rd: u8, rs1: u8, imm: i16) -> u32 {
    encode_i(imm, rs1, 0b010, rd, 0b0000011)
}

fn encode_sw(rs2: u8, rs1: u8, imm: i16) -> u32 {
    let imm = imm as u16 as u32;
    (((imm >> 5) & 0x7f) << 25)
        | ((rs2 as u32) << 20)
        | ((rs1 as u32) << 15)
        | (0b010 << 12)
        | ((imm & 0x1f) << 7)
        | 0b0100011
}

fn encode_lui(rd: u8, upper_20: u32) -> u32 {
    (upper_20 << 12) | ((rd as u32) << 7) | 0b0110111
}

fn encode_jal(rd: u8, imm: i32) -> u32 {
    let imm = imm as u32;
    let bit20 = ((imm >> 20) & 0x1) << 31;
    let bits10_1 = ((imm >> 1) & 0x03ff) << 21;
    let bit11 = ((imm >> 11) & 0x1) << 20;
    let bits19_12 = ((imm >> 12) & 0xff) << 12;
    bit20 | bits19_12 | bit11 | bits10_1 | ((rd as u32) << 7) | 0b1101111
}

fn encode_csrrw(rd: u8, csr: u16, rs1: u8) -> u32 {
    ((csr as u32) << 20) | ((rs1 as u32) << 15) | (0b001 << 12) | ((rd as u32) << 7) | 0b1110011
}

fn encode_sfence_vma(rs1: u8, rs2: u8) -> u32 {
    0x1200_0073 | ((rs1 as u32) << 15) | ((rs2 as u32) << 20)
}

fn encode_i(imm: i16, rs1: u8, funct3: u32, rd: u8, opcode: u32) -> u32 {
    (((imm as u16 as u32) & 0x0fff) << 20)
        | ((rs1 as u32) << 15)
        | (funct3 << 12)
        | ((rd as u32) << 7)
        | opcode
}

fn install_sv32_mapping<D>(
    device: &mut D,
    root_table: Address,
    leaf_table: Address,
    virtual_address: u32,
    physical_address: u32,
    flags: u32,
) -> Result<(), MicrobenchError>
where
    D: Addressable,
{
    let root_index = ((virtual_address >> 22) & 0x3ff) as u64;
    let leaf_index = ((virtual_address >> 12) & 0x3ff) as u64;
    let root_pte = PTE_V | ((leaf_table as u32 >> PAGE_SHIFT) << 10);
    let leaf_pte = PTE_V | flags | ((physical_address >> PAGE_SHIFT) << 10);

    write_word(device, root_table + root_index * 4, root_pte)?;
    write_word(device, leaf_table + leaf_index * 4, leaf_pte)?;
    Ok(())
}

fn install_sv32_superpage_mapping<D>(
    device: &mut D,
    root_table: Address,
    virtual_address: u32,
    physical_base: u32,
    flags: u32,
) -> Result<(), MicrobenchError>
where
    D: Addressable,
{
    let root_index = ((virtual_address >> 22) & 0x3ff) as u64;
    let root_pte = PTE_V | flags | ((physical_base >> PAGE_SHIFT) << 10);
    write_word(device, root_table + root_index * 4, root_pte)?;
    Ok(())
}

const fn sv32_satp_with_asid(root_table: Address, asid: u32) -> u32 {
    SATP_MODE_SV32 | (asid << 22) | ((root_table as u32) >> PAGE_SHIFT)
}

#[cfg(test)]
mod tests {
    use super::{
        run_conflict_miss_benchmark, run_interrupt_latency_benchmark, run_line_refill_benchmark,
        run_memory_microbenchmarks, run_translation_caching_benchmark,
        run_virtual_memory_paths_benchmark, run_write_back_pressure_benchmark,
    };

    #[test]
    fn conflict_miss_benchmark_shows_alias_penalty() {
        let report = run_conflict_miss_benchmark().expect("benchmark should run");

        assert!(report.thrash_stall_cycles > report.hot_stall_cycles);
        assert!(report.thrash_stats.read_misses > report.hot_stats.read_misses);
    }

    #[test]
    fn line_refill_benchmark_distinguishes_hits_from_refills() {
        let report = run_line_refill_benchmark().expect("benchmark should run");

        assert!(report.first_line_stall_cycles > report.same_line_stall_cycles);
        assert!(report.next_line_stall_cycles > report.same_line_stall_cycles);
        assert_eq!(report.stats.refills, 2);
    }

    #[test]
    fn write_back_pressure_benchmark_records_dirty_evictions() {
        let report = run_write_back_pressure_benchmark().expect("benchmark should run");

        assert!(report.stall_cycles > 0);
        assert!(report.stats.dirty_evictions > 0);
        assert!(report.stats.write_back_words > 0);
    }

    #[test]
    fn interrupt_latency_benchmark_reports_loaded_penalty() {
        let report = run_interrupt_latency_benchmark().expect("benchmark should run");

        assert!(report.reference.idle_cycles > 0);
        assert!(report.reference.loaded_cycles >= report.reference.idle_cycles);
        assert!(report.pipeline.idle_cycles > 0);
        assert!(report.pipeline.loaded_cycles >= report.pipeline.idle_cycles);
    }

    #[test]
    fn translation_caching_benchmark_shows_namespace_and_flush_effects() {
        let report = run_translation_caching_benchmark().expect("benchmark should run");

        for sample in [report.reference, report.pipeline] {
            assert!(sample.cold_cycles >= sample.warm_cycles);
            assert!(sample.switched_asid_cycles >= sample.returned_asid_cycles);
            assert!(sample.global_switched_asid_cycles <= sample.switched_asid_cycles);
            assert!(sample.sfence_reload_cycles >= sample.returned_asid_cycles);
        }
    }

    #[test]
    fn virtual_memory_paths_benchmark_reports_superpage_and_namespace_flows() {
        let report = run_virtual_memory_paths_benchmark().expect("benchmark should run");

        for sample in [report.reference, report.pipeline] {
            assert!(sample.superpage_access_cycles > 0);
            assert!(sample.namespace_preserved_cycles > 0);
            assert!(sample.namespace_reloaded_cycles > 0);
        }
    }

    #[test]
    fn top_level_microbenchmark_runner_completes() {
        run_memory_microbenchmarks().expect("all benchmarks should run");
    }
}
