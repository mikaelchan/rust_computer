/// Privilege modes reserved for future expansion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PrivilegeMode {
    User,
    Supervisor,
    #[default]
    Machine,
}
