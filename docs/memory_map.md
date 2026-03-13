# Memory Map

Initial suggested layout:

- `0x0000_0000..0x0000_0fff`: ROM / boot image
- `0x1000_0000..0x1000_0fff`: RAM
- `0x2000_0000..0x2000_0007`: simple UART
- `0x3000_0000..0x3000_000f`: machine timer (`mtime` / `mtimecmp`)

This stays intentionally small so the first computer model can focus on CPU behavior. The timer is the first external interrupt source and feeds the CPU through the unified bus/device surface rather than a hard-coded side channel. Additional storage, DMA, or MMIO devices can be mapped later without changing the CPU trait surface.
