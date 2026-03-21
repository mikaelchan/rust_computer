/// Privilege modes supported by the current privileged CPU slice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PrivilegeMode {
    User,
    Supervisor,
    #[default]
    Machine,
}

impl PrivilegeMode {
    #[must_use]
    pub const fn csr_level(self) -> u8 {
        match self {
            Self::User => 0,
            Self::Supervisor => 1,
            Self::Machine => 3,
        }
    }
}
