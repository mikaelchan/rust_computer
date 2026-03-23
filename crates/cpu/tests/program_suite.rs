use rvsim_cpu::{CpuError, CpuModel, PipelineCore, ReferenceCore};
use rvsim_devices::{MachineSoftwareInterrupt, Ram, Rom};
use rvsim_isa::CsrAddress;
use rvsim_system::{Bus, Machine, MemoryMap, Processor};

const RESET_VECTOR: u32 = 0;
const RAM_BASE: u64 = 0x1000_0000;
const RAM_BYTES: usize = 0x1000;
const VM_RAM_BYTES: usize = 0x8000;
const MSIP_BASE: u64 = 0x5000_0000;
const PAGE_SHIFT: u32 = 12;
const SATP_MODE_SV32: u32 = 1 << 31;
const MSTATUS_MPRV: u32 = 1 << 17;
const MSTATUS_MPP_SHIFT: u32 = 11;
const PTE_V: u32 = 1 << 0;
const PTE_R: u32 = 1 << 1;
const PTE_W: u32 = 1 << 2;
const PTE_G: u32 = 1 << 5;
const PTE_A: u32 = 1 << 6;
const PTE_D: u32 = 1 << 7;
const VM_ROOT_TABLE_1: u64 = RAM_BASE;
const VM_LEAF_TABLE_1: u64 = RAM_BASE + 0x1000;
const VM_ROOT_TABLE_2: u64 = RAM_BASE + 0x2000;
const VM_LEAF_TABLE_2: u64 = RAM_BASE + 0x3000;
const VM_PHYS_PAGE_A: u64 = RAM_BASE + 0x4000;
const VM_PHYS_PAGE_B: u64 = RAM_BASE + 0x5000;
const VM_ROOT_TABLE_3: u64 = RAM_BASE + 0x6000;
const VM_VIRTUAL_ADDR: u32 = 0x4000;
const VM_VALUE_A: u32 = 0x1111_2222;
const VM_VALUE_B: u32 = 0x3333_4444;
const VM_SUPERPAGE_VIRTUAL_ADDR: u32 = 0x0040_5000;
const VM_SUPERPAGE_PHYSICAL_ADDR: u64 = RAM_BASE + 0x5000;
const VM_SUPERPAGE_STORE_VALUE: u32 = 0x5555_6666;

const STORE_LOAD_PROGRAM: &str = include_str!("programs/store_load.hex");
const COUNT_LOOP_PROGRAM: &str = include_str!("programs/count_loop.hex");
const MSIP_INTERRUPT_PROGRAM: &str = include_str!("programs/msip_interrupt.hex");
const SV32_ASID_SWITCH_PROGRAM: &str = include_str!("programs/sv32_asid_switch.hex");
const SV32_SFENCE_REMAP_PROGRAM: &str = include_str!("programs/sv32_sfence_remap.hex");
const SV32_SUPERPAGE_PROGRAM: &str = include_str!("programs/sv32_superpage.hex");
const SV32_GLOBAL_ASID_FENCE_PROGRAM: &str = include_str!("programs/sv32_global_asid_fence.hex");

fn parse_program_image(source: &str) -> Vec<u32> {
    source
        .lines()
        .filter_map(|line| {
            let trimmed = line.split('#').next().unwrap_or_default().trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(
                    u32::from_str_radix(trimmed.trim_start_matches("0x"), 16)
                        .expect("program image line should be valid hex"),
                )
            }
        })
        .collect()
}

fn build_machine<P>(cpu: P, program_image: &str) -> Machine<P, MemoryMap>
where
    P: Processor<Error = CpuError> + CpuModel,
{
    build_machine_with(cpu, program_image, RAM_BYTES, |_| {})
}

fn build_machine_with<P, F>(
    cpu: P,
    program_image: &str,
    ram_bytes: usize,
    setup: F,
) -> Machine<P, MemoryMap>
where
    P: Processor<Error = CpuError> + CpuModel,
    F: FnOnce(&mut Machine<P, MemoryMap>),
{
    let words = parse_program_image(program_image);
    let mut memory = MemoryMap::new();
    memory
        .map_device(Rom::from_words(RESET_VECTOR as u64, &words))
        .expect("ROM should map");
    memory
        .map_device(Ram::new(RAM_BASE, ram_bytes))
        .expect("RAM should map");
    memory
        .map_device(MachineSoftwareInterrupt::new(MSIP_BASE))
        .expect("MSIP device should map");

    let mut machine = Machine::new(cpu, memory);
    setup(&mut machine);
    machine
}

fn step_until<P, F>(machine: &mut Machine<P, MemoryMap>, max_cycles: usize, mut predicate: F)
where
    P: Processor<Error = CpuError> + CpuModel,
    F: FnMut(&Machine<P, MemoryMap>) -> bool,
{
    for _ in 0..max_cycles {
        machine.step_cycle().expect("program step should succeed");
        if predicate(machine) {
            return;
        }
    }

    panic!(
        "program on {} did not reach its stop condition in {max_cycles} cycles",
        machine.cpu().model_name()
    );
}

fn assert_store_load_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine(make_cpu(RESET_VECTOR), STORE_LOAD_PROGRAM);

    step_until(&mut machine, 12, |machine| {
        machine.cpu().hart_state().registers.read(3) == 9
    });

    assert_eq!(machine.cpu().hart_state().registers.read(3), 9);
    assert_eq!(
        machine
            .bus_mut()
            .load32(RAM_BASE)
            .expect("RAM word should remain readable"),
        9
    );
}

fn assert_count_loop_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine(make_cpu(RESET_VECTOR), COUNT_LOOP_PROGRAM);

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(2) == 3 && machine.cpu().hart_state().pc == 20
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 0);
    assert_eq!(machine.cpu().hart_state().registers.read(2), 3);
}

fn assert_msip_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine(make_cpu(RESET_VECTOR), MSIP_INTERRUPT_PROGRAM);

    step_until(&mut machine, 16, |machine| {
        machine.cpu().hart_state().pc == 0x14
    });
    machine
        .bus_mut()
        .store32(MSIP_BASE, 1)
        .expect("MSIP write should succeed");
    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(10) >= 1
    });
    machine
        .bus_mut()
        .store32(MSIP_BASE, 0)
        .expect("MSIP clear should succeed");
    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().pc == 0x14
    });

    assert!(machine.cpu().hart_state().registers.read(10) >= 1);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_0003
    );
}

fn assert_sv32_asid_switch_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_ASID_SWITCH_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_asid_switch_state,
    );

    step_until(&mut machine, 48, |machine| {
        machine.cpu().hart_state().registers.read(12) == VM_VALUE_A
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(11), VM_VALUE_B);
    assert_eq!(machine.cpu().hart_state().registers.read(12), VM_VALUE_A);
}

fn assert_sv32_sfence_remap_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_SFENCE_REMAP_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_sfence_remap_state,
    );

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(12) == VM_VALUE_B
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(11), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(12), VM_VALUE_B);
    assert_eq!(
        machine
            .bus_mut()
            .load32(VM_LEAF_TABLE_1 + (((VM_VIRTUAL_ADDR >> 12) & 0x3ff) as u64) * 4)
            .expect("remapped leaf pte should remain readable"),
        sv32_leaf_pte(VM_PHYS_PAGE_B as u32, PTE_R | PTE_W | PTE_A | PTE_D)
    );
}

fn assert_sv32_superpage_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_SUPERPAGE_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_superpage_state,
    );

    step_until(&mut machine, 48, |machine| {
        machine.cpu().hart_state().registers.read(11) == VM_SUPERPAGE_STORE_VALUE
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_B);
    assert_eq!(
        machine.cpu().hart_state().registers.read(11),
        VM_SUPERPAGE_STORE_VALUE
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(VM_SUPERPAGE_PHYSICAL_ADDR + 4)
            .expect("superpage store target should remain readable"),
        VM_SUPERPAGE_STORE_VALUE
    );
}

fn assert_sv32_global_asid_fence_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_GLOBAL_ASID_FENCE_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_global_asid_fence_state,
    );

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(12) == VM_VALUE_B
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(11), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(12), VM_VALUE_B);
    assert_eq!(
        machine
            .bus_mut()
            .load32(VM_LEAF_TABLE_1 + (((VM_VIRTUAL_ADDR >> 12) & 0x3ff) as u64) * 4)
            .expect("global leaf pte should remain readable"),
        sv32_leaf_pte(VM_PHYS_PAGE_B as u32, PTE_R | PTE_W | PTE_A | PTE_D)
    );
}

fn setup_sv32_asid_switch_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    write_word(machine, VM_PHYS_PAGE_B, VM_VALUE_B);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_VIRTUAL_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_W | PTE_A | PTE_D,
    );
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_2,
        VM_LEAF_TABLE_2,
        VM_VIRTUAL_ADDR,
        VM_PHYS_PAGE_B as u32,
        PTE_R | PTE_W | PTE_A | PTE_D,
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_VIRTUAL_ADDR);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(2, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(3, sv32_satp_with_asid(VM_ROOT_TABLE_2, 2));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn setup_sv32_sfence_remap_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let leaf_pte_addr = VM_LEAF_TABLE_1 + (((VM_VIRTUAL_ADDR >> 12) & 0x3ff) as u64) * 4;

    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    write_word(machine, VM_PHYS_PAGE_B, VM_VALUE_B);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_VIRTUAL_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_W | PTE_A | PTE_D,
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_VIRTUAL_ADDR);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(2, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(3, leaf_pte_addr as u32);
    machine.cpu_mut().hart_state_mut().registers.write(
        4,
        sv32_leaf_pte(VM_PHYS_PAGE_B as u32, PTE_R | PTE_W | PTE_A | PTE_D),
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(6, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(7, 1 << MSTATUS_MPP_SHIFT);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn setup_sv32_superpage_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_SUPERPAGE_PHYSICAL_ADDR, VM_VALUE_B);
    install_sv32_superpage_mapping(
        machine,
        VM_ROOT_TABLE_3,
        VM_SUPERPAGE_VIRTUAL_ADDR,
        RAM_BASE as u32,
        PTE_R | PTE_W | PTE_A | PTE_D,
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_SUPERPAGE_VIRTUAL_ADDR);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(2, sv32_satp_with_asid(VM_ROOT_TABLE_3, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(4, VM_SUPERPAGE_STORE_VALUE);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn setup_sv32_global_asid_fence_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let leaf_pte_addr = VM_LEAF_TABLE_1 + (((VM_VIRTUAL_ADDR >> 12) & 0x3ff) as u64) * 4;
    let global_flags = PTE_R | PTE_W | PTE_A | PTE_D | PTE_G;

    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    write_word(machine, VM_PHYS_PAGE_B, VM_VALUE_B);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_VIRTUAL_ADDR,
        VM_PHYS_PAGE_A as u32,
        global_flags,
    );
    write_word(
        machine,
        VM_ROOT_TABLE_2 + (((VM_VIRTUAL_ADDR >> 22) & 0x3ff) as u64) * 4,
        PTE_V | ((VM_LEAF_TABLE_1 as u32 >> PAGE_SHIFT) << 10),
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_VIRTUAL_ADDR);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(2, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(3, sv32_satp_with_asid(VM_ROOT_TABLE_2, 2));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(4, leaf_pte_addr as u32);
    machine.cpu_mut().hart_state_mut().registers.write(5, 2);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(6, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(7, 1 << MSTATUS_MPP_SHIFT);
    machine.cpu_mut().hart_state_mut().registers.write(
        8,
        sv32_leaf_pte(VM_PHYS_PAGE_B as u32, PTE_R | PTE_W | PTE_A | PTE_D),
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn write_word<P>(machine: &mut Machine<P, MemoryMap>, addr: u64, value: u32)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .bus_mut()
        .store32(addr, value)
        .expect("RAM write during test setup should succeed");
}

fn install_sv32_mapping<P>(
    machine: &mut Machine<P, MemoryMap>,
    root_table: u64,
    leaf_table: u64,
    virtual_address: u32,
    physical_address: u32,
    flags: u32,
) where
    P: Processor<Error = CpuError> + CpuModel,
{
    let root_index = ((virtual_address >> 22) & 0x3ff) as u64;
    let leaf_index = ((virtual_address >> 12) & 0x3ff) as u64;

    write_word(
        machine,
        root_table + root_index * 4,
        PTE_V | ((leaf_table as u32 >> PAGE_SHIFT) << 10),
    );
    write_word(
        machine,
        leaf_table + leaf_index * 4,
        sv32_leaf_pte(physical_address, flags),
    );
}

fn install_sv32_superpage_mapping<P>(
    machine: &mut Machine<P, MemoryMap>,
    root_table: u64,
    virtual_address: u32,
    physical_base: u32,
    flags: u32,
) where
    P: Processor<Error = CpuError> + CpuModel,
{
    let root_index = ((virtual_address >> 22) & 0x3ff) as u64;
    write_word(
        machine,
        root_table + root_index * 4,
        sv32_leaf_pte(physical_base, flags),
    );
}

const fn sv32_leaf_pte(physical_address: u32, flags: u32) -> u32 {
    PTE_V | flags | ((physical_address >> PAGE_SHIFT) << 10)
}

const fn sv32_satp_with_asid(root_table: u64, asid: u32) -> u32 {
    SATP_MODE_SV32 | (asid << 22) | ((root_table as u32) >> PAGE_SHIFT)
}

#[test]
fn reference_core_runs_store_load_program() {
    assert_store_load_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_store_load_program() {
    assert_store_load_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_count_loop_program() {
    assert_count_loop_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_count_loop_program() {
    assert_count_loop_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_msip_interrupt_program() {
    assert_msip_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_msip_interrupt_program() {
    assert_msip_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_asid_switch_program() {
    assert_sv32_asid_switch_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_asid_switch_program() {
    assert_sv32_asid_switch_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_sfence_remap_program() {
    assert_sv32_sfence_remap_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_sfence_remap_program() {
    assert_sv32_sfence_remap_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_superpage_program() {
    assert_sv32_superpage_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_superpage_program() {
    assert_sv32_superpage_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_global_asid_fence_program() {
    assert_sv32_global_asid_fence_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_global_asid_fence_program() {
    assert_sv32_global_asid_fence_program(PipelineCore::new);
}
