//! Control and status register definitions used by the computer model.

use core::fmt;

/// Control/status register addresses modeled by the current computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum CsrAddress {
    Sstatus = 0x100,
    Sie = 0x104,
    Stvec = 0x105,
    Satp = 0x180,
    Mstatus = 0x300,
    Medeleg = 0x302,
    Mideleg = 0x303,
    Mie = 0x304,
    Mtvec = 0x305,
    Mcycle = 0xb00,
    Sepc = 0x141,
    Scause = 0x142,
    Stval = 0x143,
    Sip = 0x144,
    Mepc = 0x341,
    Mcause = 0x342,
    Mtval = 0x343,
    Mip = 0x344,
}

impl CsrAddress {
    #[must_use]
    pub const fn min_privilege_level(self) -> u8 {
        ((self as u16 >> 8) & 0b11) as u8
    }
}

impl TryFrom<u16> for CsrAddress {
    type Error = CsrAddressError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x100 => Ok(Self::Sstatus),
            0x104 => Ok(Self::Sie),
            0x105 => Ok(Self::Stvec),
            0x180 => Ok(Self::Satp),
            0x300 => Ok(Self::Mstatus),
            0x302 => Ok(Self::Medeleg),
            0x303 => Ok(Self::Mideleg),
            0x304 => Ok(Self::Mie),
            0x305 => Ok(Self::Mtvec),
            0xb00 => Ok(Self::Mcycle),
            0x141 => Ok(Self::Sepc),
            0x142 => Ok(Self::Scause),
            0x143 => Ok(Self::Stval),
            0x144 => Ok(Self::Sip),
            0x341 => Ok(Self::Mepc),
            0x342 => Ok(Self::Mcause),
            0x343 => Ok(Self::Mtval),
            0x344 => Ok(Self::Mip),
            _ => Err(CsrAddressError(value)),
        }
    }
}

/// Returned when a raw CSR index is not modeled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsrAddressError(pub u16);

impl fmt::Display for CsrAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported csr address 0x{:03x}", self.0)
    }
}

impl std::error::Error for CsrAddressError {}
