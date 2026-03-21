//! Control and status register definitions used by the computer model.

use core::fmt;

/// Control/status register addresses modeled by the current computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum CsrAddress {
    Sstatus = 0x100,
    Sie = 0x104,
    Stvec = 0x105,
    Scounteren = 0x106,
    Satp = 0x180,
    Mstatus = 0x300,
    Medeleg = 0x302,
    Mideleg = 0x303,
    Mie = 0x304,
    Mtvec = 0x305,
    Mcounteren = 0x306,
    Mcycle = 0xb00,
    Minstret = 0xb02,
    Mcycleh = 0xb80,
    Minstreth = 0xb82,
    Cycle = 0xc00,
    Time = 0xc01,
    Instret = 0xc02,
    Cycleh = 0xc80,
    Timeh = 0xc81,
    Instreth = 0xc82,
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

    #[must_use]
    pub const fn is_read_only(self) -> bool {
        ((self as u16 >> 10) & 0b11) == 0b11
    }

    #[must_use]
    pub const fn counteren_mask(self) -> Option<u32> {
        match self {
            Self::Cycle | Self::Cycleh => Some(1 << 0),
            Self::Time | Self::Timeh => Some(1 << 1),
            Self::Instret | Self::Instreth => Some(1 << 2),
            _ => None,
        }
    }
}

impl TryFrom<u16> for CsrAddress {
    type Error = CsrAddressError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x100 => Ok(Self::Sstatus),
            0x104 => Ok(Self::Sie),
            0x105 => Ok(Self::Stvec),
            0x106 => Ok(Self::Scounteren),
            0x180 => Ok(Self::Satp),
            0x300 => Ok(Self::Mstatus),
            0x302 => Ok(Self::Medeleg),
            0x303 => Ok(Self::Mideleg),
            0x304 => Ok(Self::Mie),
            0x305 => Ok(Self::Mtvec),
            0x306 => Ok(Self::Mcounteren),
            0xb00 => Ok(Self::Mcycle),
            0xb02 => Ok(Self::Minstret),
            0xb80 => Ok(Self::Mcycleh),
            0xb82 => Ok(Self::Minstreth),
            0xc00 => Ok(Self::Cycle),
            0xc01 => Ok(Self::Time),
            0xc02 => Ok(Self::Instret),
            0xc80 => Ok(Self::Cycleh),
            0xc81 => Ok(Self::Timeh),
            0xc82 => Ok(Self::Instreth),
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

#[cfg(test)]
mod tests {
    use super::CsrAddress;

    #[test]
    fn high_half_counter_csrs_decode_and_keep_counteren_mapping() {
        assert_eq!(CsrAddress::try_from(0xb80), Ok(CsrAddress::Mcycleh));
        assert_eq!(CsrAddress::try_from(0xb82), Ok(CsrAddress::Minstreth));
        assert_eq!(CsrAddress::try_from(0xc80), Ok(CsrAddress::Cycleh));
        assert_eq!(CsrAddress::try_from(0xc81), Ok(CsrAddress::Timeh));
        assert_eq!(CsrAddress::try_from(0xc82), Ok(CsrAddress::Instreth));

        assert_eq!(CsrAddress::Cycleh.counteren_mask(), Some(1 << 0));
        assert_eq!(CsrAddress::Timeh.counteren_mask(), Some(1 << 1));
        assert_eq!(CsrAddress::Instreth.counteren_mask(), Some(1 << 2));
        assert!(CsrAddress::Cycleh.is_read_only());
        assert!(CsrAddress::Timeh.is_read_only());
        assert!(CsrAddress::Instreth.is_read_only());
    }
}
