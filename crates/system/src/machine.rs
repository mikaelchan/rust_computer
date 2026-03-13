//! Top-level machine wrapper that ties a processor to a memory map.

use crate::{Bus, Clock, CpuCycle, Processor};

/// A single-core von Neumann machine with one processor and a unified address space.
#[derive(Debug)]
pub struct Machine<P, B> {
    clock: Clock,
    cpu: P,
    bus: B,
}

impl<P, B> Machine<P, B>
where
    P: Processor,
    B: Bus,
{
    #[must_use]
    pub fn new(cpu: P, bus: B) -> Self {
        Self {
            clock: Clock::default(),
            cpu,
            bus,
        }
    }

    pub fn reset(&mut self) {
        self.clock.reset();
        self.bus.reset();
        self.cpu.reset();
    }

    pub fn step_cycle(&mut self) -> Result<CpuCycle, P::Error> {
        self.bus.tick();
        let result = self.cpu.step_cycle(&mut self.bus)?;
        self.clock.tick();
        Ok(result)
    }

    pub fn run_cycles(&mut self, cycles: u64) -> Result<Vec<CpuCycle>, P::Error> {
        let mut reports = Vec::with_capacity(cycles as usize);
        for _ in 0..cycles {
            reports.push(self.step_cycle()?);
        }
        Ok(reports)
    }

    #[must_use]
    pub fn clock(&self) -> Clock {
        self.clock
    }

    #[must_use]
    pub fn cpu(&self) -> &P {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut P {
        &mut self.cpu
    }

    #[must_use]
    pub fn bus(&self) -> &B {
        &self.bus
    }

    pub fn bus_mut(&mut self) -> &mut B {
        &mut self.bus
    }
}
