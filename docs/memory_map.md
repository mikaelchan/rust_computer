# Memory Map

Initial suggested layout:

- `0x0000_0000..0x0000_0fff`: ROM / boot image
- `0x1000_0000..0x1000_0fff`: RAM
- `0x2000_0000..0x2000_0007`: simple UART
- `0x3000_0000..0x3000_000f`: machine timer (`mtime` / `mtimecmp`)
- `0x4000_0000..0x4000_000f`: interrupt controller (`pending`, `enable`, `set-pending`, `claim/complete`)
- `0x5000_0000..0x5000_0003`: machine software interrupt source (`msip`)

This stays intentionally small so the first computer model can focus on CPU behavior. The timer, machine software interrupt source, and interrupt controller are the first asynchronous event sources and feed the CPU through the unified bus/device surface rather than hard-coded side channels. The controller currently exposes 32 software-visible sources with a simple lowest-source-id priority rule. Additional storage, DMA, or MMIO devices can be mapped later without changing the CPU trait surface.

Timing note: mapped devices can now be wrapped with a fixed-latency adapter. The current bus model allows only one access to make progress at a time; a latency-bearing access injects wait states, and the CPU retries the access on later cycles. A unified cache or split L1 instruction/data cache can wrap the whole bus for selected physical ranges such as ROM and RAM, with line refills and dirty write-backs both walking the backing bus word-by-word and inheriting the target device latency. Cacheable ranges are validated against the configured line size, so cached regions must start on a line boundary and span a whole number of lines. MMIO regions continue to bypass the cache.
