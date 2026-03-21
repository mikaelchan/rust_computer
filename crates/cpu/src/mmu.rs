use rvsim_isa::{CsrAddress, Exception, Trap};
use rvsim_system::{Bus, BusError};

use crate::state::{CsrFile, PrivilegeMode};

const PAGE_SHIFT: u32 = 12;
const PAGE_SIZE: u32 = 1 << PAGE_SHIFT;
const PAGE_OFFSET_MASK: u32 = PAGE_SIZE - 1;
const SATP_MODE_SHIFT: u32 = 31;
const SATP_MODE_SV32: u32 = 1;
const SATP_PPN_MASK: u32 = (1 << 22) - 1;
const PTE_V: u32 = 1 << 0;
const PTE_R: u32 = 1 << 1;
const PTE_W: u32 = 1 << 2;
const PTE_X: u32 = 1 << 3;
const PTE_U: u32 = 1 << 4;
const PTE_A: u32 = 1 << 6;
const PTE_D: u32 = 1 << 7;
const PTE_PPN_MASK: u32 = (1 << 22) - 1;
const PTE_PPN0_MASK: u32 = (1 << 10) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryAccess {
    Instruction,
    Load,
    Store,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationResult {
    PhysicalAddress(u32),
    Stall,
    PageFault(Trap),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageWalker {
    active: Option<WalkState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WalkState {
    request: TranslationRequest,
    level: u32,
    pte_address: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TranslationRequest {
    satp: u32,
    privilege: PrivilegeMode,
    virtual_address: u32,
    access: MemoryAccess,
}

enum StepResult {
    Stall,
    Drained,
    Ready(u32),
    PageFault(Trap),
}

impl PageWalker {
    pub fn reset(&mut self) {
        self.active = None;
    }

    pub fn translate(
        &mut self,
        bus: &mut dyn Bus,
        csrs: &CsrFile,
        privilege: PrivilegeMode,
        virtual_address: u32,
        access: MemoryAccess,
    ) -> Result<TranslationResult, BusError> {
        let satp = csrs.read(CsrAddress::Satp);
        if !translation_enabled(satp, privilege) {
            self.active = None;
            return Ok(TranslationResult::PhysicalAddress(virtual_address));
        }

        let request = TranslationRequest {
            satp,
            privilege,
            virtual_address,
            access,
        };

        loop {
            if let Some(active) = self.active
                && active.request != request
            {
                match self.step(bus, true)? {
                    StepResult::Stall => return Ok(TranslationResult::Stall),
                    StepResult::Drained => continue,
                    StepResult::Ready(_) | StepResult::PageFault(_) => unreachable!(),
                }
            }

            if self.active.is_none() {
                self.active = Some(WalkState {
                    request,
                    level: 1,
                    pte_address: root_pte_address(request),
                });
            }

            match self.step(bus, false)? {
                StepResult::Ready(physical_address) => {
                    return Ok(TranslationResult::PhysicalAddress(physical_address));
                }
                StepResult::PageFault(trap) => return Ok(TranslationResult::PageFault(trap)),
                StepResult::Stall => return Ok(TranslationResult::Stall),
                StepResult::Drained => continue,
            }
        }
    }

    fn step(&mut self, bus: &mut dyn Bus, drain_only: bool) -> Result<StepResult, BusError> {
        let state = self
            .active
            .expect("page walker step requires an active translation");

        let pte = match bus.load32(u64::from(state.pte_address)) {
            Ok(pte) => pte,
            Err(BusError::Busy { .. }) => return Ok(StepResult::Stall),
            Err(BusError::MisalignedAccess { .. })
            | Err(BusError::UnmappedAddress { .. })
            | Err(BusError::ReadOnlyAddress { .. })
            | Err(BusError::DeviceFault { .. }) => {
                self.active = None;
                return Ok(if drain_only {
                    StepResult::Drained
                } else {
                    StepResult::PageFault(page_fault(state.request))
                });
            }
        };

        let flags = pte_flags(pte);
        if (flags & PTE_V) == 0 || ((flags & PTE_R) == 0 && (flags & PTE_W) != 0) {
            self.active = None;
            return Ok(if drain_only {
                StepResult::Drained
            } else {
                StepResult::PageFault(page_fault(state.request))
            });
        }

        if (flags & (PTE_R | PTE_X)) != 0 {
            let trap = page_fault(state.request);
            let Some(physical_address) = translate_leaf(state, pte) else {
                self.active = None;
                return Ok(if drain_only {
                    StepResult::Drained
                } else {
                    StepResult::PageFault(trap)
                });
            };
            if !permissions_allow(state.request, flags) {
                self.active = None;
                return Ok(if drain_only {
                    StepResult::Drained
                } else {
                    StepResult::PageFault(trap)
                });
            }

            self.active = None;
            return Ok(if drain_only {
                StepResult::Drained
            } else {
                StepResult::Ready(physical_address)
            });
        }

        if state.level == 0 {
            self.active = None;
            return Ok(if drain_only {
                StepResult::Drained
            } else {
                StepResult::PageFault(page_fault(state.request))
            });
        }

        let next_level = state.level - 1;
        let next_table_base = pte_ppn(pte) << PAGE_SHIFT;
        let vpn_index = vpn(state.request.virtual_address, next_level);
        self.active = Some(WalkState {
            request: state.request,
            level: next_level,
            pte_address: next_table_base.wrapping_add(vpn_index * 4),
        });
        Ok(StepResult::Drained)
    }
}

const fn translation_enabled(satp: u32, privilege: PrivilegeMode) -> bool {
    !matches!(privilege, PrivilegeMode::Machine) && (satp >> SATP_MODE_SHIFT) == SATP_MODE_SV32
}

const fn root_pte_address(request: TranslationRequest) -> u32 {
    let root_table = (request.satp & SATP_PPN_MASK) << PAGE_SHIFT;
    root_table.wrapping_add(vpn(request.virtual_address, 1) * 4)
}

const fn vpn(virtual_address: u32, level: u32) -> u32 {
    match level {
        0 => (virtual_address >> 12) & 0x3ff,
        1 => (virtual_address >> 22) & 0x3ff,
        _ => 0,
    }
}

const fn pte_flags(pte: u32) -> u32 {
    pte & 0xff
}

const fn pte_ppn(pte: u32) -> u32 {
    (pte >> 10) & PTE_PPN_MASK
}

fn translate_leaf(state: WalkState, pte: u32) -> Option<u32> {
    let virtual_address = state.request.virtual_address;
    let offset = virtual_address & PAGE_OFFSET_MASK;
    let pte_ppn = pte_ppn(pte);
    let pte_ppn1 = pte_ppn >> 10;
    let pte_ppn0 = pte_ppn & PTE_PPN0_MASK;
    let physical_page = match state.level {
        1 => {
            if pte_ppn0 != 0 {
                return None;
            }
            (pte_ppn1 << 10) | vpn(virtual_address, 0)
        }
        0 => (pte_ppn1 << 10) | pte_ppn0,
        _ => return None,
    };

    Some((physical_page << PAGE_SHIFT) | offset)
}

const fn permissions_allow(request: TranslationRequest, flags: u32) -> bool {
    let user_page = (flags & PTE_U) != 0;
    let accessed = (flags & PTE_A) != 0;
    let dirty = (flags & PTE_D) != 0;
    let readable = (flags & PTE_R) != 0;
    let writable = (flags & PTE_W) != 0;
    let executable = (flags & PTE_X) != 0;

    if !accessed || (matches!(request.access, MemoryAccess::Store) && !dirty) {
        return false;
    }

    match request.privilege {
        PrivilegeMode::Machine => true,
        PrivilegeMode::Supervisor if user_page => false,
        PrivilegeMode::User if !user_page => false,
        _ => match request.access {
            MemoryAccess::Instruction => executable,
            MemoryAccess::Load => readable,
            MemoryAccess::Store => writable,
        },
    }
}

const fn page_fault(request: TranslationRequest) -> Trap {
    Trap::Exception(match request.access {
        MemoryAccess::Instruction => Exception::InstructionPageFault {
            addr: request.virtual_address,
        },
        MemoryAccess::Load => Exception::LoadPageFault {
            addr: request.virtual_address,
        },
        MemoryAccess::Store => Exception::StorePageFault {
            addr: request.virtual_address,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PrivilegeMode;

    #[test]
    fn supervisor_translation_is_enabled_only_for_sv32() {
        assert!(!translation_enabled(0, PrivilegeMode::Supervisor));
        assert!(translation_enabled(
            1 << SATP_MODE_SHIFT,
            PrivilegeMode::Supervisor
        ));
        assert!(!translation_enabled(
            1 << SATP_MODE_SHIFT,
            PrivilegeMode::Machine
        ));
    }

    #[test]
    fn supervisor_superpage_requires_zero_lower_ppn_bits() {
        let request = TranslationRequest {
            satp: 1 << SATP_MODE_SHIFT,
            privilege: PrivilegeMode::Supervisor,
            virtual_address: 0x0040_1234,
            access: MemoryAccess::Instruction,
        };
        let state = WalkState {
            request,
            level: 1,
            pte_address: 0,
        };

        assert!(translate_leaf(state, (1 << 10) | PTE_V | PTE_R | PTE_X | PTE_A).is_none());
    }

    #[test]
    fn supervisor_cannot_access_user_page_without_sum_support() {
        let request = TranslationRequest {
            satp: 1 << SATP_MODE_SHIFT,
            privilege: PrivilegeMode::Supervisor,
            virtual_address: 0,
            access: MemoryAccess::Load,
        };

        assert!(!permissions_allow(
            request,
            PTE_V | PTE_R | PTE_U | PTE_A | PTE_D
        ));
    }
}
