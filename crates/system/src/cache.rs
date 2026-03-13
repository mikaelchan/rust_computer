//! Direct-mapped cache wrappers that sit in front of a backing bus.

use crate::{Address, AddressRange, Bus, BusError, InterruptSet};

const WORD_BYTES: usize = 4;

/// Static cache geometry and address policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    line_count: usize,
    cached_ranges: Vec<AddressRange>,
}

impl CacheConfig {
    #[must_use]
    pub fn new(line_count: usize, cached_ranges: Vec<AddressRange>) -> Self {
        assert!(line_count > 0, "cache requires at least one line");
        Self {
            line_count,
            cached_ranges,
        }
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.line_count
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
    pub write_accesses: u64,
    pub invalidations: u64,
}

/// Split cache stats keep instruction and data paths visible separately.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SplitCacheStats {
    pub instruction: CacheStats,
    pub data: CacheStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CacheLine {
    valid: bool,
    tag: u64,
    word: u32,
}

#[derive(Debug, Clone)]
struct CacheBank {
    config: CacheConfig,
    lines: Vec<CacheLine>,
    stats: CacheStats,
    pending_fill: Option<Address>,
}

impl CacheBank {
    fn new(config: CacheConfig) -> Self {
        Self {
            lines: vec![CacheLine::default(); config.line_count],
            config,
            stats: CacheStats::default(),
            pending_fill: None,
        }
    }

    fn reset(&mut self) {
        self.lines.fill(CacheLine::default());
        self.stats = CacheStats::default();
        self.pending_fill = None;
    }

    fn stats(&self) -> CacheStats {
        self.stats
    }

    fn config(&self) -> &CacheConfig {
        &self.config
    }

    fn caches_address(&self, addr: Address) -> bool {
        self.config.caches_address(addr)
    }

    fn line_index_and_tag(&self, word_addr: Address) -> (usize, u64) {
        let word_number = word_addr / WORD_BYTES as u64;
        let index = (word_number as usize) % self.lines.len();
        let tag = word_number / self.lines.len() as u64;
        (index, tag)
    }

    fn load_word<B, F>(
        &mut self,
        inner: &mut B,
        word_addr: Address,
        access: F,
    ) -> Result<u32, BusError>
    where
        B: Bus,
        F: FnOnce(&mut B, Address) -> Result<u32, BusError>,
    {
        let (index, tag) = self.line_index_and_tag(word_addr);
        let line = self.lines[index];
        if line.valid && line.tag == tag {
            self.stats.read_hits += 1;
            return Ok(line.word);
        }

        if self.pending_fill != Some(word_addr) {
            self.stats.read_misses += 1;
            self.pending_fill = Some(word_addr);
        }

        match access(inner, word_addr) {
            Ok(word) => {
                self.lines[index] = CacheLine {
                    valid: true,
                    tag,
                    word,
                };
                self.pending_fill = None;
                Ok(word)
            }
            Err(BusError::Busy { remaining_cycles }) => Err(BusError::Busy { remaining_cycles }),
            Err(error) => {
                self.pending_fill = None;
                Err(error)
            }
        }
    }

    fn note_write_access(&mut self) {
        self.stats.write_accesses += 1;
    }

    fn invalidate_word(&mut self, word_addr: Address) {
        if !self.caches_address(word_addr) {
            return;
        }

        let (index, tag) = self.line_index_and_tag(word_addr);
        if self.lines[index].valid && self.lines[index].tag == tag {
            self.lines[index].valid = false;
            self.stats.invalidations += 1;
        }
    }
}

/// A simple direct-mapped, write-through, no-write-allocate unified cache.
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

/// A split direct-mapped L1 with independent instruction and data banks.
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

fn load_cached_byte<B, F>(
    bank: &mut CacheBank,
    inner: &mut B,
    addr: Address,
    access: F,
) -> Result<u8, BusError>
where
    B: Bus,
    F: FnOnce(&mut B, Address) -> Result<u32, BusError>,
{
    let word = bank.load_word(inner, addr & !0b11, access)?;
    Ok(extract_byte(word, addr))
}

fn load_cached_half<B, F>(
    bank: &mut CacheBank,
    inner: &mut B,
    addr: Address,
    access: F,
) -> Result<u16, BusError>
where
    B: Bus,
    F: FnOnce(&mut B, Address) -> Result<u32, BusError>,
{
    let word = bank.load_word(inner, addr & !0b11, access)?;
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
    let word_addr = addr & !0b11;
    data.note_write_access();
    data.invalidate_word(word_addr);
    if let Some(instruction) = instruction {
        instruction.invalidate_word(word_addr);
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

        if !self.bank.caches_address(addr) {
            return self.inner.fetch32(addr);
        }

        self.bank
            .load_word(&mut self.inner, addr, |inner, word_addr| {
                inner.fetch32(word_addr)
            })
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        if !self.bank.caches_address(addr) {
            return self.inner.load8(addr);
        }

        load_cached_byte(&mut self.bank, &mut self.inner, addr, |inner, word_addr| {
            inner.load32(word_addr)
        })
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

        if !self.bank.caches_address(addr) || !self.bank.caches_address(addr + 1) {
            return self.inner.load16(addr);
        }

        load_cached_half(&mut self.bank, &mut self.inner, addr, |inner, word_addr| {
            inner.load32(word_addr)
        })
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.bank.caches_address(addr) {
            return self.inner.load32(addr);
        }

        self.bank
            .load_word(&mut self.inner, addr, |inner, word_addr| {
                inner.load32(word_addr)
            })
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
        self.inner.is_busy()
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

        if !self.instruction.caches_address(addr) {
            return self.inner.fetch32(addr);
        }

        self.instruction
            .load_word(&mut self.inner, addr, |inner, word_addr| {
                inner.fetch32(word_addr)
            })
    }

    fn load8(&mut self, addr: Address) -> Result<u8, BusError> {
        if !self.data.caches_address(addr) {
            return self.inner.load8(addr);
        }

        load_cached_byte(&mut self.data, &mut self.inner, addr, |inner, word_addr| {
            inner.load32(word_addr)
        })
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

        if !self.data.caches_address(addr) || !self.data.caches_address(addr + 1) {
            return self.inner.load16(addr);
        }

        load_cached_half(&mut self.data, &mut self.inner, addr, |inner, word_addr| {
            inner.load32(word_addr)
        })
    }

    fn load32(&mut self, addr: Address) -> Result<u32, BusError> {
        if addr % 4 != 0 {
            return Err(BusError::MisalignedAccess { addr, width: 4 });
        }

        if !self.data.caches_address(addr) {
            return self.inner.load32(addr);
        }

        self.data
            .load_word(&mut self.inner, addr, |inner, word_addr| {
                inner.load32(word_addr)
            })
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
        self.inner.is_busy()
    }

    fn pending_interrupts(&self) -> InterruptSet {
        self.inner.pending_interrupts()
    }
}

#[cfg(test)]
mod tests {
    use crate::{AccessKind, AddressRange, Addressable, Bus, BusError, MemoryMap};

    use super::{CacheConfig, DirectMappedCache, SplitL1Cache};

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
    }

    #[test]
    fn split_cache_tracks_instruction_and_data_hits_separately() {
        const RAM_BASE: u64 = 0x1000_0000;

        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(0, &[0x0050_0093, 0x0000_006f], 1))
            .expect("rom should map");
        memory
            .map_device(WordDevice::from_words(RAM_BASE, &[9], 1))
            .expect("ram should map");

        let mut cache = SplitL1Cache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(0, 0x1000)]),
            CacheConfig::new(8, vec![crate::AddressRange::new(RAM_BASE, 0x1000)]),
        );

        let _ = cache.fetch32(0).expect_err("instruction miss should stall");
        cache.tick();
        assert_eq!(
            cache.fetch32(0).expect("instruction fill should succeed"),
            0x0050_0093
        );
        assert_eq!(
            cache.fetch32(0).expect("instruction hit should succeed"),
            0x0050_0093
        );

        let _ = cache.load32(RAM_BASE).expect_err("data miss should stall");
        cache.tick();
        assert_eq!(cache.load32(RAM_BASE).expect("data fill should succeed"), 9);
        assert_eq!(cache.load32(RAM_BASE).expect("data hit should succeed"), 9);

        let stats = cache.stats();
        assert_eq!(stats.instruction.read_misses, 1);
        assert_eq!(stats.instruction.read_hits, 1);
        assert_eq!(stats.data.read_misses, 1);
        assert_eq!(stats.data.read_hits, 1);
    }

    #[test]
    fn split_cache_invalidates_instruction_line_after_store_to_cached_region() {
        let mut memory = MemoryMap::new();
        memory
            .map_device(WordDevice::from_words(0, &[0x0050_0093], 1))
            .expect("memory should map");

        let mut cache = SplitL1Cache::new(
            memory,
            CacheConfig::new(8, vec![crate::AddressRange::new(0, 0x1000)]),
            CacheConfig::new(8, vec![crate::AddressRange::new(0, 0x1000)]),
        );

        let _ = cache.fetch32(0).expect_err("instruction miss should stall");
        cache.tick();
        assert_eq!(
            cache.fetch32(0).expect("instruction fill should succeed"),
            0x0050_0093
        );

        let _ = cache
            .store32(0, 0x00a0_0113)
            .expect_err("write-through store should stall");
        cache.tick();
        cache
            .store32(0, 0x00a0_0113)
            .expect("retry should complete write-through");

        let _ = cache
            .fetch32(0)
            .expect_err("instruction fetch should miss after invalidation");
        cache.tick();
        assert_eq!(
            cache
                .fetch32(0)
                .expect("refill should observe updated word"),
            0x00a0_0113
        );

        let stats = cache.stats();
        assert_eq!(stats.instruction.read_misses, 2);
        assert_eq!(stats.instruction.invalidations, 1);
        assert_eq!(stats.data.write_accesses, 2);
    }
}
