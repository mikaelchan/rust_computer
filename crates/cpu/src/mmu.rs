use rvsim_isa::{CsrAddress, Exception, Trap};
use rvsim_system::{Bus, BusError};

use crate::state::{CsrFile, PrivilegeMode};

const PAGE_SHIFT: u32 = 12;
const PAGE_SIZE: u32 = 1 << PAGE_SHIFT;
const PAGE_OFFSET_MASK: u32 = PAGE_SIZE - 1;
const SUPERPAGE_SHIFT: u32 = 22;
const SUPERPAGE_SIZE: u32 = 1 << SUPERPAGE_SHIFT;
const SUPERPAGE_OFFSET_MASK: u32 = SUPERPAGE_SIZE - 1;
const SATP_MODE_SHIFT: u32 = 31;
const SATP_MODE_SV32: u32 = 1;
const SATP_PPN_MASK: u32 = (1 << 22) - 1;
const MSTATUS_SUM: u32 = 1 << 18;
const MSTATUS_MXR: u32 = 1 << 19;
const PTE_V: u32 = 1 << 0;
const PTE_R: u32 = 1 << 1;
const PTE_W: u32 = 1 << 2;
const PTE_X: u32 = 1 << 3;
const PTE_U: u32 = 1 << 4;
const PTE_A: u32 = 1 << 6;
const PTE_D: u32 = 1 << 7;
const PTE_PPN_MASK: u32 = (1 << 22) - 1;
const PTE_PPN0_MASK: u32 = (1 << 10) - 1;
const TLB_ENTRY_COUNT: usize = 8;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageWalker {
    active: Option<WalkState>,
    tlb: [Option<TlbEntry>; TLB_ENTRY_COUNT],
    next_tlb_entry: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WalkState {
    request: TranslationRequest,
    level: u32,
    pte_address: u32,
    phase: WalkPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalkPhase {
    ReadPte,
    WriteLeaf {
        physical_address: u32,
        flags: u32,
        value: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TranslationRequest {
    satp: u32,
    status: u32,
    privilege: PrivilegeMode,
    virtual_address: u32,
    access: MemoryAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TlbEntry {
    satp: u32,
    virtual_base: u32,
    physical_base: u32,
    page_mask: u32,
    flags: u32,
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
        self.flush();
        self.next_tlb_entry = 0;
    }

    pub fn flush(&mut self) {
        self.tlb = [None; TLB_ENTRY_COUNT];
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
        let request = TranslationRequest {
            satp,
            status: translation_status(csrs),
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

            if !translation_enabled(satp, privilege) {
                return Ok(TranslationResult::PhysicalAddress(virtual_address));
            }

            if self.active.is_none() {
                if let Some(result) = self.lookup_tlb(request) {
                    return Ok(result);
                }

                self.active = Some(WalkState {
                    request,
                    level: 1,
                    pte_address: root_pte_address(request),
                    phase: WalkPhase::ReadPte,
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

        if let WalkPhase::WriteLeaf {
            physical_address,
            flags,
            value,
        } = state.phase
        {
            return self.finish_leaf_update(bus, state, physical_address, flags, value, drain_only);
        }

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

            if needs_access_bit_update(state.request, flags) {
                self.active = Some(WalkState {
                    phase: WalkPhase::WriteLeaf {
                        physical_address,
                        flags: updated_flags(state.request, flags),
                        value: updated_pte(pte, state.request),
                    },
                    ..state
                });
                return self.step(bus, drain_only);
            }

            if !drain_only {
                self.insert_tlb(state, flags, physical_address);
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
            phase: WalkPhase::ReadPte,
        });
        Ok(StepResult::Drained)
    }

    fn finish_leaf_update(
        &mut self,
        bus: &mut dyn Bus,
        state: WalkState,
        physical_address: u32,
        flags: u32,
        value: u32,
        drain_only: bool,
    ) -> Result<StepResult, BusError> {
        match bus.store32(u64::from(state.pte_address), value) {
            Ok(()) => {
                if !drain_only {
                    self.insert_tlb(state, flags, physical_address);
                }
                self.active = None;
                Ok(if drain_only {
                    StepResult::Drained
                } else {
                    StepResult::Ready(physical_address)
                })
            }
            Err(BusError::Busy { .. }) => Ok(StepResult::Stall),
            Err(BusError::MisalignedAccess { .. })
            | Err(BusError::UnmappedAddress { .. })
            | Err(BusError::ReadOnlyAddress { .. })
            | Err(BusError::DeviceFault { .. }) => {
                self.active = None;
                Ok(if drain_only {
                    StepResult::Drained
                } else {
                    StepResult::PageFault(page_fault(state.request))
                })
            }
        }
    }

    fn lookup_tlb(&self, request: TranslationRequest) -> Option<TranslationResult> {
        let mut saw_permission_fault = false;
        for entry in self.tlb.iter().flatten().copied() {
            if !entry.matches(request) {
                continue;
            }
            if !permissions_allow(request, entry.flags) {
                saw_permission_fault = true;
                continue;
            }
            if needs_access_bit_update(request, entry.flags) {
                continue;
            }

            return Some(TranslationResult::PhysicalAddress(entry.translate(request)));
        }

        saw_permission_fault.then_some(TranslationResult::PageFault(page_fault(request)))
    }

    fn insert_tlb(&mut self, state: WalkState, flags: u32, physical_address: u32) {
        let page_mask = match state.level {
            1 => SUPERPAGE_OFFSET_MASK,
            0 => PAGE_OFFSET_MASK,
            _ => return,
        };
        let entry = TlbEntry {
            satp: state.request.satp,
            virtual_base: state.request.virtual_address & !page_mask,
            physical_base: physical_address & !page_mask,
            page_mask,
            flags,
        };
        self.tlb[self.next_tlb_entry] = Some(entry);
        self.next_tlb_entry = (self.next_tlb_entry + 1) % TLB_ENTRY_COUNT;
    }
}

impl Default for PageWalker {
    fn default() -> Self {
        Self {
            active: None,
            tlb: [None; TLB_ENTRY_COUNT],
            next_tlb_entry: 0,
        }
    }
}

impl TlbEntry {
    const fn matches(self, request: TranslationRequest) -> bool {
        self.satp == request.satp
            && (request.virtual_address & !self.page_mask) == self.virtual_base
    }

    const fn translate(self, request: TranslationRequest) -> u32 {
        self.physical_base | (request.virtual_address & self.page_mask)
    }
}

const fn translation_enabled(satp: u32, privilege: PrivilegeMode) -> bool {
    !matches!(privilege, PrivilegeMode::Machine) && (satp >> SATP_MODE_SHIFT) == SATP_MODE_SV32
}

fn translation_status(csrs: &CsrFile) -> u32 {
    csrs.read(CsrAddress::Mstatus) & (MSTATUS_SUM | MSTATUS_MXR)
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

const fn access_bits_for(request: TranslationRequest) -> u32 {
    PTE_A
        | if matches!(request.access, MemoryAccess::Store) {
            PTE_D
        } else {
            0
        }
}

const fn needs_access_bit_update(request: TranslationRequest, flags: u32) -> bool {
    (flags & access_bits_for(request)) != access_bits_for(request)
}

const fn updated_flags(request: TranslationRequest, flags: u32) -> u32 {
    flags | access_bits_for(request)
}

const fn updated_pte(pte: u32, request: TranslationRequest) -> u32 {
    pte | access_bits_for(request)
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
    let readable = (flags & PTE_R) != 0;
    let writable = (flags & PTE_W) != 0;
    let executable = (flags & PTE_X) != 0;
    let mxr = (request.status & MSTATUS_MXR) != 0;
    let sum = (request.status & MSTATUS_SUM) != 0;
    let load_permitted = readable || (mxr && executable);

    match request.privilege {
        PrivilegeMode::Machine => true,
        PrivilegeMode::Supervisor if user_page => {
            if matches!(request.access, MemoryAccess::Instruction) || !sum {
                false
            } else {
                match request.access {
                    MemoryAccess::Instruction => executable,
                    MemoryAccess::Load => load_permitted,
                    MemoryAccess::Store => writable,
                }
            }
        }
        PrivilegeMode::User if !user_page => false,
        _ => match request.access {
            MemoryAccess::Instruction => executable,
            MemoryAccess::Load => load_permitted,
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
    use rvsim_system::BusError;

    #[derive(Debug)]
    struct CountingBus {
        bytes: Vec<u8>,
        load32_count: u32,
        store32_count: u32,
    }

    impl CountingBus {
        fn new(size: usize) -> Self {
            Self {
                bytes: vec![0; size],
                load32_count: 0,
                store32_count: 0,
            }
        }

        fn store_word(&mut self, addr: u32, value: u32) {
            let offset = addr as usize;
            self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }

        fn read_word(&self, addr: u32) -> u32 {
            let offset = addr as usize;
            u32::from_le_bytes(self.bytes[offset..offset + 4].try_into().unwrap_or([0; 4]))
        }
    }

    impl Bus for CountingBus {
        fn load8(&mut self, addr: u64) -> Result<u8, BusError> {
            Ok(self.bytes[addr as usize])
        }

        fn store8(&mut self, addr: u64, value: u8) -> Result<(), BusError> {
            self.bytes[addr as usize] = value;
            Ok(())
        }

        fn load32(&mut self, addr: u64) -> Result<u32, BusError> {
            self.load32_count += 1;
            Ok(self.read_word(addr as u32))
        }

        fn store32(&mut self, addr: u64, value: u32) -> Result<(), BusError> {
            self.store32_count += 1;
            self.store_word(addr as u32, value);
            Ok(())
        }
    }

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
            status: 0,
            privilege: PrivilegeMode::Supervisor,
            virtual_address: 0x0040_1234,
            access: MemoryAccess::Instruction,
        };
        let state = WalkState {
            request,
            level: 1,
            pte_address: 0,
            phase: WalkPhase::ReadPte,
        };

        assert!(translate_leaf(state, (1 << 10) | PTE_V | PTE_R | PTE_X | PTE_A).is_none());
    }

    #[test]
    fn supervisor_cannot_access_user_page_without_sum_support() {
        let request = TranslationRequest {
            satp: 1 << SATP_MODE_SHIFT,
            status: 0,
            privilege: PrivilegeMode::Supervisor,
            virtual_address: 0,
            access: MemoryAccess::Load,
        };

        assert!(!permissions_allow(
            request,
            PTE_V | PTE_R | PTE_U | PTE_A | PTE_D
        ));
    }

    #[test]
    fn supervisor_sum_allows_loads_from_user_pages_but_not_fetches() {
        let load_request = TranslationRequest {
            satp: 1 << SATP_MODE_SHIFT,
            status: MSTATUS_SUM,
            privilege: PrivilegeMode::Supervisor,
            virtual_address: 0,
            access: MemoryAccess::Load,
        };
        let fetch_request = TranslationRequest {
            access: MemoryAccess::Instruction,
            ..load_request
        };

        assert!(permissions_allow(
            load_request,
            PTE_V | PTE_R | PTE_U | PTE_A | PTE_D
        ));
        assert!(!permissions_allow(
            fetch_request,
            PTE_V | PTE_X | PTE_U | PTE_A
        ));
    }

    #[test]
    fn mxr_allows_loads_from_execute_only_pages() {
        let request = TranslationRequest {
            satp: 1 << SATP_MODE_SHIFT,
            status: MSTATUS_MXR,
            privilege: PrivilegeMode::Supervisor,
            virtual_address: 0,
            access: MemoryAccess::Load,
        };

        assert!(permissions_allow(request, PTE_V | PTE_X | PTE_A));
        assert!(!permissions_allow(
            TranslationRequest {
                status: 0,
                ..request
            },
            PTE_V | PTE_X | PTE_A
        ));
    }

    #[test]
    fn tlb_hit_reuses_completed_translation() {
        let mut bus = CountingBus::new(0x10_000);
        let mut walker = PageWalker::default();
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Satp, sv32_satp(0x2000));
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x1000,
            PTE_R | PTE_A | PTE_D,
        );

        let first = walker
            .translate(
                &mut bus,
                &csrs,
                PrivilegeMode::Supervisor,
                0x4123,
                MemoryAccess::Load,
            )
            .expect("first translation should succeed");
        let second = walker
            .translate(
                &mut bus,
                &csrs,
                PrivilegeMode::Supervisor,
                0x4123,
                MemoryAccess::Load,
            )
            .expect("second translation should hit tlb");

        assert_eq!(first, TranslationResult::PhysicalAddress(0x1123));
        assert_eq!(second, TranslationResult::PhysicalAddress(0x1123));
        assert_eq!(bus.load32_count, 2);
    }

    #[test]
    fn flush_discards_cached_translation() {
        let mut bus = CountingBus::new(0x10_000);
        let mut walker = PageWalker::default();
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Satp, sv32_satp(0x2000));
        install_sv32_mapping(
            &mut bus,
            0x2000,
            0x3000,
            0x4000,
            0x1000,
            PTE_R | PTE_A | PTE_D,
        );

        let _ = walker
            .translate(
                &mut bus,
                &csrs,
                PrivilegeMode::Supervisor,
                0x4123,
                MemoryAccess::Load,
            )
            .expect("first translation should succeed");
        walker.flush();
        let translated = walker
            .translate(
                &mut bus,
                &csrs,
                PrivilegeMode::Supervisor,
                0x4123,
                MemoryAccess::Load,
            )
            .expect("translation after flush should refill tlb");

        assert_eq!(translated, TranslationResult::PhysicalAddress(0x1123));
        assert_eq!(bus.load32_count, 4);
    }

    #[test]
    fn load_translation_sets_accessed_bit() {
        let mut bus = CountingBus::new(0x10_000);
        let mut walker = PageWalker::default();
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Satp, sv32_satp(0x2000));
        install_sv32_mapping(&mut bus, 0x2000, 0x3000, 0x4000, 0x1000, PTE_R);

        let translated = walker
            .translate(
                &mut bus,
                &csrs,
                PrivilegeMode::Supervisor,
                0x4123,
                MemoryAccess::Load,
            )
            .expect("load translation should update accessed bit");

        assert_eq!(translated, TranslationResult::PhysicalAddress(0x1123));
        assert_eq!(bus.read_word(0x3010) & PTE_A, PTE_A);
        assert_eq!(bus.read_word(0x3010) & PTE_D, 0);
        assert_eq!(bus.store32_count, 1);
    }

    #[test]
    fn store_after_load_rewalks_to_set_dirty_bit() {
        let mut bus = CountingBus::new(0x10_000);
        let mut walker = PageWalker::default();
        let mut csrs = CsrFile::default();
        csrs.write(CsrAddress::Satp, sv32_satp(0x2000));
        install_sv32_mapping(&mut bus, 0x2000, 0x3000, 0x4000, 0x1000, PTE_R | PTE_W);

        let load = walker
            .translate(
                &mut bus,
                &csrs,
                PrivilegeMode::Supervisor,
                0x4123,
                MemoryAccess::Load,
            )
            .expect("load translation should update accessed bit");
        let store = walker
            .translate(
                &mut bus,
                &csrs,
                PrivilegeMode::Supervisor,
                0x4123,
                MemoryAccess::Store,
            )
            .expect("store translation should rewalk to update dirty bit");

        assert_eq!(load, TranslationResult::PhysicalAddress(0x1123));
        assert_eq!(store, TranslationResult::PhysicalAddress(0x1123));
        assert_eq!(bus.read_word(0x3010) & (PTE_A | PTE_D), PTE_A | PTE_D);
        assert_eq!(bus.store32_count, 2);
    }

    fn install_sv32_mapping(
        bus: &mut CountingBus,
        root_table: u32,
        leaf_table: u32,
        virtual_page: u32,
        physical_page: u32,
        flags: u32,
    ) {
        let vpn1 = (virtual_page >> 22) & 0x3ff;
        let vpn0 = (virtual_page >> 12) & 0x3ff;
        bus.store_word(root_table + (vpn1 * 4), sv32_nonleaf(leaf_table));
        bus.store_word(
            leaf_table + (vpn0 * 4),
            sv32_leaf(physical_page, flags | PTE_V),
        );
    }

    const fn sv32_nonleaf(next_table: u32) -> u32 {
        ((next_table >> 12) << 10) | PTE_V
    }

    const fn sv32_leaf(physical_page: u32, flags: u32) -> u32 {
        ((physical_page >> 12) << 10) | flags
    }

    const fn sv32_satp(root_table: u32) -> u32 {
        (SATP_MODE_SV32 << SATP_MODE_SHIFT) | (root_table >> 12)
    }
}
