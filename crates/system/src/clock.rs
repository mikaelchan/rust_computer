//! Clock and cycle-count helpers.

/// Global cycle counter for the machine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    cycle: u64,
}

impl Clock {
    #[must_use]
    pub const fn current(self) -> u64 {
        self.cycle
    }

    pub fn tick(&mut self) {
        self.cycle += 1;
    }

    pub fn reset(&mut self) {
        self.cycle = 0;
    }
}
