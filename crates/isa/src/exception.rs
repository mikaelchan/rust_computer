//! Trap and exception definitions visible at the ISA boundary.

use core::fmt;

/// Interrupt causes used by machine mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    MachineSoftware,
    MachineTimer,
    MachineExternal,
}

/// Exception causes raised by the initial RV32I implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exception {
    InstructionAddressMisaligned { addr: u32 },
    IllegalInstruction { instruction: u32 },
    Breakpoint,
    LoadAddressMisaligned { addr: u32 },
    StoreAddressMisaligned { addr: u32 },
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
    pub const fn mcause(self) -> u32 {
        match self {
            Self::Interrupt(Interrupt::MachineSoftware) => (1_u32 << 31) | 3,
            Self::Exception(Exception::InstructionAddressMisaligned { .. }) => 0,
            Self::Exception(Exception::IllegalInstruction { .. }) => 2,
            Self::Exception(Exception::Breakpoint) => 3,
            Self::Exception(Exception::LoadAddressMisaligned { .. }) => 4,
            Self::Exception(Exception::StoreAddressMisaligned { .. }) => 6,
            Self::Exception(Exception::EnvironmentCallFromMMode) => 11,
            Self::Interrupt(Interrupt::MachineTimer) => (1_u32 << 31) | 7,
            Self::Interrupt(Interrupt::MachineExternal) => (1_u32 << 31) | 11,
        }
    }

    #[must_use]
    pub const fn mtval(self) -> u32 {
        match self {
            Self::Exception(Exception::InstructionAddressMisaligned { addr }) => addr,
            Self::Exception(Exception::IllegalInstruction { instruction }) => instruction,
            Self::Exception(Exception::LoadAddressMisaligned { addr }) => addr,
            Self::Exception(Exception::StoreAddressMisaligned { addr }) => addr,
            Self::Exception(Exception::Breakpoint)
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
