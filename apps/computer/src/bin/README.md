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

## Output Philosophy

- Binaries in this directory should mainly adapt typed library results into human-readable output.
- Measurement logic, machine construction, and validation-oriented helpers should stay in the library side of the app package.

That split prevents binaries from turning into untestable one-off scripts.

## Current Pattern

- `memory_microbench.rs` calls into `run_memory_microbenchmarks()`.
- The binary then formats the returned report into compact console lines.

Future binaries should follow the same pattern unless they truly need interactive behavior.

## Related Reading

- [computer app](../../../../apps/computer/src/README.md)
- [repository index](../../../../README.md)
- [memory experiments guide](../../../../docs/memory_experiments.md)
