# hazard logic

This module isolates the pipeline hazard rules.

## Responsibilities

- Detect control hazards that require redirects or flushes.
- Detect data hazards that require stalls or forwarding.
- Detect structural hazards when multiple users contend for shared resources.

## Files

- `control.rs`: branch, trap, and redirect-related hazards.
- `data.rs`: register and result dependency logic.
- `structural.rs`: resource-contention checks.

## Why This Separation Matters

- Hazard policy tends to change faster than instruction semantics.
- Keeping the rules separate makes it easier to tune the pipeline without disturbing the reference core.
- Tests can target hazard classes directly instead of inferring them from larger pipeline failures.

## Extension Notes

- If a new feature introduces a stall or flush condition, document it here first.
- Prefer small, explicit hazard predicates over one large opaque controller.

## Related Reading

- [cpu crate](../../../../crates/cpu/src/README.md)
- [cpu core models](../../../../crates/cpu/src/core/README.md)
- [pipeline stages](../../../../crates/cpu/src/pipeline/README.md)
- [CPU plan](../../../../docs/cpu_plan.md)
