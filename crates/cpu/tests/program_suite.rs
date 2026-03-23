use std::{cell::RefCell, rc::Rc};

use rvsim_cpu::{CpuError, CpuModel, PipelineCore, PrivilegeMode, ReferenceCore};
use rvsim_devices::{
    BlockDevice, DmaController, InterruptController, MachineSoftwareInterrupt, MachineTimer, Ram,
    Rom, SupervisorSoftwareInterrupt,
};
use rvsim_isa::CsrAddress;
use rvsim_system::{ArbiterBus, Bus, Machine, MemoryMap, Processor};

const RESET_VECTOR: u32 = 0;
const RAM_BASE: u64 = 0x1000_0000;
const RAM_BYTES: usize = 0x1000;
const VM_RAM_BYTES: usize = 0x10000;
const TIMER_BASE: u64 = 0x3000_0000;
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
const MSTATUS_SUM: u32 = 1 << 18;
const MSTATUS_MXR: u32 = 1 << 19;
const MSTATUS_MPP_SHIFT: u32 = 11;
const MSTATUS_TVM: u32 = 1 << 20;
const MSTATUS_TW: u32 = 1 << 21;
const MSTATUS_TSR: u32 = 1 << 22;
const PTE_V: u32 = 1 << 0;
const PTE_R: u32 = 1 << 1;
const PTE_W: u32 = 1 << 2;
const PTE_X: u32 = 1 << 3;
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
const VM_PHYS_PAGE_C: u64 = RAM_BASE + 0x7000;
const VM_PHYS_PAGE_D: u64 = RAM_BASE + 0x8000;
const VM_VIRTUAL_ADDR: u32 = 0x4000;
const VM_TRANSLATED_LOAD_ADDR: u32 = 0x8000;
const VM_TRANSLATED_LOAD_ADDR_2: u32 = 0x9000;
const VM_VALUE_A: u32 = 0x1111_2222;
const VM_VALUE_B: u32 = 0x3333_4444;
const VM_VALUE_C: u32 = 0x7777_8888;
const VM_VALUE_D: u32 = 0x9999_AAAA;
const VM_SUPERPAGE_FETCH_VIRTUAL_BASE: u32 = 0x0040_0000;
const VM_SUPERPAGE_FETCH_DATA_PHYSICAL_ADDR: u64 = RAM_BASE + 0x8000;
const VM_SUPERPAGE_VIRTUAL_ADDR: u32 = 0x0040_5000;
const VM_SUPERPAGE_PHYSICAL_ADDR: u64 = RAM_BASE + 0x5000;
const VM_SUPERPAGE_STORE_VALUE: u32 = 0x5555_6666;

const STORE_LOAD_PROGRAM: &str = include_str!("programs/store_load.hex");
const COUNT_LOOP_PROGRAM: &str = include_str!("programs/count_loop.hex");
const MSIP_INTERRUPT_PROGRAM: &str = include_str!("programs/msip_interrupt.hex");
const WFI_MACHINE_INTERRUPT_PROGRAM: &str = include_str!("programs/wfi_machine_interrupt.hex");
const VECTORED_MACHINE_SOFTWARE_INTERRUPT_PROGRAM: &str =
    include_str!("programs/vectored_machine_software_interrupt.hex");
const VECTORED_SUPERVISOR_EXTERNAL_INTERRUPT_PROGRAM: &str =
    include_str!("programs/vectored_supervisor_external_interrupt.hex");
const MACHINE_EXTERNAL_INTERRUPT_PROGRAM: &str =
    include_str!("programs/machine_external_interrupt.hex");
const MACHINE_INTERRUPT_PRIORITY_PROGRAM: &str =
    include_str!("programs/machine_interrupt_priority.hex");
const SV32_AD_BITS_PROGRAM: &str = include_str!("programs/sv32_ad_bits.hex");
const SV32_MPRV_TRANSLATED_LOAD_PROGRAM: &str =
    include_str!("programs/sv32_mprv_translated_load.hex");
const SV32_SELECTIVE_SFENCE_PROGRAM: &str = include_str!("programs/sv32_selective_sfence.hex");
const SV32_INSTRUCTION_PAGE_FAULT_PROGRAM: &str =
    include_str!("programs/sv32_instruction_page_fault.hex");
const SV32_MALFORMED_NONLEAF_PROGRAM: &str = include_str!("programs/sv32_malformed_nonleaf.hex");
const SV32_MALFORMED_SUPERPAGE_PROGRAM: &str =
    include_str!("programs/sv32_malformed_superpage.hex");
const SV32_SUPERPAGE_FETCH_PROGRAM: &str = include_str!("programs/sv32_superpage_fetch.hex");
const SV32_SATP_NAMESPACE_PRESERVE_PROGRAM: &str =
    include_str!("programs/sv32_satp_namespace_preserve.hex");
const CSR_MIP_MACHINE_SOFTWARE_INTERRUPT_PROGRAM: &str =
    include_str!("programs/csr_mip_machine_software_interrupt.hex");
const CSR_SIP_SUPERVISOR_SOFTWARE_INTERRUPT_PROGRAM: &str =
    include_str!("programs/csr_sip_supervisor_software_interrupt.hex");
const MACHINE_TIMER_INTERRUPT_PROGRAM: &str = include_str!("programs/machine_timer_interrupt.hex");
const MACHINE_PREEMPTS_SUPERVISOR_HANDLER_PROGRAM: &str =
    include_str!("programs/machine_preempts_supervisor_handler.hex");
const MACHINE_NESTED_INTERRUPT_PROGRAM: &str =
    include_str!("programs/machine_nested_interrupt.hex");
const SUPERVISOR_NESTED_INTERRUPT_PROGRAM: &str =
    include_str!("programs/supervisor_nested_interrupt.hex");
const SV32_ASID_SWITCH_PROGRAM: &str = include_str!("programs/sv32_asid_switch.hex");
const SV32_SFENCE_REMAP_PROGRAM: &str = include_str!("programs/sv32_sfence_remap.hex");
const SV32_SUPERPAGE_PROGRAM: &str = include_str!("programs/sv32_superpage.hex");
const SV32_GLOBAL_ASID_FENCE_PROGRAM: &str = include_str!("programs/sv32_global_asid_fence.hex");
const SV32_ASID_ADDRESS_FENCE_PROGRAM: &str = include_str!("programs/sv32_asid_address_fence.hex");
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
const SUPERVISOR_TVM_SFENCE_TRAP_PROGRAM: &str =
    include_str!("programs/supervisor_tvm_sfence_trap.hex");
const SUPERVISOR_TSR_SRET_TRAP_PROGRAM: &str =
    include_str!("programs/supervisor_tsr_sret_trap.hex");
const SUPERVISOR_TW_WFI_TRAP_PROGRAM: &str = include_str!("programs/supervisor_tw_wfi_trap.hex");
const MACHINE_MCYCLEH_ACCESS_PROGRAM: &str = include_str!("programs/machine_mcycleh_access.hex");
const INSTRET_SHADOW_WRITE_TRAP_PROGRAM: &str =
    include_str!("programs/instret_shadow_write_trap.hex");
const USER_MACHINE_CSR_TRAP_PROGRAM: &str = include_str!("programs/user_machine_csr_trap.hex");
const USER_INSTRET_COUNTEREN_TRAP_PROGRAM: &str =
    include_str!("programs/user_instret_counteren_trap.hex");
const USER_CYCLEH_COUNTEREN_ENABLED_PROGRAM: &str =
    include_str!("programs/user_cycleh_counteren_enabled.hex");
const USER_TIMEH_COUNTEREN_ENABLED_PROGRAM: &str =
    include_str!("programs/user_timeh_counteren_enabled.hex");
const USER_INSTRET_COUNTEREN_ENABLED_PROGRAM: &str =
    include_str!("programs/user_instret_counteren_enabled.hex");
const SV32_USER_PAGE_LOAD_PROGRAM: &str = include_str!("programs/sv32_user_page_load.hex");
const SV32_EXECUTE_ONLY_LOAD_PROGRAM: &str = include_str!("programs/sv32_execute_only_load.hex");

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

fn assert_wfi_machine_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        WFI_MACHINE_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_wfi_machine_interrupt_state,
    );

    step_until(&mut machine, 16, |machine| {
        machine.cpu().hart_state().halted
    });

    assert!(machine.cpu().hart_state().halted);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 0);

    machine
        .bus_mut()
        .store32(MSIP_BASE, 1)
        .expect("MSIP write should succeed");

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1
            && machine.cpu().hart_state().registers.read(10) == 7
            && machine.cpu().hart_state().pc == 0x8
    });

    assert!(!machine.cpu().hart_state().halted);
    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 7);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_0003
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(MSIP_BASE)
            .expect("MSIP register should remain readable"),
        0
    );
}

fn assert_vectored_machine_software_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        VECTORED_MACHINE_SOFTWARE_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_vectored_machine_software_interrupt_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(10) == 3 && machine.cpu().hart_state().pc == 0x0
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 3);
    assert_eq!(machine.cpu().hart_state().pc, 0x0);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 0x0);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_0003
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(MSIP_BASE)
            .expect("MSIP register should remain readable"),
        0
    );
}

fn assert_vectored_supervisor_external_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        VECTORED_SUPERVISOR_EXTERNAL_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_vectored_supervisor_external_interrupt_state,
    );

    step_until(&mut machine, 40, |machine| {
        machine.cpu().hart_state().registers.read(10) == 9
            && machine.cpu().hart_state().pc == 0x0
            && matches!(machine.cpu().hart_state().privilege, PrivilegeMode::User)
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 9);
    assert_eq!(machine.cpu().hart_state().pc, 0x0);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Sepc), 0x0);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Scause),
        0x8000_0009
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(CONTROLLER_BASE)
            .expect("controller pending register should remain readable"),
        0
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_machine_external_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        MACHINE_EXTERNAL_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_machine_external_interrupt_state,
    );

    step_until(&mut machine, 40, |machine| {
        machine.cpu().hart_state().registers.read(1) == 5
            && machine.cpu().hart_state().registers.read(10) == 2
            && machine.cpu().hart_state().pc == 0x4
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 5);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 2);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 0x0);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_000b
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(CONTROLLER_BASE)
            .expect("controller pending register should remain readable"),
        0
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_machine_interrupt_priority_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with_map(
        make_cpu(RESET_VECTOR),
        MACHINE_INTERRUPT_PRIORITY_PROGRAM,
        RAM_BYTES,
        install_machine_timer_device,
        setup_machine_interrupt_priority_state,
    );

    step_until(&mut machine, 48, |machine| {
        machine.cpu().hart_state().registers.read(10) == 11 && machine.cpu().hart_state().pc == 0x0
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 11);
    assert_eq!(machine.cpu().hart_state().pc, 0x0);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 0x0);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_000b
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(CONTROLLER_BASE)
            .expect("controller pending register should remain readable"),
        0
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(MSIP_BASE)
            .expect("MSIP register should remain readable"),
        0
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(TIMER_BASE + 8)
            .expect("mtimecmp low should remain readable"),
        256
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_csr_mip_machine_software_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        CSR_MIP_MACHINE_SOFTWARE_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_csr_mip_machine_software_interrupt_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1
            && machine.cpu().hart_state().registers.read(10) == 9
            && machine.cpu().hart_state().pc == 0x8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 9);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_0003
    );
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mip) & (1 << 3),
        0
    );
}

fn assert_csr_sip_supervisor_software_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        CSR_SIP_SUPERVISOR_SOFTWARE_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_csr_sip_supervisor_software_interrupt_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1
            && machine.cpu().hart_state().registers.read(10) == 6
            && machine.cpu().hart_state().pc == 0x8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 6);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Sepc), 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Scause),
        0x8000_0001
    );
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Sip) & (1 << 1),
        0
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
}

fn assert_machine_timer_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with_map(
        make_cpu(RESET_VECTOR),
        MACHINE_TIMER_INTERRUPT_PROGRAM,
        RAM_BYTES,
        install_machine_timer_device,
        setup_machine_timer_interrupt_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(10) == 1 && machine.cpu().hart_state().pc == 0x0
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x0);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_0007
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(TIMER_BASE + 8)
            .expect("mtimecmp low should remain readable"),
        32
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(TIMER_BASE + 12)
            .expect("mtimecmp high should remain readable"),
        0
    );
}

fn assert_machine_preempts_supervisor_handler_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        MACHINE_PREEMPTS_SUPERVISOR_HANDLER_PROGRAM,
        RAM_BYTES,
        setup_machine_preempts_supervisor_handler_state,
    );

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1 && machine.cpu().hart_state().pc == 0x4
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 10);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 11);
    assert_eq!(machine.cpu().hart_state().registers.read(12), 12);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Scause),
        0x8000_0009
    );
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_0003
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(CONTROLLER_BASE)
            .expect("controller pending register should remain readable"),
        0
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(MSIP_BASE)
            .expect("MSIP register should remain readable"),
        0
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_machine_nested_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with_map(
        make_cpu(RESET_VECTOR),
        MACHINE_NESTED_INTERRUPT_PROGRAM,
        RAM_BYTES,
        install_machine_timer_device,
        setup_machine_nested_interrupt_state,
    );

    step_until(&mut machine, 96, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1 && machine.cpu().hart_state().pc == 0x4
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 10);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 11);
    assert_eq!(machine.cpu().hart_state().registers.read(12), 12);
    assert_eq!(machine.cpu().hart_state().registers.read(21), 21);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause),
        0x8000_0003
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(TIMER_BASE + 8)
            .expect("mtimecmp low should remain readable"),
        256
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(MSIP_BASE)
            .expect("MSIP register should remain readable"),
        0
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_supervisor_nested_interrupt_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SUPERVISOR_NESTED_INTERRUPT_PROGRAM,
        RAM_BYTES,
        setup_supervisor_nested_interrupt_state,
    );

    step_until(&mut machine, 96, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1 && machine.cpu().hart_state().pc == 0x4
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 10);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 11);
    assert_eq!(machine.cpu().hart_state().registers.read(12), 12);
    assert_eq!(machine.cpu().hart_state().registers.read(21), 21);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Scause),
        0x8000_0001
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(CONTROLLER_BASE)
            .expect("controller pending register should remain readable"),
        0
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(SSIP_BASE)
            .expect("SSIP register should remain readable"),
        0
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
}

fn assert_sv32_ad_bits_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_AD_BITS_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_ad_bits_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(10) == 9 && machine.cpu().hart_state().pc == 0xc
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 9);
    assert_eq!(
        machine
            .bus_mut()
            .load32(VM_PHYS_PAGE_A)
            .expect("translated store target should remain readable"),
        9
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(sv32_leaf_pte_addr(VM_LEAF_TABLE_1, VM_TRANSLATED_LOAD_ADDR))
            .expect("leaf pte should remain readable")
            & (PTE_A | PTE_D),
        PTE_A | PTE_D
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_machine_mprv_translation_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_MPRV_TRANSLATED_LOAD_PROGRAM,
        VM_RAM_BYTES,
        setup_machine_mprv_translation_state,
    );

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(10) == VM_VALUE_A
            && machine.cpu().hart_state().pc == 0x8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_sv32_selective_sfence_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_SELECTIVE_SFENCE_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_selective_sfence_state,
    );

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(13) == VM_VALUE_B
            && machine.cpu().hart_state().pc == 0x28
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(11), VM_VALUE_B);
    assert_eq!(machine.cpu().hart_state().registers.read(12), VM_VALUE_C);
    assert_eq!(machine.cpu().hart_state().registers.read(13), VM_VALUE_B);
    assert_eq!(
        machine
            .bus_mut()
            .load32(sv32_leaf_pte_addr(VM_LEAF_TABLE_1, VM_TRANSLATED_LOAD_ADDR))
            .expect("first remapped leaf pte should remain readable"),
        sv32_leaf_pte(VM_PHYS_PAGE_C as u32, PTE_R | PTE_A | PTE_D)
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(sv32_leaf_pte_addr(
                VM_LEAF_TABLE_1,
                VM_TRANSLATED_LOAD_ADDR_2
            ))
            .expect("second remapped leaf pte should remain readable"),
        sv32_leaf_pte(VM_PHYS_PAGE_D as u32, PTE_R | PTE_A | PTE_D)
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_sv32_instruction_page_fault_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(VM_VIRTUAL_ADDR),
        SV32_INSTRUCTION_PAGE_FAULT_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_instruction_page_fault_state,
    );

    step_until(&mut machine, 8, |machine| {
        machine.cpu().hart_state().registers.read(10) == 1 && machine.cpu().hart_state().pc == 0x84
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x84);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 12);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mepc),
        VM_VIRTUAL_ADDR
    );
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        VM_VIRTUAL_ADDR
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_sv32_malformed_nonleaf_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_MALFORMED_NONLEAF_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_malformed_nonleaf_state,
    );

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(10) == 1 && machine.cpu().hart_state().pc == 0x24
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(2), 0);
    assert_eq!(machine.cpu().hart_state().pc, 0x24);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 13);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        VM_TRANSLATED_LOAD_ADDR
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_sv32_malformed_superpage_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_MALFORMED_SUPERPAGE_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_malformed_superpage_state,
    );

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(10) == 1 && machine.cpu().hart_state().pc == 0x24
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(2), 0);
    assert_eq!(machine.cpu().hart_state().pc, 0x24);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 13);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 0x4);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        VM_SUPERPAGE_VIRTUAL_ADDR
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_sv32_superpage_fetch_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let program_words = parse_program_image(SV32_SUPERPAGE_FETCH_PROGRAM);
    let mut machine = build_machine_with(
        make_cpu(VM_SUPERPAGE_FETCH_VIRTUAL_BASE),
        SV32_SUPERPAGE_FETCH_PROGRAM,
        VM_RAM_BYTES,
        move |machine| setup_sv32_superpage_fetch_state(machine, &program_words),
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(10) == 9
            && machine.cpu().hart_state().pc == VM_SUPERPAGE_FETCH_VIRTUAL_BASE + 0x10
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 9);
    assert_eq!(
        machine.cpu().hart_state().pc,
        VM_SUPERPAGE_FETCH_VIRTUAL_BASE + 0x10
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(VM_SUPERPAGE_FETCH_DATA_PHYSICAL_ADDR)
            .expect("superpage fetch/data store target should remain readable"),
        9
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
}

fn assert_sv32_satp_namespace_preserve_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_SATP_NAMESPACE_PRESERVE_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_satp_namespace_preserve_state,
    );

    step_until(&mut machine, 64, |machine| {
        machine.cpu().hart_state().registers.read(13) == VM_VALUE_C
            && machine.cpu().hart_state().pc == 0x2c
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(11), VM_VALUE_B);
    assert_eq!(machine.cpu().hart_state().registers.read(12), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(13), VM_VALUE_C);
    assert_eq!(
        machine
            .bus_mut()
            .load32(sv32_leaf_pte_addr(VM_LEAF_TABLE_1, VM_TRANSLATED_LOAD_ADDR))
            .expect("updated ASID 1 leaf pte should remain readable"),
        sv32_leaf_pte(VM_PHYS_PAGE_C as u32, PTE_R | PTE_A | PTE_D)
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
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

fn assert_sv32_asid_address_fence_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_ASID_ADDRESS_FENCE_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_asid_address_fence_state,
    );

    step_until(&mut machine, 96, |machine| {
        machine.cpu().hart_state().registers.read(15) == VM_VALUE_D
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().registers.read(11), VM_VALUE_B);
    assert_eq!(machine.cpu().hart_state().registers.read(12), VM_VALUE_C);
    assert_eq!(machine.cpu().hart_state().registers.read(13), VM_VALUE_B);
    assert_eq!(machine.cpu().hart_state().registers.read(15), VM_VALUE_D);
    assert_eq!(
        machine
            .bus_mut()
            .load32(sv32_leaf_pte_addr(VM_LEAF_TABLE_1, VM_VIRTUAL_ADDR))
            .expect("asid 1 leaf pte should remain readable"),
        sv32_leaf_pte(VM_PHYS_PAGE_C as u32, PTE_R | PTE_W | PTE_A | PTE_D)
    );
    assert_eq!(
        machine
            .bus_mut()
            .load32(sv32_leaf_pte_addr(VM_LEAF_TABLE_2, VM_VIRTUAL_ADDR))
            .expect("asid 2 leaf pte should remain readable"),
        sv32_leaf_pte(VM_PHYS_PAGE_D as u32, PTE_R | PTE_W | PTE_A | PTE_D)
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

fn assert_sv32_supervisor_sum_load_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(VM_VIRTUAL_ADDR),
        SV32_USER_PAGE_LOAD_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_supervisor_sum_load_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(10) == VM_VALUE_A
            && machine.cpu().hart_state().pc == VM_VIRTUAL_ADDR + 8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().pc, VM_VIRTUAL_ADDR + 8);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
}

fn assert_sv32_supervisor_mxr_load_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(VM_VIRTUAL_ADDR),
        SV32_EXECUTE_ONLY_LOAD_PROGRAM,
        VM_RAM_BYTES,
        setup_sv32_supervisor_mxr_load_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().registers.read(10) == VM_VALUE_A
            && machine.cpu().hart_state().pc == VM_VIRTUAL_ADDR + 8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().pc, VM_VIRTUAL_ADDR + 8);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
}

fn assert_machine_mprv_supervisor_sum_load_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_USER_PAGE_LOAD_PROGRAM,
        VM_RAM_BYTES,
        setup_machine_mprv_supervisor_sum_load_state,
    );

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(10) == VM_VALUE_A
            && machine.cpu().hart_state().pc == 0x8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_machine_mprv_supervisor_mxr_load_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SV32_EXECUTE_ONLY_LOAD_PROGRAM,
        VM_RAM_BYTES,
        setup_machine_mprv_supervisor_mxr_load_state,
    );

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(10) == VM_VALUE_A
            && machine.cpu().hart_state().pc == 0x8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), VM_VALUE_A);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
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

fn assert_supervisor_tvm_sfence_trap_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    const SUPERVISOR_SFENCE_VMA: u32 = 0x1200_0073;

    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SUPERVISOR_TVM_SFENCE_TRAP_PROGRAM,
        RAM_BYTES,
        setup_supervisor_tvm_sfence_trap_state,
    );

    step_until(&mut machine, 40, |machine| {
        machine.cpu().hart_state().registers.read(13) == 1
            && machine.cpu().hart_state().pc == 0x8
            && matches!(
                machine.cpu().hart_state().privilege,
                PrivilegeMode::Supervisor
            )
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 2);
    assert_eq!(
        machine.cpu().hart_state().registers.read(12),
        SUPERVISOR_SFENCE_VMA
    );
    assert_eq!(machine.cpu().hart_state().registers.read(13), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 2);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        SUPERVISOR_SFENCE_VMA
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
}

fn assert_supervisor_tsr_sret_trap_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    const SUPERVISOR_SRET: u32 = 0x1020_0073;

    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SUPERVISOR_TSR_SRET_TRAP_PROGRAM,
        RAM_BYTES,
        setup_supervisor_tsr_sret_trap_state,
    );

    step_until(&mut machine, 40, |machine| {
        machine.cpu().hart_state().registers.read(13) == 1
            && machine.cpu().hart_state().pc == 0x8
            && matches!(
                machine.cpu().hart_state().privilege,
                PrivilegeMode::Supervisor
            )
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 2);
    assert_eq!(
        machine.cpu().hart_state().registers.read(12),
        SUPERVISOR_SRET
    );
    assert_eq!(machine.cpu().hart_state().registers.read(13), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 2);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        SUPERVISOR_SRET
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
}

fn assert_supervisor_tw_wfi_trap_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    const SUPERVISOR_WFI: u32 = 0x1050_0073;

    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        SUPERVISOR_TW_WFI_TRAP_PROGRAM,
        RAM_BYTES,
        setup_supervisor_tw_wfi_trap_state,
    );

    step_until(&mut machine, 40, |machine| {
        machine.cpu().hart_state().registers.read(13) == 1
            && machine.cpu().hart_state().pc == 0x8
            && matches!(
                machine.cpu().hart_state().privilege,
                PrivilegeMode::Supervisor
            )
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().registers.read(11), 2);
    assert_eq!(
        machine.cpu().hart_state().registers.read(12),
        SUPERVISOR_WFI
    );
    assert_eq!(machine.cpu().hart_state().registers.read(13), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 2);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        SUPERVISOR_WFI
    );
    assert!(!machine.cpu().hart_state().halted);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Supervisor
    ));
}

fn assert_user_machine_csr_trap_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    const USER_MACHINE_CSR_INSTRUCTION: u32 = 0x3000_d0f3;

    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        USER_MACHINE_CSR_TRAP_PROGRAM,
        RAM_BYTES,
        setup_user_machine_csr_trap_state,
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
        USER_MACHINE_CSR_INSTRUCTION
    );
    assert_eq!(machine.cpu().hart_state().registers.read(13), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 2);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        USER_MACHINE_CSR_INSTRUCTION
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_user_instret_counteren_trap_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    const USER_INSTRET_READ_INSTRUCTION: u32 = 0xc020_20f3;

    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        USER_INSTRET_COUNTEREN_TRAP_PROGRAM,
        RAM_BYTES,
        setup_user_instret_counteren_trap_state,
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
        USER_INSTRET_READ_INSTRUCTION
    );
    assert_eq!(machine.cpu().hart_state().registers.read(13), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 2);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        USER_INSTRET_READ_INSTRUCTION
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_user_cycleh_counteren_enabled_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        USER_CYCLEH_COUNTEREN_ENABLED_PROGRAM,
        RAM_BYTES,
        setup_user_cycleh_counteren_enabled_state,
    );

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1 && machine.cpu().hart_state().pc == 0x4
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Cycleh), 1);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_user_timeh_counteren_enabled_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with_map(
        make_cpu(RESET_VECTOR),
        USER_TIMEH_COUNTEREN_ENABLED_PROGRAM,
        RAM_BYTES,
        install_machine_timer_device,
        setup_user_timeh_counteren_enabled_state,
    );

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(1) == 1 && machine.cpu().hart_state().pc == 0x4
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x4);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Timeh), 1);
    assert_eq!(
        machine
            .bus_mut()
            .load32(TIMER_BASE + 4)
            .expect("mtime high should remain readable"),
        1
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_machine_mcycleh_access_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine(make_cpu(RESET_VECTOR), MACHINE_MCYCLEH_ACCESS_PROGRAM);

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(10) == 1 && machine.cpu().hart_state().pc == 0x8
    });

    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x8);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcycleh), 1);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
    ));
}

fn assert_user_instret_counteren_enabled_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        USER_INSTRET_COUNTEREN_ENABLED_PROGRAM,
        RAM_BYTES,
        setup_user_instret_counteren_enabled_state,
    );

    step_until(&mut machine, 32, |machine| {
        machine.cpu().hart_state().halted
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 0);
    assert_eq!(machine.cpu().hart_state().registers.read(2), 7);
    assert_eq!(machine.cpu().hart_state().registers.read(3), 2);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Minstret),
        4
    );
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Instret), 4);
    assert!(machine.cpu().hart_state().halted);
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::User
    ));
}

fn assert_instret_shadow_write_trap_program<P>(make_cpu: fn(u32) -> P)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    const INSTRET_SHADOW_WRITE_INSTRUCTION: u32 = 0xc020_10f3;

    let mut machine = build_machine_with(
        make_cpu(RESET_VECTOR),
        INSTRET_SHADOW_WRITE_TRAP_PROGRAM,
        RAM_BYTES,
        setup_instret_shadow_write_trap_state,
    );

    step_until(&mut machine, 24, |machine| {
        machine.cpu().hart_state().registers.read(10) == 1 && machine.cpu().hart_state().pc == 0x24
    });

    assert_eq!(machine.cpu().hart_state().registers.read(1), 0);
    assert_eq!(machine.cpu().hart_state().registers.read(10), 1);
    assert_eq!(machine.cpu().hart_state().pc, 0x24);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mcause), 2);
    assert_eq!(machine.cpu().hart_state().csrs.read(CsrAddress::Mepc), 0);
    assert_eq!(
        machine.cpu().hart_state().csrs.read(CsrAddress::Mtval),
        INSTRET_SHADOW_WRITE_INSTRUCTION
    );
    assert!(matches!(
        machine.cpu().hart_state().privilege,
        PrivilegeMode::Machine
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

fn setup_sv32_ad_bits_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_W,
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_TRANSLATED_LOAD_ADDR);
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Machine;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn setup_machine_mprv_translation_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_A | PTE_D,
    );
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Machine;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn setup_sv32_selective_sfence_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    write_word(machine, VM_PHYS_PAGE_B, VM_VALUE_B);
    write_word(machine, VM_PHYS_PAGE_C, VM_VALUE_C);
    write_word(machine, VM_PHYS_PAGE_D, VM_VALUE_D);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_A | PTE_D,
    );
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR_2,
        VM_PHYS_PAGE_B as u32,
        PTE_R | PTE_A | PTE_D,
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_TRANSLATED_LOAD_ADDR);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(2, VM_TRANSLATED_LOAD_ADDR_2);
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(3, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine.cpu_mut().hart_state_mut().registers.write(
        4,
        sv32_leaf_pte_addr(VM_LEAF_TABLE_1, VM_TRANSLATED_LOAD_ADDR) as u32,
    );
    machine.cpu_mut().hart_state_mut().registers.write(
        5,
        sv32_leaf_pte_addr(VM_LEAF_TABLE_1, VM_TRANSLATED_LOAD_ADDR_2) as u32,
    );
    machine.cpu_mut().hart_state_mut().registers.write(
        6,
        sv32_leaf_pte(VM_PHYS_PAGE_C as u32, PTE_R | PTE_A | PTE_D),
    );
    machine.cpu_mut().hart_state_mut().registers.write(
        7,
        sv32_leaf_pte(VM_PHYS_PAGE_D as u32, PTE_R | PTE_A | PTE_D),
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(8, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(9, 1 << MSTATUS_MPP_SHIFT);
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Machine;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn setup_sv32_instruction_page_fault_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Supervisor;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x80);
}

fn setup_sv32_malformed_nonleaf_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(
        machine,
        VM_ROOT_TABLE_1,
        sv32_nonleaf_pte(VM_LEAF_TABLE_1, PTE_A),
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
}

fn setup_sv32_malformed_superpage_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(
        machine,
        VM_ROOT_TABLE_3 + (((VM_SUPERPAGE_VIRTUAL_ADDR >> 22) & 0x3ff) as u64) * 4,
        sv32_leaf_pte(VM_SUPERPAGE_PHYSICAL_ADDR as u32, PTE_R | PTE_A | PTE_D),
    );
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
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
}

fn setup_sv32_superpage_fetch_state<P>(machine: &mut Machine<P, MemoryMap>, program_words: &[u32])
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_program_words(machine, RAM_BASE, program_words);
    write_word(machine, VM_SUPERPAGE_FETCH_DATA_PHYSICAL_ADDR, 0);
    install_sv32_superpage_mapping(
        machine,
        VM_ROOT_TABLE_3,
        VM_SUPERPAGE_FETCH_VIRTUAL_BASE,
        RAM_BASE as u32,
        PTE_R | PTE_W | PTE_X | PTE_A | PTE_D,
    );
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Supervisor;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_3, 1));
}

fn setup_sv32_satp_namespace_preserve_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    write_word(machine, VM_PHYS_PAGE_B, VM_VALUE_B);
    write_word(machine, VM_PHYS_PAGE_C, VM_VALUE_C);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_A | PTE_D,
    );
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_2,
        VM_LEAF_TABLE_2,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_B as u32,
        PTE_R | PTE_A | PTE_D,
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(1, VM_TRANSLATED_LOAD_ADDR);
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
    machine.cpu_mut().hart_state_mut().registers.write(
        4,
        sv32_leaf_pte_addr(VM_LEAF_TABLE_1, VM_TRANSLATED_LOAD_ADDR) as u32,
    );
    machine.cpu_mut().hart_state_mut().registers.write(
        5,
        sv32_leaf_pte(VM_PHYS_PAGE_C as u32, PTE_R | PTE_A | PTE_D),
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
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Machine;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
}

fn setup_csr_mip_machine_software_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
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
}

fn setup_wfi_machine_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
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
}

fn setup_vectored_machine_software_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .bus_mut()
        .store32(MSIP_BASE, 1)
        .expect("MSIP register should write during setup");
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
        .write(CsrAddress::Mtvec, 0x40 | 0b01);
}

fn setup_vectored_supervisor_external_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
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
        .write(CsrAddress::Stvec, 0x20 | 0b01);
}

fn setup_machine_external_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .bus_mut()
        .store32(CONTROLLER_BASE + 4, 1)
        .expect("controller enable register should write during setup");
    machine
        .bus_mut()
        .store32(CONTROLLER_BASE + 8, 1)
        .expect("controller pending register should write during setup");
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, 1 << 3);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mie, 1 << 11);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
}

fn setup_machine_interrupt_priority_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .bus_mut()
        .store32(CONTROLLER_BASE + 4, 1)
        .expect("controller enable register should write during setup");
    machine
        .bus_mut()
        .store32(CONTROLLER_BASE + 8, 1)
        .expect("controller pending register should write during setup");
    machine
        .bus_mut()
        .store32(MSIP_BASE, 1)
        .expect("MSIP register should write during setup");
    machine
        .bus_mut()
        .store32(TIMER_BASE + 8, 1)
        .expect("mtimecmp low should write during setup");
    machine
        .bus_mut()
        .store32(TIMER_BASE + 12, 0)
        .expect("mtimecmp high should write during setup");
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, 1 << 3);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mie, (1 << 3) | (1 << 7) | (1 << 11));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
}

fn setup_csr_sip_supervisor_software_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Supervisor;
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
        .write(CsrAddress::Sstatus, 1 << 1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Stvec, 0x20);
}

fn install_machine_timer_device(memory: &mut MemoryMap) {
    memory
        .map_device(MachineTimer::new(TIMER_BASE))
        .expect("machine timer should map");
}

fn setup_machine_timer_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .bus_mut()
        .store32(TIMER_BASE + 8, 1)
        .expect("mtimecmp low should write during setup");
    machine
        .bus_mut()
        .store32(TIMER_BASE + 12, 0)
        .expect("mtimecmp high should write during setup");
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, 1 << 3);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mie, 1 << 7);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
}

fn setup_machine_preempts_supervisor_handler_state<P>(machine: &mut Machine<P, MemoryMap>)
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
        .write(CsrAddress::Mie, 1 << 3);
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
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x60);
}

fn setup_machine_nested_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .bus_mut()
        .store32(TIMER_BASE + 8, 1)
        .expect("mtimecmp low should write during setup");
    machine
        .bus_mut()
        .store32(TIMER_BASE + 12, 0)
        .expect("mtimecmp high should write during setup");
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mstatus, 1 << 3);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mie, (1 << 7) | (1 << 3));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20 | 0b01);
}

fn setup_supervisor_nested_interrupt_state<P>(machine: &mut Machine<P, MemoryMap>)
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
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Supervisor;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mideleg, (1 << 9) | (1 << 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sie, (1 << 9) | (1 << 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sstatus, 1 << 1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Stvec, 0x20 | 0b01);
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

fn setup_sv32_asid_address_fence_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    write_word(machine, VM_PHYS_PAGE_B, VM_VALUE_B);
    write_word(machine, VM_PHYS_PAGE_C, VM_VALUE_C);
    write_word(machine, VM_PHYS_PAGE_D, VM_VALUE_D);
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
    machine.cpu_mut().hart_state_mut().registers.write(4, 1);
    machine.cpu_mut().hart_state_mut().registers.write(
        5,
        sv32_leaf_pte_addr(VM_LEAF_TABLE_1, VM_VIRTUAL_ADDR) as u32,
    );
    machine.cpu_mut().hart_state_mut().registers.write(
        6,
        sv32_leaf_pte(VM_PHYS_PAGE_C as u32, PTE_R | PTE_W | PTE_A | PTE_D),
    );
    machine.cpu_mut().hart_state_mut().registers.write(
        7,
        sv32_leaf_pte_addr(VM_LEAF_TABLE_2, VM_VIRTUAL_ADDR) as u32,
    );
    machine.cpu_mut().hart_state_mut().registers.write(
        8,
        sv32_leaf_pte(VM_PHYS_PAGE_D as u32, PTE_R | PTE_W | PTE_A | PTE_D),
    );
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(9, MSTATUS_MPRV | (1 << MSTATUS_MPP_SHIFT));
    machine
        .cpu_mut()
        .hart_state_mut()
        .registers
        .write(14, 1 << MSTATUS_MPP_SHIFT);
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

fn setup_sv32_supervisor_sum_load_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_VIRTUAL_ADDR,
        RESET_VECTOR,
        PTE_R | PTE_X | PTE_A,
    );
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_U | PTE_A | PTE_D,
    );
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Supervisor;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sstatus, MSTATUS_SUM);
}

fn setup_sv32_supervisor_mxr_load_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_VIRTUAL_ADDR,
        RESET_VECTOR,
        PTE_R | PTE_X | PTE_A,
    );
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_X | PTE_A,
    );
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Supervisor;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sstatus, MSTATUS_MXR);
}

fn setup_machine_mprv_supervisor_sum_load_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_R | PTE_U | PTE_A | PTE_D,
    );
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Machine;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine.cpu_mut().hart_state_mut().csrs.write(
        CsrAddress::Mstatus,
        MSTATUS_MPRV | MSTATUS_SUM | (1 << MSTATUS_MPP_SHIFT),
    );
}

fn setup_machine_mprv_supervisor_mxr_load_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    write_word(machine, VM_PHYS_PAGE_A, VM_VALUE_A);
    install_sv32_mapping(
        machine,
        VM_ROOT_TABLE_1,
        VM_LEAF_TABLE_1,
        VM_TRANSLATED_LOAD_ADDR,
        VM_PHYS_PAGE_A as u32,
        PTE_X | PTE_A,
    );
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::Machine;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Satp, sv32_satp_with_asid(VM_ROOT_TABLE_1, 1));
    machine.cpu_mut().hart_state_mut().csrs.write(
        CsrAddress::Mstatus,
        MSTATUS_MPRV | MSTATUS_MXR | (1 << MSTATUS_MPP_SHIFT),
    );
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

fn setup_supervisor_tvm_sfence_trap_state<P>(machine: &mut Machine<P, MemoryMap>)
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

fn setup_supervisor_tsr_sret_trap_state<P>(machine: &mut Machine<P, MemoryMap>)
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
        .write(CsrAddress::Mstatus, MSTATUS_TSR);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Sepc, 0x40);
}

fn setup_supervisor_tw_wfi_trap_state<P>(machine: &mut Machine<P, MemoryMap>)
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
        .write(CsrAddress::Mstatus, MSTATUS_TW);
}

fn setup_user_machine_csr_trap_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
}

fn setup_user_instret_counteren_trap_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mcounteren, 1 << 2);
}

fn setup_user_cycleh_counteren_enabled_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mcounteren, 1 << 0);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Scounteren, 1 << 0);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mcycleh, 1);
}

fn setup_user_timeh_counteren_enabled_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mcounteren, 1 << 1);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Scounteren, 1 << 1);
    machine
        .bus_mut()
        .store32(TIMER_BASE + 4, 1)
        .expect("mtime high should write during setup");
}

fn setup_instret_shadow_write_trap_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mtvec, 0x20);
}

fn setup_user_instret_counteren_enabled_state<P>(machine: &mut Machine<P, MemoryMap>)
where
    P: Processor<Error = CpuError> + CpuModel,
{
    machine.cpu_mut().hart_state_mut().privilege = PrivilegeMode::User;
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Mcounteren, 1 << 2);
    machine
        .cpu_mut()
        .hart_state_mut()
        .csrs
        .write(CsrAddress::Scounteren, 1 << 2);
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

fn write_program_words<P>(machine: &mut Machine<P, MemoryMap>, base: u64, words: &[u32])
where
    P: Processor<Error = CpuError> + CpuModel,
{
    for (index, word) in words.iter().copied().enumerate() {
        write_word(machine, base + (index as u64) * 4, word);
    }
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
        sv32_nonleaf_pte(leaf_table, 0),
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

const fn sv32_leaf_pte_addr(leaf_table: u64, virtual_address: u32) -> u64 {
    leaf_table + (((virtual_address >> PAGE_SHIFT) & 0x3ff) as u64) * 4
}

const fn sv32_leaf_pte(physical_address: u32, flags: u32) -> u32 {
    PTE_V | flags | ((physical_address >> PAGE_SHIFT) << 10)
}

const fn sv32_nonleaf_pte(next_table: u64, flags: u32) -> u32 {
    PTE_V | flags | (((next_table as u32) >> PAGE_SHIFT) << 10)
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
fn reference_core_runs_wfi_machine_interrupt_program() {
    assert_wfi_machine_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_wfi_machine_interrupt_program() {
    assert_wfi_machine_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_vectored_machine_software_interrupt_program() {
    assert_vectored_machine_software_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_vectored_machine_software_interrupt_program() {
    assert_vectored_machine_software_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_vectored_supervisor_external_interrupt_program() {
    assert_vectored_supervisor_external_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_vectored_supervisor_external_interrupt_program() {
    assert_vectored_supervisor_external_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_external_interrupt_program() {
    assert_machine_external_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_external_interrupt_program() {
    assert_machine_external_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_interrupt_priority_program() {
    assert_machine_interrupt_priority_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_interrupt_priority_program() {
    assert_machine_interrupt_priority_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_csr_mip_machine_software_interrupt_program() {
    assert_csr_mip_machine_software_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_csr_mip_machine_software_interrupt_program() {
    assert_csr_mip_machine_software_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_csr_sip_supervisor_software_interrupt_program() {
    assert_csr_sip_supervisor_software_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_csr_sip_supervisor_software_interrupt_program() {
    assert_csr_sip_supervisor_software_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_timer_interrupt_program() {
    assert_machine_timer_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_timer_interrupt_program() {
    assert_machine_timer_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_preempts_supervisor_handler_program() {
    assert_machine_preempts_supervisor_handler_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_preempts_supervisor_handler_program() {
    assert_machine_preempts_supervisor_handler_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_nested_interrupt_program() {
    assert_machine_nested_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_nested_interrupt_program() {
    assert_machine_nested_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_supervisor_nested_interrupt_program() {
    assert_supervisor_nested_interrupt_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_supervisor_nested_interrupt_program() {
    assert_supervisor_nested_interrupt_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_ad_bits_program() {
    assert_sv32_ad_bits_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_ad_bits_program() {
    assert_sv32_ad_bits_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_mprv_translation_program() {
    assert_machine_mprv_translation_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_mprv_translation_program() {
    assert_machine_mprv_translation_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_selective_sfence_program() {
    assert_sv32_selective_sfence_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_selective_sfence_program() {
    assert_sv32_selective_sfence_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_instruction_page_fault_program() {
    assert_sv32_instruction_page_fault_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_instruction_page_fault_program() {
    assert_sv32_instruction_page_fault_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_malformed_nonleaf_program() {
    assert_sv32_malformed_nonleaf_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_malformed_nonleaf_program() {
    assert_sv32_malformed_nonleaf_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_malformed_superpage_program() {
    assert_sv32_malformed_superpage_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_malformed_superpage_program() {
    assert_sv32_malformed_superpage_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_superpage_fetch_program() {
    assert_sv32_superpage_fetch_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_superpage_fetch_program() {
    assert_sv32_superpage_fetch_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_satp_namespace_preserve_program() {
    assert_sv32_satp_namespace_preserve_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_satp_namespace_preserve_program() {
    assert_sv32_satp_namespace_preserve_program(PipelineCore::new);
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
fn reference_core_runs_sv32_asid_address_fence_program() {
    assert_sv32_asid_address_fence_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_asid_address_fence_program() {
    assert_sv32_asid_address_fence_program(PipelineCore::new);
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
fn reference_core_runs_sv32_supervisor_sum_load_program() {
    assert_sv32_supervisor_sum_load_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_supervisor_sum_load_program() {
    assert_sv32_supervisor_sum_load_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_sv32_supervisor_mxr_load_program() {
    assert_sv32_supervisor_mxr_load_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_sv32_supervisor_mxr_load_program() {
    assert_sv32_supervisor_mxr_load_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_mprv_supervisor_sum_load_program() {
    assert_machine_mprv_supervisor_sum_load_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_mprv_supervisor_sum_load_program() {
    assert_machine_mprv_supervisor_sum_load_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_mprv_supervisor_mxr_load_program() {
    assert_machine_mprv_supervisor_mxr_load_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_mprv_supervisor_mxr_load_program() {
    assert_machine_mprv_supervisor_mxr_load_program(PipelineCore::new);
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

#[test]
fn reference_core_runs_supervisor_tvm_sfence_trap_program() {
    assert_supervisor_tvm_sfence_trap_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_supervisor_tvm_sfence_trap_program() {
    assert_supervisor_tvm_sfence_trap_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_supervisor_tsr_sret_trap_program() {
    assert_supervisor_tsr_sret_trap_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_supervisor_tsr_sret_trap_program() {
    assert_supervisor_tsr_sret_trap_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_supervisor_tw_wfi_trap_program() {
    assert_supervisor_tw_wfi_trap_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_supervisor_tw_wfi_trap_program() {
    assert_supervisor_tw_wfi_trap_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_user_machine_csr_trap_program() {
    assert_user_machine_csr_trap_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_user_machine_csr_trap_program() {
    assert_user_machine_csr_trap_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_user_instret_counteren_trap_program() {
    assert_user_instret_counteren_trap_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_user_instret_counteren_trap_program() {
    assert_user_instret_counteren_trap_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_user_cycleh_counteren_enabled_program() {
    assert_user_cycleh_counteren_enabled_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_user_cycleh_counteren_enabled_program() {
    assert_user_cycleh_counteren_enabled_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_user_timeh_counteren_enabled_program() {
    assert_user_timeh_counteren_enabled_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_user_timeh_counteren_enabled_program() {
    assert_user_timeh_counteren_enabled_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_machine_mcycleh_access_program() {
    assert_machine_mcycleh_access_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_machine_mcycleh_access_program() {
    assert_machine_mcycleh_access_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_user_instret_counteren_enabled_program() {
    assert_user_instret_counteren_enabled_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_user_instret_counteren_enabled_program() {
    assert_user_instret_counteren_enabled_program(PipelineCore::new);
}

#[test]
fn reference_core_runs_instret_shadow_write_trap_program() {
    assert_instret_shadow_write_trap_program(ReferenceCore::new);
}

#[test]
fn pipeline_core_runs_instret_shadow_write_trap_program() {
    assert_instret_shadow_write_trap_program(PipelineCore::new);
}
