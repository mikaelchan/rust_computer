use super::{BranchPrediction, BranchPredictor};

/// A direct-mapped 2-bit saturating-counter branch history table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BimodalPredictor {
    table: Vec<u8>,
}

impl BimodalPredictor {
    #[must_use]
    pub fn new(entries: usize) -> Self {
        Self {
            table: vec![0b01; entries.max(1)],
        }
    }

    fn index(&self, pc: u32) -> usize {
        ((pc >> 2) as usize) % self.table.len()
    }
}

impl Default for BimodalPredictor {
    fn default() -> Self {
        Self::new(64)
    }
}

impl BranchPredictor for BimodalPredictor {
    fn predict(&self, pc: u32, fallthrough: u32, target: u32) -> BranchPrediction {
        let counter = self.table[self.index(pc)];
        let taken = counter >= 0b10;
        BranchPrediction {
            taken,
            target: if taken { target } else { fallthrough },
        }
    }

    fn update(&mut self, pc: u32, taken: bool) {
        let index = self.index(pc);
        let entry = &mut self.table[index];
        *entry = match (*entry, taken) {
            (0b00, false) => 0b00,
            (0b00, true) => 0b01,
            (0b01, false) => 0b00,
            (0b01, true) => 0b10,
            (0b10, false) => 0b01,
            (0b10, true) => 0b11,
            (0b11, false) => 0b10,
            (0b11, true) => 0b11,
            _ => 0b01,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::BimodalPredictor;
    use crate::predictor::BranchPredictor;

    #[test]
    fn predictor_moves_towards_taken() {
        let mut predictor = BimodalPredictor::new(4);
        let first = predictor.predict(0x1000, 0x1004, 0x1080);
        assert!(!first.taken);

        predictor.update(0x1000, true);
        predictor.update(0x1000, true);
        let second = predictor.predict(0x1000, 0x1004, 0x1080);
        assert!(second.taken);
        assert_eq!(second.target, 0x1080);
    }
}
