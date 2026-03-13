//! Cache wrappers that sit in front of a backing bus.

use crate::{Address, AddressRange, Bus, BusError, InterruptSet};

const WORD_BYTES: usize = 4;

/// Replacement policy used by set-associative cache banks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReplacementPolicy {
    #[default]
    RoundRobin,
    LeastRecentlyUsed,
}

/// Static cache geometry and address policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    line_count: usize,
    line_size: usize,
    associativity: usize,
    replacement_policy: ReplacementPolicy,
    cached_ranges: Vec<AddressRange>,
}

impl CacheConfig {
    #[must_use]
    pub fn new(line_count: usize, cached_ranges: Vec<AddressRange>) -> Self {
        assert!(line_count > 0, "cache requires at least one line");
        Self {
            line_count,
            line_size: WORD_BYTES,
            associativity: 1,
            replacement_policy: ReplacementPolicy::RoundRobin,
            cached_ranges,
        }
    }

    #[must_use]
    pub fn with_line_size(mut self, line_size: usize) -> Self {
        assert!(
            line_size >= WORD_BYTES && line_size.is_power_of_two() && line_size % WORD_BYTES == 0,
            "cache line size must be a power-of-two multiple of four bytes"
        );
        self.line_size = line_size;
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
    pub fn cached_ranges(&self) -> &[AddressRange] {
        &self.cached_ranges
    }

    #[must_use]
    pub fn caches_address(&self, addr: Address) -> bool {
        self.cached_ranges.iter().any(|range| range.contains(addr))
    }
}

/// Aggregate cache counters useful for validation and basic benchmarking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub read_hits: u64,
    pub read_misses: u64,
    pub refills: u64,
    pub evictions: u64,
    pub write_accesses: u64,
    pub invalidations: u64,
}

/// Split cache stats keep instruction and data paths visible separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SplitCacheStats {
    pub instruction: CacheStats,
    pub data: CacheStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheLine {
    valid: bool,
    tag: u64,
    words: Box<[u32]>,
    last_used: u64,
}

impl CacheLine {
    fn new(words_per_line: usize) -> Self {
        Self {
            valid: false,
            tag: 0,
            words: vec![0; words_per_line].into_boxed_slice(),
            last_used: 0,
        }
    }

    fn reset(&mut self) {
        self.valid = false;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DecodedAddress {
    line_base: Address,
    set_index: usize,
    tag: u64,
    word_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRefill {
    line_base: Address,
    set_index: usize,
    tag: u64,
    way_index: usize,
    next_word_index: usize,
    words: Box<[u32]>,
    kind: RefillKind,
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
            next_word_index: 0,
            words: vec![0; line_words].into_boxed_slice(),
            kind,
        }
    }

    fn matches(&self, decoded: DecodedAddress, kind: RefillKind) -> bool {
        self.line_base == decoded.line_base && self.kind == kind
    }
}

#[derive(Debug, Clone)]
struct CacheBank {
    config: CacheConfig,
    sets: Vec<CacheSet>,
    stats: CacheStats,
    pending_fill: Option<PendingRefill>,
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
            pending_fill: None,
            access_epoch: 0,
        }
    }

    fn reset(&mut self) {
        for set in &mut self.sets {
            set.reset();
        }
        self.stats = CacheStats::default();
        self.pending_fill = None;
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

    fn has_pending_fill(&self) -> bool {
        self.pending_fill.is_some()
    }

    fn caches_address(&self, addr: Address) -> bool {
        self.config.caches_address(addr)
    }

    fn line_base(&self, addr: Address) -> Address {
        addr & !((self.config.line_size as u64) - 1)
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

    fn start_refill(&mut self, decoded: DecodedAddress, kind: RefillKind) {
        let victim = self.select_victim(decoded.set_index);
        if self.sets[decoded.set_index].lines[victim].valid {
            self.stats.evictions += 1;
        }
        self.stats.read_misses += 1;
        self.pending_fill = Some(PendingRefill::new(
            decoded.line_base,
            decoded.set_index,
            decoded.tag,
            victim,
            self.line_words(),
            kind,
        ));
    }

    fn continue_refill<B>(&mut self, inner: &mut B) -> Result<(), BusError>
    where
        B: Bus,
    {
        let Some(mut pending) = self.pending_fill.take() else {
            return Ok(());
        };

        while pending.next_word_index < pending.words.len() {
            let word_addr = pending.line_base + (pending.next_word_index * WORD_BYTES) as u64;
            let word = match pending.kind {
                RefillKind::Fetch => inner.fetch32(word_addr),
                RefillKind::Load => inner.load32(word_addr),
            };

            match word {
                Ok(word) => {
                    pending.words[pending.next_word_index] = word;
                    pending.next_word_index += 1;
                }
                Err(BusError::Busy { remaining_cycles }) => {
                    self.pending_fill = Some(pending);
                    return Err(BusError::Busy { remaining_cycles });
                }
                Err(error) => {
                    return Err(error);
                }
            }
        }

        {
            let line = &mut self.sets[pending.set_index].lines[pending.way_index];
            line.valid = true;
            line.tag = pending.tag;
            line.words.as_mut().copy_from_slice(&pending.words);
        }
        self.stats.refills += 1;
        self.touch_line(pending.set_index, pending.way_index);
        Ok(())
    }

    fn load_word<B>(
        &mut self,
        inner: &mut B,
        addr: Address,
        kind: RefillKind,
    ) -> Result<u32, BusError>
    where
        B: Bus,
    {
        let decoded = self.decode(addr);
        if let Some(way_index) = self.lookup_way(decoded) {
            let word = self.read_word(decoded, way_index);
            self.stats.read_hits += 1;
            self.touch_line(decoded.set_index, way_index);
            return Ok(word);
        }

        if let Some(pending) = &self.pending_fill {
            if !pending.matches(decoded, kind) {
                return Err(BusError::Busy {
                    remaining_cycles: 1,
                });
            }
        } else {
            self.start_refill(decoded, kind);
        }

        self.continue_refill(inner)?;
        let way_index = self
            .lookup_way(decoded)
            .expect("completed refill should install a line");
        Ok(self.read_word(decoded, way_index))
    }

    fn note_write_access(&mut self) {
        self.stats.write_accesses += 1;
    }

    fn invalidate_line(&mut self, addr: Address) {
        if !self.caches_address(addr) {
            return;
        }

        let decoded = self.decode(addr);
        if self
            .pending_fill
            .as_ref()
            .is_some_and(|pending| pending.line_base == decoded.line_base)
        {
            self.pending_fill = None;
        }

        let set = &mut self.sets[decoded.set_index];
        if let Some(line) = set
            .lines
            .iter_mut()
            .find(|line| line.valid && line.tag == decoded.tag)
        {
            line.valid = false;
            self.stats.invalidations += 1;
        }
    }
}

/// A simple write-through unified cache wrapper.
#[derive(Debug)]
pub struct DirectMappedCache<B> {
    inner: B,
    bank: CacheBank,
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

/// A split L1 with independent instruction and data banks.
#[derive(Debug)]
pub struct SplitL1Cache<B> {
    inner: B,
    instruction: CacheBank,
    data: CacheBank,
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
    B: Bus,
{
    let word = bank.load_word(inner, addr & !0b11, RefillKind::Load)?;
    Ok(extract_byte(word, addr))
}

fn load_cached_half<B>(bank: &mut CacheBank, inner: &mut B, addr: Address) -> Result<u16, BusError>
where
    B: Bus,
{
    let word = bank.load_word(inner, addr & !0b11, RefillKind::Load)?;
    Ok(extract_half(word, addr))
}

fn store_through<B, F>(
    instruction: Option<&mut CacheBank>,
    data: &mut CacheBank,
    inner: &mut B,
    addr: Address,
    store: F,
) -> Result<(), BusError>
where
    B: Bus,
    F: FnOnce(&mut B, Address) -> Result<(), BusError>,
{
    data.note_write_access();
    data.invalidate_line(addr);
    if let Some(instruction) = instruction {
        instruction.invalidate_line(addr);
    }
    store(inner, addr)
}

impl<B> Bus for DirectMappedCache<B>
where
    B: Bus,
{
    fn reset(&mut self) {
        self.bank.reset();
        self.inner.reset();
    }

    fn fetch32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.bank.caches_address(addr) || !self.bank.caches_line_containing(addr) {
            return self.inner.fetch32(addr);
        }

        self.bank
            .load_word(&mut self.inner, addr, RefillKind::Fetch)
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        if !self.bank.caches_address(addr) || !self.bank.caches_line_containing(addr) {
            return self.inner.load8(addr);
        }

        load_cached_byte(&mut self.bank, &mut self.inner, addr)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        store_through(
            None,
            &mut self.bank,
            &mut self.inner,
            addr,
            |inner, address| inner.store8(address, value),
        )
    }

    fn load16(&mut self, addr: Address) -> Result<u16, BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        if !self.bank.caches_address(addr)
            || !self.bank.caches_address(addr + 1)
            || !self.bank.caches_line_containing(addr)
        {
            return self.inner.load16(addr);
        }

        load_cached_half(&mut self.bank, &mut self.inner, addr)
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.bank.caches_address(addr) || !self.bank.caches_line_containing(addr) {
            return self.inner.load32(addr);
        }

        self.bank.load_word(&mut self.inner, addr, RefillKind::Load)
    }

    fn store16(&mut self, addr: Address, value: u16) -> Result<(), BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        store_through(
            None,
            &mut self.bank,
            &mut self.inner,
            addr,
            |inner, address| inner.store16(address, value),
        )
    }

    fn store32(&mut self, addr: Address, value: u32) -> Result<(), BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        store_through(
            None,
            &mut self.bank,
            &mut self.inner,
            addr,
            |inner, address| inner.store32(address, value),
        )
    }

    fn tick(&mut self) {
        self.inner.tick();
    }

    fn is_busy(&self) -> bool {
        self.bank.has_pending_fill() || self.inner.is_busy()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.pending_interrupts()
    }
}

impl<B> Bus for SplitL1Cache<B>
where
    B: Bus,
{
    fn reset(&mut self) {
        self.instruction.reset();
        self.data.reset();
        self.inner.reset();
    }

    fn fetch32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.instruction.caches_address(addr) || !self.instruction.caches_line_containing(addr)
        {
            return self.inner.fetch32(addr);
        }

        self.instruction
            .load_word(&mut self.inner, addr, RefillKind::Fetch)
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        if !self.data.caches_address(addr) || !self.data.caches_line_containing(addr) {
            return self.inner.load8(addr);
        }

        load_cached_byte(&mut self.data, &mut self.inner, addr)
    }

    fn store8(&mut self, addr: Address, value: u8) -> Result<(), BusError> {
        store_through(
            Some(&mut self.instruction),
            &mut self.data,
            &mut self.inner,
            addr,
            |inner, address| inner.store8(address, value),
        )
    }

    fn load16(&mut self, addr: Address) -> Result<u16, BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        if !self.data.caches_address(addr)
            || !self.data.caches_address(addr + 1)
            || !self.data.caches_line_containing(addr)
        {
            return self.inner.load16(addr);
        }

        load_cached_half(&mut self.data, &mut self.inner, addr)
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.data.caches_address(addr) || !self.data.caches_line_containing(addr) {
            return self.inner.load32(addr);
        }

        self.data.load_word(&mut self.inner, addr, RefillKind::Load)
    }

    fn store16(&mut self, addr: Address, value: u16) -> Result<(), BusError> {
        if addr % 2 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 2 });
        }

        store_through(
            Some(&mut self.instruction),
            &mut self.data,
            &mut self.inner,
            addr,
            |inner, address| inner.store16(address, value),
        )
    }

    fn store32(&mut self, addr: Address, value: u32) -> Result<(), BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        store_through(
            Some(&mut self.instruction),
            &mut self.data,
            &mut self.inner,
            addr,
            |inner, address| inner.store32(address, value),
        )
    }

    fn tick(&mut self) {
        self.inner.tick();
    }

    fn is_busy(&self) -> bool {
        self.instruction.has_pending_fill() || self.data.has_pending_fill() || self.inner.is_busy()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.pending_interrupts()
    }
}

#[cfg(test)]
mod tests {
    use crate::{AccessKind, AddressRange, Addressable, Bus, BusError, MemoryMap};

    use super::{CacheConfig, DirectMappedCache, ReplacementPolicy, SplitL1Cache};

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
        assert_eq!(cache.stats().read_hits, 1);
        assert_eq!(cache.stats().refills, 1);
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

        assert_eq!(
            cache.fetch32(12).expect("line neighbor should hit"),
            0x0000_006f
        );
        assert_eq!(cache.stats().read_hits, 1);
        assert_eq!(cache.stats().read_misses, 1);
    }

    #[test]
    fn unified_cache_invalidates_cached_word_after_store() {
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

        let _ = cache
            .load32(RAM_BASE)
            .expect_err("load after invalidation should miss again");
        cache.tick();
        assert_eq!(cache.load32(RAM_BASE).expect("refill should succeed"), 9);
        assert!(cache.stats().invalidations >= 1);
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
                .with_line_size(16),
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
        assert_eq!(stats.instruction.read_hits, 1);
        assert_eq!(stats.instruction.refills, 1);
        assert_eq!(stats.data.read_misses, 1);
        assert_eq!(stats.data.read_hits, 1);
        assert_eq!(stats.data.refills, 1);
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
}
