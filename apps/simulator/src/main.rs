use rvsim_cpu::{CpuModel, ReferenceCore};
use rvsim_devices::{Ram, Rom, SimpleUart};
use rvsim_system::{Machine, MemoryMap};

const RESET_VECTOR: u32 = 0x0000_0000;
const RAM_BASE: u64 = 0x1000_0000;
const UART_BASE: u64 = 0x2000_0000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let program = [
        0x0050_0093, // addi x1, x0, 5
        0x00a0_0113, // addi x2, x0, 10
        0x0020_81b3, // add x3, x1, x2
        0x1000_0237, // lui x4, 0x10000
        0x0032_2023, // sw x3, 0(x4)
        0x0002_2283, // lw x5, 0(x4)
        0x0000_006f, // jal x0, 0
    ];

    let mut memory = MemoryMap::new();
    memory.map_device(Rom::from_words(RESET_VECTOR as u64, &program))?;
    memory.map_device(Ram::new(RAM_BASE, 0x1000))?;
    memory.map_device(SimpleUart::new(UART_BASE))?;

    let cpu = ReferenceCore::new(RESET_VECTOR);
    let mut machine = Machine::new(cpu, memory);

    for _ in 0..6 {
        let report = machine.step_cycle()?;
        println!(
            "cycle={} retired={} pc=0x{:08x}",
            machine.clock().current(),
            report.retired_instructions,
            machine.cpu().hart_state().pc
        );
    }

    println!(
        "x3={} x5={}",
        machine.cpu().hart_state().registers.read(3),
        machine.cpu().hart_state().registers.read(5)
    );

    println!(
        "workspace ready: use ReferenceCore for correctness and PipelineCore for staged expansion"
    );

    Ok(())
}
