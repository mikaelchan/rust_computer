# Memory Map

Initial suggested layout:

- `0x0000_0000..0x0000_0fff`: ROM / boot image
- `0x1000_0000..0x1000_0fff`: RAM / DRAM-backed main memory
- `0x2000_0000..0x2000_0007`: simple UART
- `0x3000_0000..0x3000_000f`: machine timer (`mtime` / `mtimecmp`)
- `0x4000_0000..0x4000_000f`: interrupt controller (`pending`, `enable`, `set-pending`, `claim/complete`)
- `0x5000_0000..0x5000_0003`: machine software interrupt source (`msip`)
- `0x6000_0000..0x6000_0017`: DMA controller registers
- `0x7000_0000..0x7000_0023`: block-device registers and one-block staging window in the current example configuration

This stays intentionally small so the first computer model can focus on CPU behavior. The timer, machine software interrupt source, and interrupt controller are the first asynchronous event sources and feed the CPU through the unified bus/device surface rather than hard-coded side channels. The controller currently exposes 32 software-visible sources with a simple lowest-source-id priority rule. Additional storage, DMA, or MMIO devices can be mapped later without changing the CPU trait surface.

Timing note: mapped devices can now be wrapped with a fixed-latency adapter, and main memory can also be modeled as a simple DRAM device with row-miss, row-hit, and sequential-burst latencies. The current bus model allows only one access to make progress at a time; a latency-bearing access injects wait states, and the CPU retries the access on later cycles. A unified cache or split L1 instruction/data cache can wrap the whole bus for selected physical ranges such as ROM and RAM, with line refills and dirty write-backs both walking the backing bus word-by-word and inheriting the target device latency. Cacheable ranges are validated against the configured line size, so cached regions must start on a line boundary and span a whole number of lines. MMIO regions continue to bypass the cache.

## Notes On Scope

- The list above mixes the original minimalist layout with ranges that are now present in the integrated example machine.
- Not every experiment needs every device. Smaller benchmark setups often map only the subset they need.
- Supervisor software interrupts are currently modeled in the codebase even when a particular top-level machine configuration does not expose a dedicated MMIO range for them.

## Related Reading

- [architecture.md](./architecture.md)
- [memory_experiments.md](./memory_experiments.md)
- [../crates/system/src/README.md](../crates/system/src/README.md)
- [../crates/devices/src/README.md](../crates/devices/src/README.md)
- [../apps/computer/src/README.md](../apps/computer/src/README.md)

## How To Validate

- `cargo run -p rvsim-computer`
  Verifies the current integrated example still matches the documented map shape closely enough to boot and exercise devices.
- `cargo test -p rvsim-system`
  Verifies address decoding, cache bypass behavior, and mapped-device routing.
- `cargo test -p rvsim-devices`
  Verifies the devices that occupy the mapped regions.
