use rvsim_cpu::{CpuError, CpuModel, PipelineCore, ReferenceCore};
use rvsim_devices::{MachineSoftwareInterrupt, Ram, Rom};
use rvsim_isa::CsrAddress;
use rvsim_system::{Bus, Machine, MemoryMap, Processor};

const RESET_VECTOR: u32 = 0;
const RAM_BASE: u64 = 0x1000_0000;
const RAM_BYTES: usize = 0x1000;
const MSIP_BASE: u64 = 0x5000_0000;

const STORE_LOAD_PROGRAM: &str = include_str!("programs/store_load.hex");
const COUNT_LOOP_PROGRAM: &str = include_str!("programs/count_loop.hex");
const MSIP_INTERRUPT_PROGRAM: &str = include_str!("programs/msip_interrupt.hex");

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
    let words = parse_program_image(program_image);
    let mut memory = MemoryMap::new();
    memory
        .map_device(Rom::from_words(RESET_VECTOR as u64, &words))
        .expect("ROM should map");
    memory
        .map_device(Ram::new(RAM_BASE, RAM_BYTES))
        .expect("RAM should map");
    memory
        .map_device(MachineSoftwareInterrupt::new(MSIP_BASE))
        .expect("MSIP device should map");
    Machine::new(cpu, memory)
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
