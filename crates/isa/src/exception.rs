//! Trap and exception definitions visible at the ISA boundary.

use core::fmt;

/// Interrupt causes modeled by the current privileged CPU slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    SupervisorSoftware,
    MachineSoftware,
    SupervisorTimer,
    MachineTimer,
    SupervisorExternal,
    MachineExternal,
}

impl Interrupt {
    #[must_use]
    pub const fn cause_code(self) -> u32 {
        match self {
            Self::SupervisorSoftware => 1,
            Self::MachineSoftware => 3,
            Self::SupervisorTimer => 5,
            Self::MachineTimer => 7,
            Self::SupervisorExternal => 9,
            Self::MachineExternal => 11,
        }
    }

    #[must_use]
    pub const fn is_supervisor(self) -> bool {
        matches!(
            self,
            Self::SupervisorSoftware | Self::SupervisorTimer | Self::SupervisorExternal
        )
    }
}

/// Exception causes raised by the initial RV32I implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exception {
    InstructionAddressMisaligned { addr: u32 },
    InstructionAccessFault { addr: u32 },
    InstructionPageFault { addr: u32 },
    IllegalInstruction { instruction: u32 },
    Breakpoint,
    LoadAddressMisaligned { addr: u32 },
    LoadAccessFault { addr: u32 },
    LoadPageFault { addr: u32 },
    StoreAddressMisaligned { addr: u32 },
    StoreAccessFault { addr: u32 },
    StorePageFault { addr: u32 },
    EnvironmentCallFromUMode,
    EnvironmentCallFromSMode,
    EnvironmentCallFromMMode,
}

/// A trap is either a synchronous exception or an asynchronous interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trap {
    Exception(Exception),
    Interrupt(Interrupt),
}

impl Trap {
    #[must_use]
    pub const fn is_interrupt(self) -> bool {
        matches!(self, Self::Interrupt(_))
    }

    #[must_use]
    pub const fn cause_code(self) -> u32 {
        match self {
            Self::Interrupt(interrupt) => interrupt.cause_code(),
            Self::Exception(Exception::InstructionAddressMisaligned { .. }) => 0,
            Self::Exception(Exception::InstructionAccessFault { .. }) => 1,
            Self::Exception(Exception::IllegalInstruction { .. }) => 2,
            Self::Exception(Exception::Breakpoint) => 3,
            Self::Exception(Exception::LoadAddressMisaligned { .. }) => 4,
            Self::Exception(Exception::LoadAccessFault { .. }) => 5,
            Self::Exception(Exception::StoreAddressMisaligned { .. }) => 6,
            Self::Exception(Exception::StoreAccessFault { .. }) => 7,
            Self::Exception(Exception::EnvironmentCallFromUMode) => 8,
            Self::Exception(Exception::EnvironmentCallFromSMode) => 9,
            Self::Exception(Exception::EnvironmentCallFromMMode) => 11,
            Self::Exception(Exception::InstructionPageFault { .. }) => 12,
            Self::Exception(Exception::LoadPageFault { .. }) => 13,
            Self::Exception(Exception::StorePageFault { .. }) => 15,
        }
    }

    #[must_use]
    pub const fn cause_bits(self) -> u32 {
        let interrupt_bit = if self.is_interrupt() { 1 << 31 } else { 0 };
        self.cause_code() | interrupt_bit
    }

    #[must_use]
    pub const fn tval(self) -> u32 {
        match self {
            Self::Exception(Exception::InstructionAddressMisaligned { addr }) => addr,
            Self::Exception(Exception::InstructionAccessFault { addr }) => addr,
            Self::Exception(Exception::InstructionPageFault { addr }) => addr,
            Self::Exception(Exception::IllegalInstruction { instruction }) => instruction,
            Self::Exception(Exception::LoadAddressMisaligned { addr }) => addr,
            Self::Exception(Exception::LoadAccessFault { addr }) => addr,
            Self::Exception(Exception::LoadPageFault { addr }) => addr,
            Self::Exception(Exception::StoreAddressMisaligned { addr }) => addr,
            Self::Exception(Exception::StoreAccessFault { addr }) => addr,
            Self::Exception(Exception::StorePageFault { addr }) => addr,
            Self::Exception(Exception::Breakpoint)
            | Self::Exception(Exception::EnvironmentCallFromUMode)
            | Self::Exception(Exception::EnvironmentCallFromSMode)
            | Self::Exception(Exception::EnvironmentCallFromMMode)
            | Self::Interrupt(_) => 0,
        }
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exception(exception) => write!(f, "exception: {exception:?}"),
            Self::Interrupt(interrupt) => write!(f, "interrupt: {interrupt:?}"),
        }
    }
}

impl std::error::Error for Trap {}

#[cfg(test)]
mod tests {
    use super::{Exception, Trap};

    #[test]
    fn access_faults_keep_standard_cause_codes_and_tvals() {
        let instruction = Trap::Exception(Exception::InstructionAccessFault { addr: 0x1000 });
        let load = Trap::Exception(Exception::LoadAccessFault { addr: 0x2000 });
        let store = Trap::Exception(Exception::StoreAccessFault { addr: 0x3000 });

        assert_eq!(instruction.cause_code(), 1);
        assert_eq!(load.cause_code(), 5);
        assert_eq!(store.cause_code(), 7);

        assert_eq!(instruction.tval(), 0x1000);
        assert_eq!(load.tval(), 0x2000);
        assert_eq!(store.tval(), 0x3000);
    }
}
