/// Summary of a write-back event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteBackStatus {
    pub retired_instructions: u64,
}
