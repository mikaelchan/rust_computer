# Documentation Conventions

This file defines the documentation maintenance rules for the repository.

## Goals

- Keep code-adjacent knowledge close to the module it describes.
- Keep project-level architecture and experiment guidance under `docs/`.
- Make every important document point to the next useful place to read and the next useful command to run.

## Where Documentation Lives

- Directory-backed code modules should document themselves in the nearest `README.md`.
- Top-level design, roadmap, and experiment guides should live in `docs/`.
- Milestone tracking belongs in `TODO.md`.
- Versioned architectural regression program format belongs with the program images in `crates/cpu/tests/programs/README.md`.

## Required Style Rules

- Use relative Markdown links inside repository documentation.
- Do not use workspace-absolute paths such as `/Users/...` in repository docs.
- Prefer concrete, implementation-aware language over generic descriptions.
- Keep module docs focused on responsibilities, boundaries, data flow, and extension points.
- Keep design docs focused on architecture, workflow, interpretation, and validation entry points.

## Recommended Sections For Module READMEs

Not every module needs every section, but this is the default target shape:

- short title and one-sentence purpose
- responsibilities or major pieces
- key data flow, protocol, or execution model
- extension notes or design boundaries
- related reading
- how to validate

If a module has meaningful timing or ownership rules, document them explicitly instead of implying them.

## Recommended Sections For Design Docs

- what the document is for
- current scope or baseline assumptions
- recommended reading path or related reading
- validation entry points

If a design doc references one concrete code module heavily, link directly to that module README.

## Validation Guidance Rule

Important entry-point documents should tell the reader how to validate the described subsystem.

Typical patterns are:

- `cargo test -p rvsim-cpu`
- `cargo test -p rvsim-system`
- `cargo test -p rvsim-devices`
- `cargo test -p rvsim-isa`
- `cargo run -p rvsim-computer`
- `cargo run -p rvsim-computer --bin memory_microbench`

Use the narrowest command that gives useful feedback for the document's scope.

## When To Update Documentation

Update the nearest relevant documentation when any of the following change:

- a module boundary or responsibility
- the control flow of a major subsystem
- visible timing or precision behavior
- MMIO map, device inventory, or experiment workflow
- validation commands or benchmark entry points

If a code change would surprise a reader of the current README, the README should probably change in the same commit.

## Review Checklist

Before committing documentation updates, check:

1. links are relative
2. commands match current crate or binary names
3. descriptions match the current implementation rather than an older plan
4. the document tells the reader what to read next or run next

## Related Reading

- [../README.md](../README.md)
- [./README.md](./README.md)
- [./architecture.md](./architecture.md)
- [./memory_experiments.md](./memory_experiments.md)
