//! Cache wrappers that sit in front of a backing bus.

use std::collections::VecDeque;

use crate::{
    AccessKind, Address, AddressRange, BurstBus, BurstPhase, BurstRequest, BurstResponse, Bus,
    BusError, InterruptSet,
};

const WORD_BYTES: usize = 4;

/// Replacement policy used by set-associative cache banks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReplacementPolicy {
    #[default]
    RoundRobin,
    LeastRecentlyUsed,
}

/// Store commit policy for cached lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WritePolicy {
    #[default]
    WriteThrough,
    WriteBack,
}

/// Miss policy for store accesses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StoreAllocationPolicy {
    #[default]
    NoWriteAllocate,
    WriteAllocate,
}

/// Static cache geometry and address policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    line_count: usize,
    line_size: usize,
    associativity: usize,
    replacement_policy: ReplacementPolicy,
    write_policy: WritePolicy,
    store_allocation_policy: StoreAllocationPolicy,
    cached_ranges: Vec<AddressRange>,
}

impl CacheConfig {
    #[must_use]
    pub fn new(line_count: usize, cached_ranges: Vec<AddressRange>) -> Self {
        assert!(line_count > 0, "cache requires at least one line");
        let config = Self {
            line_count,
            line_size: WORD_BYTES,
            associativity: 1,
            replacement_policy: ReplacementPolicy::RoundRobin,
            write_policy: WritePolicy::WriteThrough,
            store_allocation_policy: StoreAllocationPolicy::NoWriteAllocate,
            cached_ranges,
        };
        config.validate_cached_ranges();
        config
    }

    #[must_use]
    pub fn with_line_size(mut self, line_size: usize) -> Self {
        assert!(
            line_size >= WORD_BYTES && line_size.is_power_of_two() && line_size % WORD_BYTES == 0,
            "cache line size must be a power-of-two multiple of four bytes"
        );
        self.line_size = line_size;
        self.validate_cached_ranges();
        self
    }

    #[must_use]
    pub fn with_associativity(mut self, associativity: usize) -> Self {
        assert!(
            associativity > 0,
            "cache associativity must be at least one"
        );
        assert!(
            self.line_count.is_multiple_of(associativity),
            "cache line count must be divisible by associativity"
        );
        self.associativity = associativity;
        self
    }

    #[must_use]
    pub fn with_replacement_policy(mut self, replacement_policy: ReplacementPolicy) -> Self {
        self.replacement_policy = replacement_policy;
        self
    }

    #[must_use]
    pub fn with_write_policy(mut self, write_policy: WritePolicy) -> Self {
        self.write_policy = write_policy;
        self
    }

    #[must_use]
    pub fn with_store_allocation_policy(
        mut self,
        store_allocation_policy: StoreAllocationPolicy,
    ) -> Self {
        self.store_allocation_policy = store_allocation_policy;
        self
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.line_count
    }

    #[must_use]
    pub fn line_size(&self) -> usize {
        self.line_size
    }

    #[must_use]
    pub fn associativity(&self) -> usize {
        self.associativity
    }

    #[must_use]
    pub fn set_count(&self) -> usize {
        self.line_count / self.associativity
    }

    #[must_use]
    pub fn replacement_policy(&self) -> ReplacementPolicy {
        self.replacement_policy
    }

    #[must_use]
    pub fn write_policy(&self) -> WritePolicy {
        self.write_policy
    }

    #[must_use]
    pub fn store_allocation_policy(&self) -> StoreAllocationPolicy {
        self.store_allocation_policy
    }

    #[must_use]
    pub fn cached_ranges(&self) -> &[AddressRange] {
        &self.cached_ranges
    }

    #[must_use]
    pub fn caches_address(&self, addr: Address) -> bool {
        self.cached_ranges.iter().any(|range| range.contains(addr))
    }

    fn validate_cached_ranges(&self) {
        let line_size = self.line_size as u64;
        for range in &self.cached_ranges {
            assert!(
                range.start % line_size == 0,
                "cacheable range start 0x{:08x} must align to cache line size {}",
                range.start,
                self.line_size
            );
            assert!(
                range.size % line_size == 0,
                "cacheable range size 0x{:08x} must be a whole number of cache lines of size {}",
                range.size,
                self.line_size
            );
        }
    }
}

/// Aggregate cache counters useful for validation and basic benchmarking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub read_hits: u64,
    pub read_misses: u64,
    pub refills: u64,
    pub refill_words: u64,
    pub evictions: u64,
    pub dirty_evictions: u64,
    pub write_backs: u64,
    pub write_back_words: u64,
    pub write_accesses: u64,
    pub bypassed_reads: u64,
    pub bypassed_writes: u64,
    pub invalidations: u64,
}

/// Split cache stats keep instruction and data paths visible separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SplitCacheStats {
    pub instruction: CacheStats,
    pub data: CacheStats,
}

/// Software-managed cache maintenance hooks for DMA and self-modifying-code experiments.
pub trait CacheMaintenance {
    fn write_back_range(&mut self, start: Address, len: u64) -> Result<(), BusError>;
    fn invalidate_range(&mut self, start: Address, len: u64) -> Result<(), BusError>;

    fn write_back_invalidate_range(&mut self, start: Address, len: u64) -> Result<(), BusError> {
        self.write_back_range(start, len)?;
        self.invalidate_range(start, len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheLine {
    valid: bool,
    dirty: bool,
    tag: u64,
    words: Box<[u32]>,
    last_used: u64,
}

impl CacheLine {
    fn new(words_per_line: usize) -> Self {
        Self {
            valid: false,
            dirty: false,
            tag: 0,
            words: vec![0; words_per_line].into_boxed_slice(),
            last_used: 0,
        }
    }

    fn reset(&mut self) {
        self.valid = false;
        self.dirty = false;
        self.tag = 0;
        self.last_used = 0;
        self.words.fill(0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheSet {
    lines: Vec<CacheLine>,
    next_victim: usize,
}

impl CacheSet {
    fn new(associativity: usize, words_per_line: usize) -> Self {
        let mut lines = Vec::with_capacity(associativity);
        for _ in 0..associativity {
            lines.push(CacheLine::new(words_per_line));
        }

        Self {
            lines,
            next_victim: 0,
        }
    }

    fn reset(&mut self) {
        for line in &mut self.lines {
            line.reset();
        }
        self.next_victim = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefillKind {
    Fetch,
    Load,
}

impl RefillKind {
    fn access_kind(self) -> AccessKind {
        match self {
            Self::Fetch => AccessKind::Fetch,
            Self::Load => AccessKind::Load,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DecodedAddress {
    line_base: Address,
    set_index: usize,
    tag: u64,
    word_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoreOp {
    addr: Address,
    width: usize,
    value: u32,
}

impl StoreOp {
    fn to_bytes(self) -> [u8; WORD_BYTES] {
        self.value.to_le_bytes()
    }

    fn apply(self, line_base: Address, words: &mut [u32]) {
        let word_index = ((self.addr - line_base) as usize) / WORD_BYTES;
        let byte_offset = ((self.addr - line_base) as usize) % WORD_BYTES;
        let mut word_bytes = words[word_index].to_le_bytes();
        let store_bytes = self.to_bytes();

        word_bytes[byte_offset..byte_offset + self.width]
            .copy_from_slice(&store_bytes[..self.width]);
        words[word_index] = u32::from_le_bytes(word_bytes);
    }

    fn commit<B>(self, inner: &mut B) -> Result<(), BusError>
    where
        B: Bus,
    {
        match self.width {
            1 => inner.store8(self.addr, self.value as u8),
            2 => inner.store16(self.addr, self.value as u16),
            4 => inner.store32(self.addr, self.value),
            _ => unreachable!("unsupported store width"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingWriteBack {
    line_base: Address,
    words: Box<[u32]>,
    burst_id: Option<u64>,
}

impl PendingWriteBack {
    fn new(line_base: Address, words: &[u32]) -> Self {
        Self {
            line_base,
            words: words.to_vec().into_boxed_slice(),
            burst_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRefill {
    line_base: Address,
    set_index: usize,
    tag: u64,
    way_index: usize,
    words: Box<[u32]>,
    kind: RefillKind,
    burst_id: Option<u64>,
}

impl PendingRefill {
    fn new(
        line_base: Address,
        set_index: usize,
        tag: u64,
        way_index: usize,
        line_words: usize,
        kind: RefillKind,
    ) -> Self {
        Self {
            line_base,
            set_index,
            tag,
            way_index,
            words: vec![0; line_words].into_boxed_slice(),
            kind,
            burst_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingLineOp {
    write_back: Option<PendingWriteBack>,
    refill: PendingRefill,
}

impl PendingLineOp {
    fn matches_line(&self, decoded: DecodedAddress) -> bool {
        self.refill.line_base == decoded.line_base
    }
}

enum BurstDrive<T> {
    Busy { remaining_cycles: u32 },
    Ready(T),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveCacheBurst {
    id: u64,
    request: BurstRequest,
    beat_index: usize,
    read_words: Box<[u32]>,
    phase: BurstPhase,
}

enum CacheBeatAdvance {
    Busy(u32),
    Completed(Option<u32>),
    Failed(BusError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaintenanceKind {
    WriteBack,
    Invalidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaintenanceRequest {
    start: Address,
    len: u64,
    kind: MaintenanceKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingBankWriteBack {
    line_bases: VecDeque<Address>,
    active_line_base: Option<Address>,
    active_write_back: Option<PendingWriteBack>,
}

impl PendingBankWriteBack {
    fn new(line_bases: VecDeque<Address>) -> Self {
        Self {
            line_bases,
            active_line_base: None,
            active_write_back: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DirectMaintenanceStage {
    LocalWriteBack(PendingBankWriteBack),
    Inner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingDirectMaintenance {
    request: MaintenanceRequest,
    stage: DirectMaintenanceStage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SplitMaintenanceStage {
    DataWriteBack(PendingBankWriteBack),
    Inner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSplitMaintenance {
    request: MaintenanceRequest,
    stage: SplitMaintenanceStage,
}

#[derive(Debug, Clone)]
struct CacheBank {
    config: CacheConfig,
    sets: Vec<CacheSet>,
    stats: CacheStats,
    pending: Option<PendingLineOp>,
    access_epoch: u64,
}

impl CacheBank {
    fn new(config: CacheConfig) -> Self {
        let words_per_line = config.line_size / WORD_BYTES;
        let set_count = config.set_count();
        let mut sets = Vec::with_capacity(set_count);
        for _ in 0..set_count {
            sets.push(CacheSet::new(config.associativity, words_per_line));
        }

        Self {
            config,
            sets,
            stats: CacheStats::default(),
            pending: None,
            access_epoch: 0,
        }
    }

    fn reset(&mut self) {
        for set in &mut self.sets {
            set.reset();
        }
        self.stats = CacheStats::default();
        self.pending = None;
        self.access_epoch = 0;
    }

    fn stats(&self) -> CacheStats {
        self.stats
    }

    fn config(&self) -> &CacheConfig {
        &self.config
    }

    fn line_words(&self) -> usize {
        self.config.line_size / WORD_BYTES
    }

    fn has_pending_activity(&self) -> bool {
        self.pending.is_some()
    }

    fn caches_address(&self, addr: Address) -> bool {
        self.config.caches_address(addr)
    }

    fn line_base(&self, addr: Address) -> Address {
        addr & !((self.config.line_size as u64) - 1)
    }

    fn line_bases_for_range(&self, start: Address, len: u64) -> VecDeque<Address> {
        let mut line_bases = VecDeque::new();
        if len == 0 {
            return line_bases;
        }

        let mut line_base = self.line_base(start);
        let end = start.checked_add(len - 1).unwrap_or(u64::MAX);
        loop {
            line_bases.push_back(line_base);
            if line_base >= end {
                break;
            }

            let Some(next_line_base) = line_base.checked_add(self.config.line_size as u64) else {
                break;
            };
            line_base = next_line_base;
        }

        line_bases
    }

    fn caches_line_containing(&self, addr: Address) -> bool {
        self.caches_line(self.line_base(addr))
    }

    fn caches_line(&self, line_base: Address) -> bool {
        let Some(line_end) = line_base.checked_add(self.config.line_size as u64) else {
            return false;
        };

        self.config
            .cached_ranges
            .iter()
            .any(|range| line_base >= range.start && line_end <= range.end())
    }

    fn decode(&self, addr: Address) -> DecodedAddress {
        let line_base = self.line_base(addr);
        let line_number = line_base / self.config.line_size as u64;
        let set_index = (line_number as usize) % self.config.set_count();
        let tag = line_number / self.config.set_count() as u64;
        let word_index = ((addr - line_base) as usize) / WORD_BYTES;

        DecodedAddress {
            line_base,
            set_index,
            tag,
            word_index,
        }
    }

    fn line_base_for_tag(&self, set_index: usize, tag: u64) -> Address {
        let line_number = tag * self.config.set_count() as u64 + set_index as u64;
        line_number * self.config.line_size as u64
    }

    fn lookup_way(&self, decoded: DecodedAddress) -> Option<usize> {
        self.sets[decoded.set_index]
            .lines
            .iter()
            .position(|line| line.valid && line.tag == decoded.tag)
    }

    fn read_word(&self, decoded: DecodedAddress, way_index: usize) -> u32 {
        self.sets[decoded.set_index].lines[way_index].words[decoded.word_index]
    }

    fn touch_line(&mut self, set_index: usize, way_index: usize) {
        self.access_epoch = self.access_epoch.wrapping_add(1);
        self.sets[set_index].lines[way_index].last_used = self.access_epoch;
    }

    fn select_victim(&mut self, set_index: usize) -> usize {
        let set = &mut self.sets[set_index];
        if let Some(index) = set.lines.iter().position(|line| !line.valid) {
            if self.config.replacement_policy == ReplacementPolicy::RoundRobin {
                set.next_victim = (index + 1) % set.lines.len();
            }
            return index;
        }

        match self.config.replacement_policy {
            ReplacementPolicy::RoundRobin => {
                let victim = set.next_victim;
                set.next_victim = (set.next_victim + 1) % set.lines.len();
                victim
            }
            ReplacementPolicy::LeastRecentlyUsed => set
                .lines
                .iter()
                .enumerate()
                .min_by_key(|(_, line)| line.last_used)
                .map(|(index, _)| index)
                .unwrap_or(0),
        }
    }

    fn start_pending(&mut self, decoded: DecodedAddress, kind: RefillKind, count_read_miss: bool) {
        let victim = self.select_victim(decoded.set_index);
        let (victim_valid, victim_dirty, victim_tag, victim_words) = {
            let line = &self.sets[decoded.set_index].lines[victim];
            (line.valid, line.dirty, line.tag, line.words.clone())
        };
        if victim_valid {
            self.stats.evictions += 1;
        }
        if self.config.write_policy == WritePolicy::WriteBack && victim_valid && victim_dirty {
            self.stats.dirty_evictions += 1;
        }

        let write_back =
            (self.config.write_policy == WritePolicy::WriteBack && victim_valid && victim_dirty)
                .then(|| {
                    PendingWriteBack::new(
                        self.line_base_for_tag(decoded.set_index, victim_tag),
                        &victim_words,
                    )
                });

        if count_read_miss {
            self.stats.read_misses += 1;
        }

        {
            let line = &mut self.sets[decoded.set_index].lines[victim];
            line.valid = false;
            line.dirty = false;
        }

        self.pending = Some(PendingLineOp {
            write_back,
            refill: PendingRefill::new(
                decoded.line_base,
                decoded.set_index,
                decoded.tag,
                victim,
                self.line_words(),
                kind,
            ),
        });
    }

    fn drive_burst_until_blocked<B>(
        &self,
        inner: &mut B,
        burst_id: &mut Option<u64>,
        request: BurstRequest,
    ) -> Result<BurstDrive<BurstResponse>, BusError>
    where
        B: BurstBus,
    {
        let id = match *burst_id {
            Some(id) => id,
            None => {
                let id = inner.submit_burst(request)?;
                *burst_id = Some(id);
                id
            }
        };

        loop {
            let phase = inner
                .burst_phase(id)
                .expect("submitted burst should remain active until completion");
            match phase {
                BurstPhase::Accepted { .. } => {
                    let next_phase = inner
                        .advance_burst(id)
                        .expect("submitted burst should advance");
                    match next_phase {
                        BurstPhase::Accepted { .. } => continue,
                        BurstPhase::InFlight {
                            remaining_cycles, ..
                        } => {
                            return Ok(BurstDrive::Busy { remaining_cycles });
                        }
                        BurstPhase::Ready { .. } => continue,
                        BurstPhase::Failed(_) => continue,
                    }
                }
                BurstPhase::InFlight {
                    remaining_cycles, ..
                } => {
                    return Ok(BurstDrive::Busy { remaining_cycles });
                }
                BurstPhase::Ready { .. } => {
                    *burst_id = None;
                    let response = inner
                        .take_burst_response(id)
                        .expect("ready burst should yield a response")?;
                    return Ok(BurstDrive::Ready(response));
                }
                BurstPhase::Failed(_) => {
                    *burst_id = None;
                    let error = inner
                        .take_burst_response(id)
                        .expect("failed burst should yield a terminal error")
                        .expect_err("failed burst should not produce a success response");
                    return Err(error);
                }
            }
        }
    }

    fn progress_write_back<B>(
        &self,
        inner: &mut B,
        write_back: &mut PendingWriteBack,
    ) -> Result<BurstDrive<usize>, BusError>
    where
        B: BurstBus,
    {
        let request = BurstRequest::write_words(write_back.line_base, write_back.words.clone());
        match self.drive_burst_until_blocked(inner, &mut write_back.burst_id, request)? {
            BurstDrive::Busy { remaining_cycles } => Ok(BurstDrive::Busy { remaining_cycles }),
            BurstDrive::Ready(BurstResponse::WriteComplete { beats }) => {
                Ok(BurstDrive::Ready(beats))
            }
            BurstDrive::Ready(BurstResponse::ReadWords(_)) => {
                unreachable!("write-back bursts should complete as writes")
            }
        }
    }

    fn progress_refill<B>(
        &self,
        inner: &mut B,
        refill: &mut PendingRefill,
    ) -> Result<BurstDrive<Box<[u32]>>, BusError>
    where
        B: BurstBus,
    {
        let request = BurstRequest::read_words(
            refill.line_base,
            refill.words.len(),
            refill.kind.access_kind(),
        );
        match self.drive_burst_until_blocked(inner, &mut refill.burst_id, request)? {
            BurstDrive::Busy { remaining_cycles } => Ok(BurstDrive::Busy { remaining_cycles }),
            BurstDrive::Ready(BurstResponse::ReadWords(words)) => Ok(BurstDrive::Ready(words)),
            BurstDrive::Ready(BurstResponse::WriteComplete { .. }) => {
                unreachable!("refill bursts should complete as reads")
            }
        }
    }

    fn continue_pending<B>(&mut self, inner: &mut B) -> Result<(), BusError>
    where
        B: BurstBus,
    {
        let Some(mut pending) = self.pending.take() else {
            return Ok(());
        };

        if let Some(write_back) = &mut pending.write_back {
            match self.progress_write_back(inner, write_back)? {
                BurstDrive::Busy { remaining_cycles } => {
                    self.pending = Some(pending);
                    return Err(BusError::Busy { remaining_cycles });
                }
                BurstDrive::Ready(beats) => {
                    self.stats.write_back_words += beats as u64;
                }
            }
            self.stats.write_backs += 1;
            pending.write_back = None;
        }

        match self.progress_refill(inner, &mut pending.refill)? {
            BurstDrive::Busy { remaining_cycles } => {
                self.pending = Some(pending);
                return Err(BusError::Busy { remaining_cycles });
            }
            BurstDrive::Ready(words) => {
                self.stats.refill_words += words.len() as u64;
                pending.refill.words = words;
            }
        }

        {
            let line = &mut self.sets[pending.refill.set_index].lines[pending.refill.way_index];
            line.valid = true;
            line.dirty = false;
            line.tag = pending.refill.tag;
            line.words.as_mut().copy_from_slice(&pending.refill.words);
        }
        self.stats.refills += 1;
        self.touch_line(pending.refill.set_index, pending.refill.way_index);
        Ok(())
    }

    fn ensure_line<B>(
        &mut self,
        inner: &mut B,
        addr: Address,
        kind: RefillKind,
    ) -> Result<(DecodedAddress, usize), BusError>
    where
        B: BurstBus,
    {
        let decoded = self.decode(addr);

        if let Some(pending) = &self.pending {
            if !pending.matches_line(decoded) {
                return Err(BusError::Busy {
                    remaining_cycles: 1,
                });
            }
            self.continue_pending(inner)?;
        }

        if let Some(way_index) = self.lookup_way(decoded) {
            return Ok((decoded, way_index));
        }

        self.start_pending(decoded, kind, true);
        self.continue_pending(inner)?;

        let way_index = self
            .lookup_way(decoded)
            .expect("completed refill should install a line");
        Ok((decoded, way_index))
    }

    fn load_word<B>(
        &mut self,
        inner: &mut B,
        addr: Address,
        kind: RefillKind,
    ) -> Result<u32, BusError>
    where
        B: BurstBus,
    {
        let (decoded, way_index) = self.ensure_line(inner, addr, kind)?;
        let word = self.read_word(decoded, way_index);
        self.stats.read_hits += 1;
        self.touch_line(decoded.set_index, way_index);
        Ok(word)
    }

    fn note_write_access(&mut self) {
        self.stats.write_accesses += 1;
    }

    fn note_bypassed_read(&mut self) {
        self.stats.bypassed_reads += 1;
    }

    fn note_bypassed_write(&mut self) {
        self.stats.bypassed_writes += 1;
    }

    fn update_cached_store(
        &mut self,
        decoded: DecodedAddress,
        way_index: usize,
        store: StoreOp,
        dirty: bool,
    ) {
        let line = &mut self.sets[decoded.set_index].lines[way_index];
        store.apply(decoded.line_base, line.words.as_mut());
        line.valid = true;
        line.tag = decoded.tag;
        line.dirty = dirty;
        self.touch_line(decoded.set_index, way_index);
    }

    fn store<B>(&mut self, inner: &mut B, store: StoreOp) -> Result<(), BusError>
    where
        B: BurstBus,
    {
        self.note_write_access();

        if !self.caches_address(store.addr) || !self.caches_line_containing(store.addr) {
            self.note_bypassed_write();
            return store.commit(inner);
        }

        let decoded = self.decode(store.addr);
        if let Some(pending) = &self.pending {
            if !pending.matches_line(decoded) {
                return Err(BusError::Busy {
                    remaining_cycles: 1,
                });
            }
            self.continue_pending(inner)?;
        }

        if let Some(way_index) = self.lookup_way(decoded) {
            return match self.config.write_policy {
                WritePolicy::WriteThrough => {
                    store.commit(inner)?;
                    self.update_cached_store(decoded, way_index, store, false);
                    Ok(())
                }
                WritePolicy::WriteBack => {
                    self.update_cached_store(decoded, way_index, store, true);
                    Ok(())
                }
            };
        }

        match self.config.store_allocation_policy {
            StoreAllocationPolicy::NoWriteAllocate => {
                self.note_bypassed_write();
                store.commit(inner)
            }
            StoreAllocationPolicy::WriteAllocate => {
                self.start_pending(decoded, RefillKind::Load, false);
                self.continue_pending(inner)?;

                let way_index = self
                    .lookup_way(decoded)
                    .expect("completed refill should install a line");
                match self.config.write_policy {
                    WritePolicy::WriteThrough => {
                        store.commit(inner)?;
                        self.update_cached_store(decoded, way_index, store, false);
                    }
                    WritePolicy::WriteBack => {
                        self.update_cached_store(decoded, way_index, store, true);
                    }
                }
                Ok(())
            }
        }
    }

    fn pending_write_back_for_line(&self, line_base: Address) -> Option<PendingWriteBack> {
        if !self.caches_line(line_base) {
            return None;
        }

        let decoded = self.decode(line_base);
        let way_index = self.lookup_way(decoded)?;
        let line = &self.sets[decoded.set_index].lines[way_index];
        (line.valid && line.dirty).then(|| PendingWriteBack::new(line_base, &line.words))
    }

    fn complete_write_back_for_line(&mut self, line_base: Address) {
        if !self.caches_line(line_base) {
            return;
        }

        let decoded = self.decode(line_base);
        let Some(way_index) = self.lookup_way(decoded) else {
            return;
        };

        self.sets[decoded.set_index].lines[way_index].dirty = false;
    }

    fn continue_write_back_range<B>(
        &mut self,
        inner: &mut B,
        pending: &mut PendingBankWriteBack,
    ) -> Result<bool, BusError>
    where
        B: BurstBus,
    {
        loop {
            if let Some(write_back) = &mut pending.active_write_back {
                match self.progress_write_back(inner, write_back)? {
                    BurstDrive::Busy { remaining_cycles } => {
                        return Err(BusError::Busy { remaining_cycles });
                    }
                    BurstDrive::Ready(beats) => {
                        self.stats.write_backs += 1;
                        self.stats.write_back_words += beats as u64;
                        if let Some(line_base) = pending.active_line_base.take() {
                            self.complete_write_back_for_line(line_base);
                        }
                        pending.active_write_back = None;
                    }
                }
                continue;
            }

            let Some(line_base) = pending.line_bases.pop_front() else {
                return Ok(true);
            };

            if let Some(write_back) = self.pending_write_back_for_line(line_base) {
                pending.active_line_base = Some(line_base);
                pending.active_write_back = Some(write_back);
            }
        }
    }

    fn invalidate_line_base(&mut self, line_base: Address) {
        if !self.caches_line(line_base) {
            return;
        }

        let decoded = self.decode(line_base);
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.matches_line(decoded))
        {
            self.pending = None;
        }

        let set = &mut self.sets[decoded.set_index];
        if let Some(line) = set
            .lines
            .iter_mut()
            .find(|line| line.valid && line.tag == decoded.tag)
        {
            line.valid = false;
            line.dirty = false;
            self.stats.invalidations += 1;
        }
    }

    fn invalidate_range(&mut self, start: Address, len: u64) {
        for line_base in self.line_bases_for_range(start, len) {
            self.invalidate_line_base(line_base);
        }
    }

    fn invalidate_line(&mut self, addr: Address) {
        self.invalidate_line_base(self.line_base(addr));
    }
}

/// A unified cache wrapper retained for compatibility with earlier milestones.
#[derive(Debug)]
pub struct DirectMappedCache<B> {
    inner: B,
    bank: CacheBank,
    active_burst: Option<ActiveCacheBurst>,
    next_burst_id: u64,
    maintenance: Option<PendingDirectMaintenance>,
}

impl<B> DirectMappedCache<B>
where
    B: Bus,
{
    #[must_use]
    pub fn new(inner: B, config: CacheConfig) -> Self {
        Self {
            inner,
            bank: CacheBank::new(config),
            active_burst: None,
            next_burst_id: 0,
            maintenance: None,
        }
    }

    #[must_use]
    pub fn inner(&self) -> &B {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }

    #[must_use]
    pub fn stats(&self) -> CacheStats {
        self.bank.stats()
    }

    #[must_use]
    pub fn config(&self) -> &CacheConfig {
        self.bank.config()
    }
}

impl<B> DirectMappedCache<B>
where
    B: BurstBus,
{
    fn active_burst_remaining_cycles(&self) -> u32 {
        match self.active_burst.as_ref().map(|active| &active.phase) {
            Some(BurstPhase::InFlight {
                remaining_cycles, ..
            }) => *remaining_cycles,
            Some(BurstPhase::Accepted { .. })
            | Some(BurstPhase::Ready { .. })
            | Some(BurstPhase::Failed(_)) => 1,
            None => 0,
        }
    }

    fn busy_remaining_cycles(&self) -> u32 {
        let active_cycles = self.active_burst_remaining_cycles();
        if active_cycles != 0 {
            active_cycles
        } else if self.maintenance.is_some()
            || self.bank.has_pending_activity()
            || self.inner.is_busy()
        {
            1
        } else {
            0
        }
    }

    fn execute_burst_beat(
        &mut self,
        request: &BurstRequest,
        beat_index: usize,
    ) -> Result<Option<u32>, BusError> {
        let addr = request.beat_addr(beat_index);
        match request {
            BurstRequest::ReadWords { kind, .. } => match kind {
                AccessKind::Fetch => <Self as Bus>::fetch32(self, addr).map(Some),
                AccessKind::Load => <Self as Bus>::load32(self, addr).map(Some),
                AccessKind::Store => unreachable!("read burst cannot carry store access kind"),
            },
            BurstRequest::WriteWords { words, .. } => {
                <Self as Bus>::store32(self, addr, words[beat_index]).map(|()| None)
            }
        }
    }

    fn advance_active_burst(&mut self) {
        let Some((request, beat_index, phase)) = self.active_burst.as_ref().map(|active| {
            (
                active.request.clone(),
                active.beat_index,
                active.phase.clone(),
            )
        }) else {
            return;
        };

        if matches!(phase, BurstPhase::Ready { .. } | BurstPhase::Failed(_)) {
            return;
        }

        let total_beats = request.beats();
        let next_state = match self.execute_burst_beat(&request, beat_index) {
            Ok(read_word) => CacheBeatAdvance::Completed(read_word),
            Err(BusError::Busy { remaining_cycles }) => CacheBeatAdvance::Busy(remaining_cycles),
            Err(error) => CacheBeatAdvance::Failed(error),
        };

        if let Some(active) = &mut self.active_burst {
            match next_state {
                CacheBeatAdvance::Busy(remaining_cycles) => {
                    active.phase = BurstPhase::InFlight {
                        beat_index,
                        total_beats,
                        remaining_cycles,
                    };
                }
                CacheBeatAdvance::Completed(read_word) => {
                    if let Some(word) = read_word {
                        active.read_words[beat_index] = word;
                    }

                    let next_beat_index = beat_index + 1;
                    active.beat_index = next_beat_index;
                    active.phase = if next_beat_index == total_beats {
                        BurstPhase::Ready {
                            completed_beats: total_beats,
                        }
                    } else {
                        BurstPhase::Accepted {
                            beat_index: next_beat_index,
                            total_beats,
                        }
                    };
                }
                CacheBeatAdvance::Failed(error) => {
                    active.phase = BurstPhase::Failed(error);
                }
            }
        }
    }
}

impl<B> DirectMappedCache<B>
where
    B: BurstBus + CacheMaintenance,
{
    fn ensure_maintenance_request(&mut self, request: MaintenanceRequest) -> Result<(), BusError> {
        if let Some(pending) = &self.maintenance {
            if pending.request == request {
                return Ok(());
            }

            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }

        if self.active_burst.is_some() || self.bank.has_pending_activity() {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }

        let stage = match request.kind {
            MaintenanceKind::WriteBack => {
                DirectMaintenanceStage::LocalWriteBack(PendingBankWriteBack::new(
                    self.bank.line_bases_for_range(request.start, request.len),
                ))
            }
            MaintenanceKind::Invalidate => {
                self.bank.invalidate_range(request.start, request.len);
                DirectMaintenanceStage::Inner
            }
        };

        self.maintenance = Some(PendingDirectMaintenance { request, stage });
        Ok(())
    }

    fn continue_maintenance(&mut self) -> Result<(), BusError> {
        let Some(mut pending) = self.maintenance.take() else {
            return Ok(());
        };

        loop {
            match &mut pending.stage {
                DirectMaintenanceStage::LocalWriteBack(write_back) => {
                    match self
                        .bank
                        .continue_write_back_range(&mut self.inner, write_back)
                    {
                        Ok(true) => {
                            pending.stage = DirectMaintenanceStage::Inner;
                        }
                        Ok(false) => {
                            unreachable!("bank write-back should either complete or block")
                        }
                        Err(error) => {
                            self.maintenance = Some(pending);
                            return Err(error);
                        }
                    }
                }
                DirectMaintenanceStage::Inner => {
                    let result = match pending.request.kind {
                        MaintenanceKind::WriteBack => self
                            .inner
                            .write_back_range(pending.request.start, pending.request.len),
                        MaintenanceKind::Invalidate => self
                            .inner
                            .invalidate_range(pending.request.start, pending.request.len),
                    };

                    match result {
                        Ok(()) => return Ok(()),
                        Err(error @ BusError::Busy { .. }) => {
                            self.maintenance = Some(pending);
                            return Err(error);
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }
}

/// A split L1 with independent instruction and data banks.
#[derive(Debug)]
pub struct SplitL1Cache<B> {
    inner: B,
    instruction: CacheBank,
    data: CacheBank,
    maintenance: Option<PendingSplitMaintenance>,
}

impl<B> SplitL1Cache<B>
where
    B: Bus,
{
    #[must_use]
    pub fn new(inner: B, instruction: CacheConfig, data: CacheConfig) -> Self {
        Self {
            inner,
            instruction: CacheBank::new(instruction),
            data: CacheBank::new(data),
            maintenance: None,
        }
    }

    #[must_use]
    pub fn inner(&self) -> &B {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }

    #[must_use]
    pub fn stats(&self) -> SplitCacheStats {
        SplitCacheStats {
            instruction: self.instruction.stats(),
            data: self.data.stats(),
        }
    }

    #[must_use]
    pub fn instruction_config(&self) -> &CacheConfig {
        self.instruction.config()
    }

    #[must_use]
    pub fn data_config(&self) -> &CacheConfig {
        self.data.config()
    }
}

impl<B> SplitL1Cache<B>
where
    B: BurstBus + CacheMaintenance,
{
    fn maintenance_busy_cycles(&self) -> u32 {
        if self.maintenance.is_some()
            || self.instruction.has_pending_activity()
            || self.data.has_pending_activity()
            || self.inner.is_busy()
        {
            1
        } else {
            0
        }
    }

    fn ensure_maintenance_request(&mut self, request: MaintenanceRequest) -> Result<(), BusError> {
        if let Some(pending) = &self.maintenance {
            if pending.request == request {
                return Ok(());
            }

            return Err(BusError::Busy {
                remaining_cycles: self.maintenance_busy_cycles().max(1),
            });
        }

        if self.instruction.has_pending_activity() || self.data.has_pending_activity() {
            return Err(BusError::Busy {
                remaining_cycles: self.maintenance_busy_cycles().max(1),
            });
        }

        let stage = match request.kind {
            MaintenanceKind::WriteBack => {
                SplitMaintenanceStage::DataWriteBack(PendingBankWriteBack::new(
                    self.data.line_bases_for_range(request.start, request.len),
                ))
            }
            MaintenanceKind::Invalidate => {
                self.data.invalidate_range(request.start, request.len);
                self.instruction
                    .invalidate_range(request.start, request.len);
                SplitMaintenanceStage::Inner
            }
        };

        self.maintenance = Some(PendingSplitMaintenance { request, stage });
        Ok(())
    }

    fn continue_maintenance(&mut self) -> Result<(), BusError> {
        let Some(mut pending) = self.maintenance.take() else {
            return Ok(());
        };

        loop {
            match &mut pending.stage {
                SplitMaintenanceStage::DataWriteBack(write_back) => {
                    match self
                        .data
                        .continue_write_back_range(&mut self.inner, write_back)
                    {
                        Ok(true) => {
                            pending.stage = SplitMaintenanceStage::Inner;
                        }
                        Ok(false) => {
                            unreachable!("bank write-back should either complete or block")
                        }
                        Err(error) => {
                            self.maintenance = Some(pending);
                            return Err(error);
                        }
                    }
                }
                SplitMaintenanceStage::Inner => {
                    let result = match pending.request.kind {
                        MaintenanceKind::WriteBack => self
                            .inner
                            .write_back_range(pending.request.start, pending.request.len),
                        MaintenanceKind::Invalidate => self
                            .inner
                            .invalidate_range(pending.request.start, pending.request.len),
                    };

                    match result {
                        Ok(()) => return Ok(()),
                        Err(error @ BusError::Busy { .. }) => {
                            self.maintenance = Some(pending);
                            return Err(error);
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
        }
    }
}

fn extract_byte(word: u32, addr: Address) -> u8 {
    word.to_le_bytes()[(addr & 0b11) as usize]
}

fn extract_half(word: u32, addr: Address) -> u16 {
    let bytes = word.to_le_bytes();
    let offset = (addr & 0b11) as usize;
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn load_cached_byte<B>(bank: &mut CacheBank, inner: &mut B, addr: Address) -> Result<u8, BusError>
where
    B: BurstBus,
{
    let word = bank.load_word(inner, addr & !0b11, RefillKind::Load)?;
    Ok(extract_byte(word, addr))
}

fn load_cached_half<B>(bank: &mut CacheBank, inner: &mut B, addr: Address) -> Result<u16, BusError>
where
    B: BurstBus,
{
    let word = bank.load_word(inner, addr & !0b11, RefillKind::Load)?;
    Ok(extract_half(word, addr))
}

impl<B> Bus for DirectMappedCache<B>
where
    B: BurstBus,
{
    fn reset(&mut self) {
        self.bank.reset();
        self.active_burst = None;
        self.next_burst_id = 0;
        self.maintenance = None;
        self.inner.reset();
    }

    fn fetch32(&mut self, addr: Address) -> Result<u32, BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.bank.caches_address(addr) || !self.bank.caches_line_containing(addr) {
            self.bank.note_bypassed_read();
            return self.inner.fetch32(addr);
        }

        self.bank
            .load_word(&mut self.inner, addr, RefillKind::Fetch)
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }
        if !self.bank.caches_address(addr) || !self.bank.caches_line_containing(addr) {
            self.bank.note_bypassed_read();
            return self.inner.load8(addr);
        }

        load_cached_byte(&mut self.bank, &mut self.inner, addr)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }
        self.bank.store(
            &mut self.inner,
            StoreOp {
                addr,
                width: 1,
                value: u32::from(value),
            },
        )
    }

    fn load16(&mut self, addr: Address) -> Result<u16, BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        if !self.bank.caches_address(addr)
            || !self.bank.caches_address(addr + 1)
            || !self.bank.caches_line_containing(addr)
        {
            self.bank.note_bypassed_read();
            return self.inner.load16(addr);
        }

        load_cached_half(&mut self.bank, &mut self.inner, addr)
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.bank.caches_address(addr) || !self.bank.caches_line_containing(addr) {
            self.bank.note_bypassed_read();
            return self.inner.load32(addr);
        }

        self.bank.load_word(&mut self.inner, addr, RefillKind::Load)
    }

    fn store16(&mut self, addr: Address, value: u16) -> Result<(), BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        self.bank.store(
            &mut self.inner,
            StoreOp {
                addr,
                width: 2,
                value: u32::from(value),
            },
        )
    }

    fn store32(&mut self, addr: Address, value: u32) -> Result<(), BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        self.bank.store(
            &mut self.inner,
            StoreOp {
                addr,
                width: 4,
                value,
            },
        )
    }

    fn tick(&mut self) {
        self.inner.tick();
        self.advance_active_burst();
    }

    fn machine_time(&self) -> Option<u64> {
        self.inner.machine_time()
    }

    fn is_busy(&self) -> bool {
        self.maintenance.is_some()
            || self.active_burst.is_some()
            || self.bank.has_pending_activity()
            || self.inner.is_busy()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.pending_interrupts()
    }
}

impl<B> BurstBus for DirectMappedCache<B>
where
    B: BurstBus,
{
    fn submit_burst(&mut self, request: BurstRequest) -> Result<u64, BusError> {
        if self.maintenance.is_some()
            || self.active_burst.is_some()
            || self.bank.has_pending_activity()
            || self.inner.is_busy()
        {
            return Err(BusError::Busy {
                remaining_cycles: self.busy_remaining_cycles().max(1),
            });
        }

        let total_beats = request.beats();
        assert!(
            total_beats > 0,
            "burst request must contain at least one beat"
        );

        let id = self.next_burst_id;
        self.next_burst_id = self.next_burst_id.wrapping_add(1);
        self.active_burst = Some(ActiveCacheBurst {
            id,
            request,
            beat_index: 0,
            read_words: vec![0; total_beats].into_boxed_slice(),
            phase: BurstPhase::Accepted {
                beat_index: 0,
                total_beats,
            },
        });
        Ok(id)
    }

    fn burst_phase(&self, id: u64) -> Option<BurstPhase> {
        self.active_burst
            .as_ref()
            .filter(|active| active.id == id)
            .map(|active| active.phase.clone())
    }

    fn advance_burst(&mut self, id: u64) -> Option<BurstPhase> {
        if self
            .active_burst
            .as_ref()
            .is_none_or(|active| active.id != id)
        {
            return None;
        }

        self.advance_active_burst();
        self.burst_phase(id)
    }

    fn take_burst_response(&mut self, id: u64) -> Option<Result<BurstResponse, BusError>> {
        let active = self.active_burst.as_ref()?;
        if active.id != id {
            return None;
        }

        match &active.phase {
            BurstPhase::Accepted { .. } | BurstPhase::InFlight { .. } => None,
            BurstPhase::Ready { completed_beats } => {
                let completed_beats = *completed_beats;
                let active = self
                    .active_burst
                    .take()
                    .expect("ready burst should still be present");
                let response = match active.request {
                    BurstRequest::ReadWords { .. } => BurstResponse::ReadWords(active.read_words),
                    BurstRequest::WriteWords { .. } => BurstResponse::WriteComplete {
                        beats: completed_beats,
                    },
                };
                Some(Ok(response))
            }
            BurstPhase::Failed(_) => {
                let active = self
                    .active_burst
                    .take()
                    .expect("failed burst should still be present");
                let BurstPhase::Failed(error) = active.phase else {
                    unreachable!("failed burst must preserve its terminal phase");
                };
                Some(Err(error))
            }
        }
    }
}

impl<B> CacheMaintenance for DirectMappedCache<B>
where
    B: BurstBus + CacheMaintenance,
{
    fn write_back_range(&mut self, start: Address, len: u64) -> Result<(), BusError> {
        let request = MaintenanceRequest {
            start,
            len,
            kind: MaintenanceKind::WriteBack,
        };
        self.ensure_maintenance_request(request)?;
        self.continue_maintenance()
    }

    fn invalidate_range(&mut self, start: Address, len: u64) -> Result<(), BusError> {
        let request = MaintenanceRequest {
            start,
            len,
            kind: MaintenanceKind::Invalidate,
        };
        self.ensure_maintenance_request(request)?;
        self.continue_maintenance()
    }
}

impl<B> Bus for SplitL1Cache<B>
where
    B: BurstBus,
{
    fn reset(&mut self) {
        self.instruction.reset();
        self.data.reset();
        self.maintenance = None;
        self.inner.reset();
    }

    fn fetch32(&mut self, addr: Address) -> Result<u32, BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: 1,
            });
        }
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.instruction.caches_address(addr) || !self.instruction.caches_line_containing(addr)
        {
            self.instruction.note_bypassed_read();
            return self.inner.fetch32(addr);
        }

        self.instruction
            .load_word(&mut self.inner, addr, RefillKind::Fetch)
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: 1,
            });
        }
        if !self.data.caches_address(addr) || !self.data.caches_line_containing(addr) {
            self.data.note_bypassed_read();
            return self.inner.load8(addr);
        }

        load_cached_byte(&mut self.data, &mut self.inner, addr)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: 1,
            });
        }
        self.data.store(
            &mut self.inner,
            StoreOp {
                addr,
                width: 1,
                value: u32::from(value),
            },
        )?;
        self.instruction.invalidate_line(addr);
        Ok(())
    }

    fn load16(&mut self, addr: Address) -> Result<u16, BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: 1,
            });
        }
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        if !self.data.caches_address(addr)
            || !self.data.caches_address(addr + 1)
            || !self.data.caches_line_containing(addr)
        {
            self.data.note_bypassed_read();
            return self.inner.load16(addr);
        }

        load_cached_half(&mut self.data, &mut self.inner, addr)
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: 1,
            });
        }
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.data.caches_address(addr) || !self.data.caches_line_containing(addr) {
            self.data.note_bypassed_read();
            return self.inner.load32(addr);
        }

        self.data.load_word(&mut self.inner, addr, RefillKind::Load)
    }

    fn store16(&mut self, addr: Address, value: u16) -> Result<(), BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: 1,
            });
        }
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        self.data.store(
            &mut self.inner,
            StoreOp {
                addr,
                width: 2,
                value: u32::from(value),
            },
        )?;
        self.instruction.invalidate_line(addr);
        Ok(())
    }

    fn store32(&mut self, addr: Address, value: u32) -> Result<(), BusError> {
        if self.maintenance.is_some() {
            return Err(BusError::Busy {
                remaining_cycles: 1,
            });
        }
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        self.data.store(
            &mut self.inner,
            StoreOp {
                addr,
                width: 4,
                value,
            },
        )?;
        self.instruction.invalidate_line(addr);
        Ok(())
    }

    fn tick(&mut self) {
        self.inner.tick();
    }

    fn machine_time(&self) -> Option<u64> {
        self.inner.machine_time()
    }

    fn is_busy(&self) -> bool {
        self.maintenance.is_some()
            || self.instruction.has_pending_activity()
            || self.data.has_pending_activity()
            || self.inner.is_busy()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.pending_interrupts()
    }
}

impl<B> CacheMaintenance for SplitL1Cache<B>
where
    B: BurstBus + CacheMaintenance,
{
    fn write_back_range(&mut self, start: Address, len: u64) -> Result<(), BusError> {
        let request = MaintenanceRequest {
            start,
            len,
            kind: MaintenanceKind::WriteBack,
        };
        self.ensure_maintenance_request(request)?;
        self.continue_maintenance()
    }

    fn invalidate_range(&mut self, start: Address, len: u64) -> Result<(), BusError> {
        let request = MaintenanceRequest {
            start,
            len,
            kind: MaintenanceKind::Invalidate,
        };
        self.ensure_maintenance_request(request)?;
        self.continue_maintenance()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AccessKind, AddressRange, Addressable, BurstBus, BurstPhase, BurstRequest, BurstResponse,
        Bus, BusError, MemoryMap,
    };

    use super::{
        CacheConfig, CacheMaintenance, DirectMappedCache, ReplacementPolicy, SplitL1Cache,
        StoreAllocationPolicy, WritePolicy,
    };

    #[derive(Debug, Clone)]
    struct WordDevice {
        range: AddressRange,
        data: Vec<u8>,
        latency_cycles: u32,
    }

    impl WordDevice {
        fn from_words(base: u64, words: &[u32], latency_cycles: u32) -> Self {
            let mut data = Vec::with_capacity(words.len() * 4);
            for word in words {
                data.extend_from_slice(&word.to_le_bytes());
            }

            Self {
                range: AddressRange::new(base, data.len() as u64),
                data,
                latency_cycles,
            }
        }

        fn new_zeroed(base: u64, size: usize, latency_cycles: u32) -> Self {
            Self {
                range: AddressRange::new(base, size as u64),
                data: vec![0; size],
                latency_cycles,
            }
        }

        fn offset(&self, addr: u64) -> Result<usize, BusError> {
            if !self.range.contains(addr) {
                return Err(BusError::UnmappedAddress { addr });
            }

            Ok((addr - self.range.start) as usize)
        }
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    struct TrafficCounters {
        read_bursts: u64,
        write_bursts: u64,
        fetch32_calls: u64,
        load32_calls: u64,
        store32_calls: u64,
    }

    #[derive(Debug)]
    struct TrackingBus<B> {
        inner: B,
        counters: TrafficCounters,
    }

    impl<B> TrackingBus<B> {
        fn new(inner: B) -> Self {
            Self {
                inner,
                counters: TrafficCounters::default(),
            }
        }

        fn inner_mut(&mut self) -> &mut B {
            &mut self.inner
        }

        fn counters(&self) -> TrafficCounters {
            self.counters
        }
    }

    impl<B> Bus for TrackingBus<B>
    where
        B: Bus,
    {
        fn reset(&mut self) {
            self.inner.reset();
        }

        fn fetch32(&mut self, addr: u64) -> Result<u32, BusError> {
            self.counters.fetch32_calls += 1;
            self.inner.fetch32(addr)
        }

        fn load8(&mut self, addr: u64) -> Result<u8, BusError> {
            self.inner.load8(addr)
        }

        fn store8(&mut self, addr: u64, value: u8) -> Result<(), BusError> {
            self.inner.store8(addr, value)
        }

        fn load16(&mut self, addr: u64) -> Result<u16, BusError> {
            self.inner.load16(addr)
        }

        fn load32(&mut self, addr: u64) -> Result<u32, BusError> {
            self.counters.load32_calls += 1;
            self.inner.load32(addr)
        }

        fn store16(&mut self, addr: u64, value: u16) -> Result<(), BusError> {
            self.inner.store16(addr, value)
        }

        fn store32(&mut self, addr: u64, value: u32) -> Result<(), BusError> {
            self.counters.store32_calls += 1;
            self.inner.store32(addr, value)
        }

        fn tick(&mut self) {
            self.inner.tick();
        }

        fn is_busy(&self) -> bool {
            self.inner.is_busy()
        }

        fn pending_interrupts(&self) -> crate::InterruptSet {
            self.inner.pending_interrupts()
        }
    }

    impl<B> BurstBus for TrackingBus<B>
    where
        B: BurstBus,
    {
        fn submit_burst(&mut self, request: BurstRequest) -> Result<u64, BusError> {
            match &request {
                BurstRequest::ReadWords { .. } => self.counters.read_bursts += 1,
                BurstRequest::WriteWords { .. } => self.counters.write_bursts += 1,
            }
            self.inner.submit_burst(request)
        }

        fn burst_phase(&self, id: u64) -> Option<BurstPhase> {
            self.inner.burst_phase(id)
        }

        fn advance_burst(&mut self, id: u64) -> Option<BurstPhase> {
            self.inner.advance_burst(id)
        }

        fn take_burst_response(&mut self, id: u64) -> Option<Result<BurstResponse, BusError>> {
            self.inner.take_burst_response(id)
        }
    }

    impl Addressable for WordDevice {
        fn name(&self) -> &'static str {
            "word-device"
        }

        fn address_range(&self) -> AddressRange {
            self.range
        }

        fn access_latency(&self, _addr: u64, _kind: AccessKind, _width: usize) -> u32 {
            self.latency_cycles
        }

        fn load8(&mut self, addr: u64) -> Result<u8, BusError> {
            let offset = self.offset(addr)?;
            self.data
                .get(offset)
                .copied()
                .ok_or(BusError::UnmappedAddress { addr })
        }

        fn store8(&mut self, addr: u64, value: u8) -> Result<(), BusError> {
            let offset = self.offset(addr)?;
            let byte = self
                .data
                .get_mut(offset)
                .ok_or(BusError::UnmappedAddress { addr })?;
            *byte = value;
            Ok(())
        }
    }

    fn retry_fetch32<B>(bus: &mut B, addr: u64) -> u32
    where
        B: Bus,
    {
        loop {
            match bus.fetch32(addr) {
                Ok(word) => return word,
                Err(BusError::Busy { .. }) => bus.tick(),
                Err(error) => panic!("unexpected fetch error: {error}"),
            }
        }
    }

    fn retry_load32<B>(bus: &mut B, addr: u64) -> u32
    where
        B: Bus,
    {
        loop {
            match bus.load32(addr) {
                Ok(word) => return word,
                Err(BusError::Busy { .. }) => bus.tick(),
                Err(error) => panic!("unexpected load error: {error}"),
            }
        }
    }

    fn retry_store32<B>(bus: &mut B, addr: u64, value: u32)
    where
        B: Bus,
    {
        loop {
            match bus.store32(addr, value) {
                Ok(()) => return,
                Err(BusError::Busy { .. }) => bus.tick(),
                Err(error) => panic!("unexpected store error: {error}"),
            }
        }
    }

    #[test]
    fn unified_cache_hits_after_first_miss_on_cached_rom_word() {
        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(0, &[0x0050_0093, 0x0000_006f], 1))
            .expect("rom should map");

        let mut cache = DirectMappedCache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(0, 0x1000)]),
        );

        let error = cache.fetch32(0).expect_err("cold miss should stall");
        assert_eq!(
            error,
            crate::BusError::Busy {
                remaining_cycles: 1
            }
        );
        cache.tick();

        assert_eq!(
            cache.fetch32(0).expect("retry should fill line"),
            0x0050_0093
        );
        assert_eq!(
            cache.fetch32(0).expect("second access should hit"),
            0x0050_0093
        );
        assert_eq!(cache.stats().read_misses, 1);
        assert_eq!(cache.stats().read_hits, 2);
        assert_eq!(cache.stats().refills, 1);
        assert_eq!(cache.stats().refill_words, 1);
    }

    #[test]
    fn line_refill_brings_in_neighbor_words() {
        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(
                0,
                &[0x0050_0093, 0x00a0_0113, 0x0020_81b3, 0x0000_006f],
                1,
            ))
            .expect("rom should map");

        let mut cache = DirectMappedCache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(0, 0x1000)]).with_line_size(16),
        );

        assert_eq!(retry_fetch32(&mut cache, 0), 0x0050_0093);
        assert_eq!(cache.stats().read_misses, 1);
        assert_eq!(cache.stats().refills, 1);
        assert_eq!(cache.stats().refill_words, 4);

        assert_eq!(
            cache.fetch32(12).expect("line neighbor should hit"),
            0x0000_006f
        );
        assert_eq!(cache.stats().read_hits, 2);
        assert_eq!(cache.stats().read_misses, 1);
    }

    #[test]
    fn write_through_store_hit_updates_cached_line() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::new_zeroed(RAM_BASE, 0x1000, 1))
            .expect("ram should map");

        let mut cache = DirectMappedCache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(RAM_BASE, 0x1000)]),
        );

        let _ = cache.load32(RAM_BASE).expect_err("cold load should stall");
        cache.tick();
        assert_eq!(cache.load32(RAM_BASE).expect("fill should succeed"), 0);

        let _ = cache
            .store32(RAM_BASE, 9)
            .expect_err("write-through store should stall");
        cache.tick();
        cache
            .store32(RAM_BASE, 9)
            .expect("retry should complete write-through");

        assert_eq!(cache.load32(RAM_BASE).expect("updated line should hit"), 9);
        assert_eq!(retry_load32(cache.inner_mut(), RAM_BASE), 9);
        assert_eq!(cache.stats().invalidations, 0);
        assert_eq!(cache.stats().refills, 1);
        assert_eq!(cache.stats().refill_words, 1);
        assert_eq!(cache.stats().bypassed_writes, 0);
    }

    #[test]
    fn no_write_allocate_store_miss_does_not_fill_cache() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::new_zeroed(RAM_BASE, 0x1000, 0))
            .expect("ram should map");

        let mut cache = DirectMappedCache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(RAM_BASE, 0x1000)]),
        );

        cache
            .store32(RAM_BASE, 7)
            .expect("cold no-write-allocate store should bypass");
        assert_eq!(cache.stats().refills, 0);
        assert_eq!(cache.stats().read_misses, 0);
        assert_eq!(cache.stats().bypassed_writes, 1);

        assert_eq!(cache.load32(RAM_BASE).expect("first read should refill"), 7);
        assert_eq!(cache.stats().read_misses, 1);
        assert_eq!(cache.stats().refills, 1);
        assert_eq!(cache.stats().refill_words, 1);
    }

    #[test]
    fn write_back_write_allocate_delays_memory_update_until_eviction() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::new_zeroed(RAM_BASE, 0x1000, 0))
            .expect("ram should map");

        let mut cache = DirectMappedCache::new(
            memory,
            CacheConfig::new(1, vec![crate::AddressRange::new(RAM_BASE, 0x1000)])
                .with_write_policy(WritePolicy::WriteBack)
                .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );

        retry_store32(&mut cache, RAM_BASE, 9);
        assert_eq!(retry_load32(&mut cache, RAM_BASE), 9);
        assert_eq!(
            cache
                .inner_mut()
                .load32(RAM_BASE)
                .expect("dirty line should not be visible in memory yet"),
            0
        );

        retry_store32(&mut cache, RAM_BASE + 4, 11);
        assert_eq!(
            cache
                .inner_mut()
                .load32(RAM_BASE)
                .expect("eviction should write dirty line back"),
            9
        );
        assert_eq!(cache.stats().evictions, 1);
        assert_eq!(cache.stats().dirty_evictions, 1);
        assert_eq!(cache.stats().write_backs, 1);
        assert_eq!(cache.stats().write_back_words, 1);
        assert_eq!(cache.stats().refills, 2);
        assert_eq!(cache.stats().refill_words, 2);
    }

    #[test]
    fn write_back_range_flushes_dirty_line_without_eviction() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::new_zeroed(RAM_BASE, 0x1000, 1))
            .expect("ram should map");

        let mut cache = DirectMappedCache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(RAM_BASE, 0x1000)])
                .with_write_policy(WritePolicy::WriteBack)
                .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );

        retry_store32(&mut cache, RAM_BASE, 0xdead_beef);
        assert_eq!(retry_load32(cache.inner_mut(), RAM_BASE), 0);

        loop {
            match cache.write_back_range(RAM_BASE, 4) {
                Ok(()) => break,
                Err(BusError::Busy { .. }) => cache.tick(),
                Err(error) => panic!("write-back maintenance failed: {error}"),
            }
        }

        assert_eq!(retry_load32(cache.inner_mut(), RAM_BASE), 0xdead_beef);
        assert_eq!(
            cache
                .load32(RAM_BASE)
                .expect("clean line should stay cached"),
            0xdead_beef
        );
        assert_eq!(cache.stats().write_backs, 1);
        assert_eq!(cache.stats().write_back_words, 1);
        assert_eq!(cache.stats().dirty_evictions, 0);
    }

    #[test]
    fn line_refills_and_write_backs_use_lower_bursts() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::new_zeroed(RAM_BASE, 0x1000, 1))
            .expect("ram should map");

        let mut cache = DirectMappedCache::new(
            TrackingBus::new(memory),
            CacheConfig::new(1, vec![crate::AddressRange::new(RAM_BASE, 0x1000)])
                .with_line_size(16)
                .with_write_policy(WritePolicy::WriteBack)
                .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );

        assert_eq!(retry_load32(&mut cache, RAM_BASE), 0);
        retry_store32(&mut cache, RAM_BASE, 1);
        retry_store32(&mut cache, RAM_BASE + 4, 2);
        retry_store32(&mut cache, RAM_BASE + 8, 3);
        retry_store32(&mut cache, RAM_BASE + 12, 4);
        assert_eq!(retry_load32(&mut cache, RAM_BASE + 16), 0);

        assert_eq!(
            cache.inner().counters(),
            TrafficCounters {
                read_bursts: 2,
                write_bursts: 1,
                fetch32_calls: 0,
                load32_calls: 0,
                store32_calls: 0,
            }
        );
        assert_eq!(cache.stats().write_back_words, 4);
        assert_eq!(cache.stats().refill_words, 8);

        let memory = cache.inner_mut().inner_mut();
        assert_eq!(retry_load32(memory, RAM_BASE), 1);
        assert_eq!(retry_load32(memory, RAM_BASE + 4), 2);
        assert_eq!(retry_load32(memory, RAM_BASE + 8), 3);
        assert_eq!(retry_load32(memory, RAM_BASE + 12), 4);
    }

    #[test]
    fn set_associative_lru_replacement_evicts_least_recently_used_line() {
        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(
                0,
                &[11, 12, 21, 22, 31, 32, 41, 42],
                0,
            ))
            .expect("memory should map");

        let mut cache = DirectMappedCache::new(
            memory,
            CacheConfig::new(4, vec![crate::AddressRange::new(0, 0x1000)])
                .with_associativity(2)
                .with_replacement_policy(ReplacementPolicy::LeastRecentlyUsed),
        );

        assert_eq!(cache.load32(0).expect("first line should fill"), 11);
        assert_eq!(cache.load32(8).expect("second way should fill"), 21);
        assert_eq!(cache.load32(0).expect("first line should hit"), 11);
        assert_eq!(cache.load32(16).expect("third line should replace lru"), 31);
        assert_eq!(cache.stats().evictions, 1);

        assert_eq!(
            cache
                .load32(8)
                .expect("evicted line should miss and refill"),
            21
        );
        assert_eq!(cache.stats().read_misses, 4);
        assert_eq!(cache.stats().evictions, 2);
    }

    #[test]
    fn unified_cache_bypasses_uncached_mmio_ranges() {
        const MMIO_BASE: u64 = 0x4000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(MMIO_BASE, &[0x1234_5678], 0))
            .expect("mmio-like device should map");

        let mut cache = DirectMappedCache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(0, 0x1000)]),
        );

        assert_eq!(
            cache
                .load32(MMIO_BASE)
                .expect("uncached load should bypass"),
            0x1234_5678
        );
        assert_eq!(cache.stats().read_hits, 0);
        assert_eq!(cache.stats().read_misses, 0);
        assert_eq!(cache.stats().refills, 0);
        assert_eq!(cache.stats().bypassed_reads, 1);
    }

    #[test]
    fn split_cache_tracks_instruction_and_data_hits_separately() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(
                0,
                &[0x0050_0093, 0x00a0_0113, 0x0000_006f, 0x0000_006f],
                1,
            ))
            .expect("rom should map");
        memory
            .map_device(WordDevice::from_words(RAM_BASE, &[9, 10, 11, 12], 1))
            .expect("ram should map");

        let mut cache = SplitL1Cache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(0, 0x1000)]).with_line_size(16),
            CacheConfig::new(8, vec![crate::AddressRange::new(RAM_BASE, 0x1000)])
                .with_line_size(16)
                .with_write_policy(WritePolicy::WriteBack)
                .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );

        assert_eq!(retry_fetch32(&mut cache, 0), 0x0050_0093);
        assert_eq!(
            cache.fetch32(4).expect("neighbor instruction should hit"),
            0x00a0_0113
        );

        assert_eq!(retry_load32(&mut cache, RAM_BASE), 9);
        assert_eq!(
            cache
                .load32(RAM_BASE + 12)
                .expect("neighbor data word should hit"),
            12
        );

        let stats = cache.stats();
        assert_eq!(stats.instruction.read_misses, 1);
        assert_eq!(stats.instruction.read_hits, 2);
        assert_eq!(stats.instruction.refills, 1);
        assert_eq!(stats.instruction.refill_words, 4);
        assert_eq!(stats.data.read_misses, 1);
        assert_eq!(stats.data.read_hits, 2);
        assert_eq!(stats.data.refills, 1);
        assert_eq!(stats.data.refill_words, 4);
    }

    #[test]
    fn split_cache_invalidates_instruction_line_after_store_to_cached_region() {
        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(0, &[0x0050_0093, 0x00a0_0113], 1))
            .expect("memory should map");

        let config =
            CacheConfig::new(8, vec![crate::AddressRange::new(0, 0x1000)]).with_line_size(8);
        let mut cache = SplitL1Cache::new(memory, config.clone(), config);

        assert_eq!(retry_fetch32(&mut cache, 0), 0x0050_0093);
        retry_store32(&mut cache, 0, 0x0020_81b3);

        let _ = cache
            .fetch32(0)
            .expect_err("instruction fetch should miss after invalidation");
        cache.tick();
        assert_eq!(retry_fetch32(&mut cache, 0), 0x0020_81b3);

        let stats = cache.stats();
        assert_eq!(stats.instruction.read_misses, 2);
        assert_eq!(stats.instruction.invalidations, 1);
        assert!(stats.data.write_accesses >= 2);
    }

    #[test]
    fn split_l1_can_refill_from_unified_l2() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(
                0,
                &[
                    0x0050_0093,
                    0x00a0_0113,
                    0x0020_81b3,
                    0x0000_006f,
                    0x0010_0093,
                    0x0020_0113,
                    0x0031_81b3,
                    0x0000_006f,
                ],
                1,
            ))
            .expect("rom should map");
        memory
            .map_device(WordDevice::from_words(
                RAM_BASE,
                &[9, 10, 11, 12, 13, 14, 15, 16],
                1,
            ))
            .expect("ram should map");

        let l2 = DirectMappedCache::new(
            memory,
            CacheConfig::new(
                8,
                vec![
                    crate::AddressRange::new(0, 0x1000),
                    crate::AddressRange::new(RAM_BASE, 0x1000),
                ],
            )
            .with_line_size(16)
            .with_associativity(2)
            .with_write_policy(WritePolicy::WriteBack)
            .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );
        let mut cache = SplitL1Cache::new(
            l2,
            CacheConfig::new(1, vec![crate::AddressRange::new(0, 0x1000)]).with_line_size(16),
            CacheConfig::new(1, vec![crate::AddressRange::new(RAM_BASE, 0x1000)])
                .with_line_size(16)
                .with_write_policy(WritePolicy::WriteBack)
                .with_store_allocation_policy(StoreAllocationPolicy::WriteAllocate),
        );

        assert_eq!(retry_fetch32(&mut cache, 0), 0x0050_0093);
        assert_eq!(retry_fetch32(&mut cache, 16), 0x0010_0093);
        assert_eq!(retry_fetch32(&mut cache, 0), 0x0050_0093);

        assert_eq!(retry_load32(&mut cache, RAM_BASE), 9);
        assert_eq!(retry_load32(&mut cache, RAM_BASE + 16), 13);
        assert_eq!(retry_load32(&mut cache, RAM_BASE), 9);

        let l1_stats = cache.stats();
        assert_eq!(l1_stats.instruction.read_misses, 3);
        assert_eq!(l1_stats.instruction.refills, 3);
        assert_eq!(l1_stats.instruction.refill_words, 12);
        assert_eq!(l1_stats.data.read_misses, 3);
        assert_eq!(l1_stats.data.refills, 3);
        assert_eq!(l1_stats.data.refill_words, 12);

        let l2_stats = cache.inner().stats();
        assert_eq!(l2_stats.read_misses, 4);
        assert_eq!(l2_stats.refills, 4);
        assert_eq!(l2_stats.refill_words, 16);
        assert!(l2_stats.read_hits >= 20);
    }

    #[test]
    #[should_panic(expected = "cacheable range start")]
    fn rejects_cacheable_range_with_unaligned_start_for_line_size() {
        let memory = MemoryMap::new();
        let config =
            CacheConfig::new(8, vec![crate::AddressRange::new(8, 0x1000)]).with_line_size(16);
        let _cache = DirectMappedCache::new(memory, config);
    }

    #[test]
    #[should_panic(expected = "cacheable range size")]
    fn rejects_cacheable_range_with_partial_line_size() {
        let memory = MemoryMap::new();
        let config = CacheConfig::new(8, vec![crate::AddressRange::new(0, 24)]).with_line_size(16);
        let _cache = DirectMappedCache::new(memory, config);
    }
}
