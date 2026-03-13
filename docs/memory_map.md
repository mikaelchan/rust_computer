# Memory Map

Initial suggested layout:

- `0x0000_0000..0x0000_0fff`: ROM / boot image
- `0x1000_0000..0x1000_0fff`: RAM
- `0x2000_0000..0x2000_0007`: simple UART

This stays intentionally small so the first simulator can focus on CPU behavior. Additional timers, storage, DMA, or MMIO devices can be mapped later without changing the CPU trait surface.
