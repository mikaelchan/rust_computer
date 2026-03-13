# Future OoO

The report goes well beyond a 5-stage scalar pipeline. That should remain a second implementation track.

Planned expansion:

- Add `ooo/` modules under `rvsim-cpu` rather than mutating the in-order pipeline into a hybrid.
- Keep `rvsim-isa` and `rvsim-system` shared so both cores consume the same decode and bus interfaces.
- Introduce `RegisterAliasTable`, `PhysicalRegisterFile`, `ReorderBuffer`, `ReservationStations`, and `LoadStoreQueue` as dedicated modules once the in-order core is stable.

This separation keeps the architectural reference model intact while allowing aggressive microarchitecture experiments later.
