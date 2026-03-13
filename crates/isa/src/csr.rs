//! Control and status register definitions used by the computer model.

use core::fmt;

/// Machine-visible CSR addresses needed for the first milestone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum CsrAddress {
    Mstatus = 0x300,
    Mie = 0x304,
    Mtvec = 0x305,
    Mcycle = 0xb00,
    Mepc = 0x341,
    Mcause = 0x342,
    Mtval = 0x343,
    Mip = 0x344,
}

impl TryFrom<u16> for CsrAddress {
    type Error = CsrAddressError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x300 => Ok(Self::Mstatus),
            0x304 => Ok(Self::Mie),
            0x305 => Ok(Self::Mtvec),
            0xb00 => Ok(Self::Mcycle),
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
