# app binaries

This directory holds extra executable entry points for the `computer` application package.

## Current Binary

- `memory_microbench.rs`: runs the memory and interrupt microbench suite and prints the collected reports.

## Why This Exists

- Some experiments are easier to invoke as dedicated binaries than as flags on the main application.
- Keeping them in `src/bin/` avoids bloating `main.rs` with unrelated entry-point logic.

## Extension Rule

- Add a new binary here when the workflow is operationally distinct.
- Keep shared setup code in `../lib.rs` or sibling modules so binaries stay thin.

## Related Reading

- [computer app](/Users/michael/Workspace/rust_computer/apps/computer/src/README.md)
- [repository index](/Users/michael/Workspace/rust_computer/README.md)
- [memory experiments guide](/Users/michael/Workspace/rust_computer/docs/memory_experiments.md)
