# Memory Map

Initial suggested layout:

- `0x0000_0000..0x0000_0fff`: ROM / boot image
- `0x1000_0000..0x1000_0fff`: RAM
- `0x2000_0000..0x2000_0007`: simple UART
- `0x3000_0000..0x3000_000f`: machine timer (`mtime` / `mtimecmp`)
- `0x4000_0000..0x4000_000f`: interrupt controller (`pending`, `enable`, `set-pending`, `claim/complete`)

This stays intentionally small so the first computer model can focus on CPU behavior. The timer and interrupt controller are the first asynchronous event sources and feed the CPU through the unified bus/device surface rather than hard-coded side channels. The controller currently exposes 32 software-visible sources with a simple lowest-source-id priority rule. Additional storage, DMA, or MMIO devices can be mapped later without changing the CPU trait surface.
