## Program Image Format

These files are versioned architectural regression programs for `rvsim-cpu`.

- One 32-bit RV32 machine word per line.
- Hex words use the `0x12345678` form.
- Blank lines are ignored.
- `#` starts a comment until end of line.

The integration test harness parses these files directly so the same program
images can be reused across CPU models and future experiment runners.
