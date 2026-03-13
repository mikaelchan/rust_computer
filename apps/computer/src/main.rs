use rvsim_cpu::{CpuModel, ReferenceCore};
use rvsim_devices::{MachineTimer, Ram, Rom, SimpleUart};
use rvsim_isa::CsrAddress;
use rvsim_system::{Bus, Machine, MemoryMap};

const RESET_VECTOR: u32 = 0x0000_0000;
const RAM_BASE: u64 = 0x1000_0000;
const UART_BASE: u64 = 0x2000_0000;
const TIMER_BASE: u64 = 0x3000_0000;

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
        0x0010_0313, // addi x6, x0, 1
        0x0000_006f, // jal x0, 0
    ];

    let mut memory = MemoryMap::new();
    memory.map_device(Rom::from_words(RESET_VECTOR as u64, &program))?;
    memory.map_device(Ram::new(RAM_BASE, 0x1000))?;
    memory.map_device(SimpleUart::new(UART_BASE))?;
    memory.map_device(MachineTimer::new(TIMER_BASE))?;

    let cpu = ReferenceCore::new(RESET_VECTOR);
    let mut machine = Machine::new(cpu, memory);
    machine.bus_mut().store32(TIMER_BASE + 8, 7)?;
    machine.bus_mut().store32(TIMER_BASE + 12, 0)?;
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

    for _ in 0..9 {
        let report = machine.step_cycle()?;
        println!(
            "cycle={} retired={} pc=0x{:08x}",
            machine.clock().current(),
            report.retired_instructions,
            machine.cpu().hart_state().pc
        );
    }

    println!(
        "x3={} x5={} interrupt_seen={} mepc=0x{:08x} mcause=0x{:08x}",
        machine.cpu().hart_state().registers.read(3),
        machine.cpu().hart_state().registers.read(5),
        machine.cpu().hart_state().registers.read(6),
        machine.cpu().hart_state().csrs.read(CsrAddress::Mepc),
        machine.cpu().hart_state().csrs.read(CsrAddress::Mcause)
    );

    println!(
        "computer ready: ReferenceCore provides the architectural oracle and external devices now include a machine timer"
    );

    Ok(())
}
