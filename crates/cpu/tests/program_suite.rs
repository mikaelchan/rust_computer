use std::{cell::RefCell, rc::Rc};

use rvsim_cpu::{CpuError, CpuModel, PipelineCore, PrivilegeMode, ReferenceCore};
use rvsim_devices::{
    BlockDevice, DmaController, InterruptController, MachineSoftwareInterrupt, Ram, Rom,
    SupervisorSoftwareInterrupt,
};
use rvsim_isa::CsrAddress;
use rvsim_system::{ArbiterBus, Bus, Machine, MemoryMap, Processor};

const RESET_VECTOR: u32 = 0;
const RAM_BASE: u64 = 0x1000_0000;
const RAM_BYTES: usize = 0x1000;
const VM_RAM_BYTES: usize = 0x8000;
const CONTROLLER_BASE: u64 = 0x4000_0000;
const MSIP_BASE: u64 = 0x5000_0000;
const SSIP_BASE: u64 = 0x6000_0000;
const BLOCK_BASE: u64 = 0x7000_0000;
const BLOCK_BYTES: usize = 16;
const DMA_BASE: u64 = 0x7000_0000;
const DMA_DEST_ADDR: u64 = RAM_BASE + 0x40;
const PAGE_SHIFT: u32 = 12;
const SATP_MODE_SV32: u32 = 1 << 31;
const MSTATUS_MPRV: u32 = 1 << 17;
const MSTATUS_MPP_SHIFT: u32 = 11;
const MSTATUS_TVM: u32 = 1 << 20;
const PTE_V: u32 = 1 << 0;
const PTE_R: u32 = 1 << 1;
const PTE_W: u32 = 1 << 2;
const PTE_U: u32 = 1 << 4;
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
const SV32_SUM_FAULT_PROGRAM: &str = include_str!("programs/sv32_sum_fault.hex");
const DELEGATED_USER_ECALL_PROGRAM: &str = include_str!("programs/delegated_user_ecall.hex");
const MACHINE_ECALL_RETURN_PROGRAM: &str = include_str!("programs/machine_ecall_return.hex");
const DELEGATED_SSIP_INTERRUPT_PROGRAM: &str =
    include_str!("programs/delegated_ssip_interrupt.hex");
const DELEGATED_EXTERNAL_INTERRUPT_PROGRAM: &str =
    include_str!("programs/delegated_external_interrupt.hex");
const DELEGATED_BLOCK_INTERRUPT_PROGRAM: &str =
    include_str!("programs/delegated_block_interrupt.hex");
const DELEGATED_DMA_INTERRUPT_PROGRAM: &str = include_str!("programs/delegated_dma_interrupt.hex");
const DELEGATED_ILLEGAL_INSTRUCTION_PROGRAM: &str =
    include_str!("programs/delegated_illegal_instruction.hex");
const SUPERVISOR_TVM_SATP_TRAP_PROGRAM: &str =
    include_str!("programs/supervisor_tvm_satp_trap.hex");

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
    build_machine_with_map(cpu, program_image, ram_bytes, |_| {}, setup)
}

fn build_machine_with_map<P, M, F>(
    cpu: P,
    program_image: &str,
    ram_bytes: usize,
    map_setup: M,
    setup: F,
) -> Machine<P, MemoryMap>
where
    P: Processor<Error = CpuError> + CpuModel,
    M: FnOnce(&mut MemoryMap),
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
        .map_device(InterruptController::new(CONTROLLER_BASE))
        .expect("interrupt controller should map");
    memory
        .map_device(MachineSoftwareInterrupt::new(MSIP_BASE))
        .expect("MSIP device should map");
    memory
        .map_device(SupervisorSoftwareInterrupt::new(SSIP_BASE))
        .expect("SSIP device should map");
    map_setup(&mut memory);

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

fn step_until_with_bus<P, B, F>(machine: &mut Machine<P, B>, max_cycles: usize, mut predicate: F)
where
    P: Processor<Error = CpuError> + CpuModel,
    B: Bus,
    F: FnMut(&Machine<P, B>) -> bool,
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

fn assert_sv32_sum_fault_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_SUM_FAULT_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_sum_fault_state,
    );

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(13) == 1 && machine.cpu().hart_state().pc == 0x8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 13);
    assert_eq!(
        machine.cpu().hart_state().registers.read(12),
        VM_VIRTUAL_ADDR
    );
    assert_eq!(machine.cpu().hart_state().registers.read(13), 1);
}

fn assert_delegated_user_ecall_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        DELEGATED_USER_ECALL_PROGRAM,
        RAM_BYTES,
        setup_delegated_user_ecall_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1
            && machine.cpu().hart_state().registers.read(2) == 7
            && machine.cpu().hart_state().pc == 0x8
            && matches!(machine.cpu().hart_state().privilege, PrivilegeMode::User)
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(2), 7);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Scause), 8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Sepc), 4);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_machine_ecall_return_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        MACHINE_ECALL_RETURN_PROGRAM,
        RAM_BYTES,
        setup_machine_ecall_return_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1
            && machine.cpu().hart_state().registers.read(2) == 9
            && machine.cpu().hart_state().pc == 0x8
            && matches!(machine.cpu().hart_state().privilege, PrivilegeMode::Machine)
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(2), 9);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 11);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 4);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_delegated_ssip_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        DELEGATED_SSIP_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_delegated_ssip_interrupt_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(1) == 5
            && machine.cpu().hart_state().registers.read(10) == 4
            && machine.cpu().hart_state().pc == 0x4
            && matches!(machine.cpu().hart_state().privilege, PrivilegeMode::User)
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 4);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Scause),
        0x8000_0001
    );
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Sepc), 0);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
    assert_eq!(
        machine
            .bus_mut()
            .load32(SSIP_BASE)
            .expect("SSIP register should remain readable"),
        0
    );
}

fn assert_delegated_external_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        DELEGATED_EXTERNAL_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_delegated_external_interrupt_state,
    );

    step_until(&mut machine, 40, |machine| {
        machine.cpu().hart_state().registers.read(1) == 5
            && machine.cpu().hart_state().registers.read(10) == 5
            && machine.cpu().hart_state().pc == 0x4
            && matches!(machine.cpu().hart_state().privilege, PrivilegeMode::User)
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 5);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Scause),
        0x8000_0009
    );
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Sepc), 0);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
    assert_eq!(
        machine
            .bus_mut()
            .load32(CONTROLLER_BASE)
            .expect("controller pending register should remain readable"),
        0
    );
}

fn assert_delegated_block_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with_map(
        make_cpu(RESET_VECTOR),
        DELEGATED_BLOCK_INTERRUPT_PROGRAM,
        RAM_BYTES,
        install_block_interrupt_device,
        setup_delegated_block_interrupt_state,
    );

    step_until(&mut machine, 48, |machine| {
        machine.cpu().hart_state().registers.read(1) == 5
            && machine.cpu().hart_state().registers.read(10) == 7
            && machine.cpu().hart_state().pc == 0x1c
            && matches!(machine.cpu().hart_state().privilege, PrivilegeMode::User)
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 7);
    assert_eq!(machine.cpu().hart_state().pc, 0x1c);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Scause),
        0x8000_0009
    );
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Sepc), 0x1c);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
    assert_eq!(
        machine
            .bus_mut()
            .load32(BLOCK_BASE + BlockDevice::CONTROL_OFFSET)
            .expect("block control register should remain readable")
            & BlockDevice::STATUS_DONE,
        0
    );
    assert_eq!(
        machine.bus_mut().pending_interrupts().highest_priority(),
        None
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(BLOCK_BASE + BlockDevice::DATA_WINDOW_OFFSET)
            .expect("block data word 0 should remain readable"),
        0x1122_3344
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(BLOCK_BASE + BlockDevice::DATA_WINDOW_OFFSET + 4)
            .expect("block data word 1 should remain readable"),
        0x5566_7788
    );
}

fn assert_delegated_dma_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_dma_machine(
        make_cpu(RESET_VECTOR),
        DELEGATED_DMA_INTERRUPT_PROGRAM,
        setup_delegated_dma_interrupt_state,
    );

    step_until_with_bus(&mut machine, 72, |machine| {
        machine.cpu().hart_state().registers.read(1) == 5
            && machine.cpu().hart_state().registers.read(10) == 6
            && machine.cpu().hart_state().pc == 0x30
            && matches!(machine.cpu().hart_state().privilege, PrivilegeMode::User)
            && machine
                .bus()
                .pending_interrupts()
                .highest_priority()
                .is_none()
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 6);
    assert_eq!(machine.cpu().hart_state().pc, 0x30);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Scause),
        0x8000_0009
    );
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Sepc), 0x30);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
    assert_eq!(
        machine
            .bus_mut()
            .load32(DMA_BASE + DmaController::CONTROL_OFFSET)
            .expect("dma control register should remain readable")
            & DmaController::STATUS_DONE,
        0
    );
    assert_eq!(machine.bus().pending_interrupts().highest_priority(), None);
    assert_eq!(
        machine
            .bus_mut()
            .load32(DMA_DEST_ADDR)
            .expect("copied word 0 should remain readable"),
        0x1122_3344
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(DMA_DEST_ADDR + 4)
            .expect("copied word 1 should remain readable"),
        0x5566_7788
    );
}

fn assert_delegated_illegal_instruction_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    const ILLEGAL_MSTATUS_CSRRWI: u32 = 0x3000_d0f3;

    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        DELEGATED_ILLEGAL_INSTRUCTION_PROGRAM,
        RAM_BYTES,
        setup_delegated_illegal_instruction_state,
    );

    step_until(&mut machine, 40, |machine| {
        machine.cpu().hart_state().registers.read(13) == 1
            && machine.cpu().hart_state().pc == 0x8
            && matches!(machine.cpu().hart_state().privilege, PrivilegeMode::User)
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 0);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 2);
    assert_eq!(
        machine.cpu().hart_state().registers.read(12),
        ILLEGAL_MSTATUS_CSRRWI
    );
    assert_eq!(machine.cpu().hart_state().registers.read(13), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Scause), 2);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Stval),
        ILLEGAL_MSTATUS_CSRRWI
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_supervisor_tvm_satp_trap_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    const SUPERVISOR_SATP_CSRRW: u32 = 0x1800_10f3;

    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SUPERVISOR_TVM_SATP_TRAP_PROGRAM,
        RAM_BYTES,
        setup_supervisor_tvm_satp_trap_state,
    );

    step_until(&mut machine, 40, |machine| {
        machine.cpu().hart_state().registers.read(13) == 1
            && machine.cpu().hart_state().pc == 0x8
            && matches!(
                machine.cpu().hart_state().privilege,
                PrivilegeMode::Supervisor
            )
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 0);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 2);
    assert_eq!(
        machine.cpu().hart_state().registers.read(12),
        SUPERVISOR_SATP_CSRRW
    );
    assert_eq!(machine.cpu().hart_state().registers.read(13), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 2);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        SUPERVISOR_SATP_CSRRW
    );
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Satp), 0);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
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

fn setup_sv32_sum_fault_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_VIRTUAL_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_U | PTE_R | PTE_W | PTE_A | PTE_D,
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_VIRTUAL_ADDR);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn setup_delegated_user_ecall_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Medeleg, 1 << 8);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Stvec, 0x20);
}

fn setup_machine_ecall_return_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
}

fn setup_delegated_ssip_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .bus_mut()
        .store32(SSIP_BASE, 1)
        .expect("SSIP register should write during setup");
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mideleg, 1 << 1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sie, 1 << 1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Stvec, 0x20);
}

fn setup_delegated_external_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .bus_mut()
        .store32(CONTROLLER_BASE + 4, 1)
        .expect("controller enable register should write during setup");
    machine
        .bus_mut()
        .store32(CONTROLLER_BASE + 16, 1)
        .expect("controller route register should write during setup");
    machine
        .bus_mut()
        .store32(CONTROLLER_BASE + 8, 1)
        .expect("controller pending register should write during setup");
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mideleg, 1 << 9);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sie, 1 << 9);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Stvec, 0x20);
}

fn install_block_interrupt_device(memory: &mut MemoryMap) {
    let mut block_device = BlockDevice::new(BLOCK_BASE, 4, BLOCK_BYTES, 2);
    block_device
        .write_block_contents(
            1,
            &[
                0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        )
        .expect("block image should load");
    memory
        .map_device(block_device)
        .expect("block device should map");
}

fn setup_delegated_block_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mideleg, 1 << 9);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sie, 1 << 9);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Stvec, 0x20);
}

fn build_dma_machine<P, F>(
    cpu: P,
    program_image: &str,
    setup: F,
) -> Machine<P, ArbiterBus<MemoryMap>>
where
    P: Processor<Error = CpuError> + CpuModel,
    F: FnOnce(&mut Machine<P, ArbiterBus<MemoryMap>>),
{
    let words = parse_program_image(program_image);
    let dma = Rc::new(RefCell::new(DmaController::new(DMA_BASE)));
    let mut memory = MemoryMap::new();
    memory
        .map_device(Rom::from_words(RESET_VECTOR as u64, &words))
        .expect("ROM should map");
    memory
        .map_device(Ram::new(RAM_BASE, RAM_BYTES))
        .expect("RAM should map");
    memory
        .map_shared_device(Rc::clone(&dma))
        .expect("DMA device should map");

    let mut bus = ArbiterBus::new(memory);
    bus.add_shared_master(Rc::clone(&dma));
    bus.store32(RAM_BASE, 0x1122_3344)
        .expect("DMA source word 0 should write during setup");
    bus.store32(RAM_BASE + 4, 0x5566_7788)
        .expect("DMA source word 1 should write during setup");

    let mut machine = Machine::new(cpu, bus);
    setup(&mut machine);
    machine
}

fn setup_delegated_dma_interrupt_state<P>(machine: &mut Machine<P, ArbiterBus<MemoryMap>>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mideleg, 1 << 9);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sie, 1 << 9);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Stvec, 0x40);
}

fn setup_delegated_illegal_instruction_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Medeleg, 1 << 2);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Stvec, 0x20);
}

fn setup_supervisor_tvm_satp_trap_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Supervisor;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_TVM);
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

#[test]
fn reference_core_runs_sv32_sum_fault_program() {
    assert_sv32_sum_fault_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_sum_fault_program() {
    assert_sv32_sum_fault_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_delegated_user_ecall_program() {
    assert_delegated_user_ecall_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_delegated_user_ecall_program() {
    assert_delegated_user_ecall_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_ecall_return_program() {
    assert_machine_ecall_return_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_ecall_return_program() {
    assert_machine_ecall_return_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_delegated_ssip_interrupt_program() {
    assert_delegated_ssip_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_delegated_ssip_interrupt_program() {
    assert_delegated_ssip_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_delegated_external_interrupt_program() {
    assert_delegated_external_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_delegated_external_interrupt_program() {
    assert_delegated_external_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_delegated_block_interrupt_program() {
    assert_delegated_block_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_delegated_block_interrupt_program() {
    assert_delegated_block_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_delegated_dma_interrupt_program() {
    assert_delegated_dma_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_delegated_dma_interrupt_program() {
    assert_delegated_dma_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_delegated_illegal_instruction_program() {
    assert_delegated_illegal_instruction_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_delegated_illegal_instruction_program() {
    assert_delegated_illegal_instruction_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_supervisor_tvm_satp_trap_program() {
    assert_supervisor_tvm_satp_trap_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_supervisor_tvm_satp_trap_program() {
    assert_supervisor_tvm_satp_trap_program(PipelineCore::new);
}
