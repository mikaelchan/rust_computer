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
    IllegalInstruction { instruction: u32 },
    Breakpoint,
    LoadAddressMisaligned { addr: u32 },
    StoreAddressMisaligned { addr: u32 },
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
            Self::Exception(Exception::IllegalInstruction { .. }) => 2,
            Self::Exception(Exception::Breakpoint) => 3,
            Self::Exception(Exception::LoadAddressMisaligned { .. }) => 4,
            Self::Exception(Exception::StoreAddressMisaligned { .. }) => 6,
            Self::Exception(Exception::EnvironmentCallFromUMode) => 8,
            Self::Exception(Exception::EnvironmentCallFromSMode) => 9,
            Self::Exception(Exception::EnvironmentCallFromMMode) => 11,
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
            Self::Exception(Exception::IllegalInstruction { instruction }) => instruction,
            Self::Exception(Exception::LoadAddressMisaligned { addr }) => addr,
            Self::Exception(Exception::StoreAddressMisaligned { addr }) => addr,
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
