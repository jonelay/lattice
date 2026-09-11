# lattice

A crystal lattice is a repeating structure, and a defect is where the pattern
breaks.

Lattice reads plain-text registers - requirements, specs, tests - and builds a
graph, checking it against a structure you define. It reports every place the
structure breaks: vacancies, missing coverage, broken references. It stores nothing.

## Getting started

From a [release](https://github.com/jonelay/lattice/releases) binary:

```sh
lattice validate --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .
```

Or build from source:

```sh
cargo build --release
target/release/lattice validate --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .
```

Try more commands on this repo's own register:

```sh
lattice trace --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target . --format plain

lattice query reaches REQ-014 --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .

lattice coverage --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .
```

To write your own profile, see [docs/profiles.md](docs/profiles.md). For adapter
configuration, see [docs/adapters.md](docs/adapters.md).
[docs/glossary.md](docs/glossary.md) defines the full vocabulary.

## Commands

| Command | What it does |
|---|---|
| `validate` | report findings |
| `trace` | full per-node report with edges and findings |
| `summary` | status table grouped by a configured attribute, or structural counts when unconfigured |
| `coverage` | per-kind node/edge connectivity counts and percentages |
| `query reaches/reached-by` | transitive reachability (`--check-resolved` flags nodes only reachable through undeclared endpoints) |
| `query path` | shortest path between two nodes |
| `query orphans` | nodes with no incoming or outgoing edges |
| `query counts` | per-kind node and edge tallies |
| `query at` | trace a single node |
| `query diff` | structural diff between two git revisions |
| `fuse` | compose multiple sources into one graph (see [docs/fuse.md](docs/fuse.md)) |
| `resolve` | print the resolved profile document |

All commands take `--format=plain|json|rich` (default: rich on a TTY). Rich is
colorized terminal output; plain is uncolored text; json is machine-readable.

Exit codes: 0 clean, 1 error-severity findings, 2 could not run.
`validate` and `trace` accept `--strict`, which promotes warnings to errors.
`SUPPRESS` entries in the profile silence expected findings without hiding them
from JSON output.

## Adapters

Lattice comes with adapters for several common formats:

- `openspec` (Python) - reads OpenSpec registers
- `entomologist`, `md`, `toml`, `github`, `gitlab` (Rust, in `crates/adapter-*/`)

A suggestion sidecar (`adapters/lattice-suggest`) proposes edges by text
similarity.

## Development

The specs in `openspec/specs/` describe what the code does.
[CHANGELOG.md](CHANGELOG.md) records the released surface. Test with
`cargo test` and, after `cargo build`, `.venv/bin/python -m pytest -q`.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
