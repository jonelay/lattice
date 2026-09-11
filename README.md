# lattice

A crystal lattice is a repeating structure; a defect is where the pattern breaks.
Lattice reads plain-text registers — requirements, specs, tests — builds a typed graph
against a declared structure, and reports every deviation — vacancies, missing coverage,
broken references — on demand, storing nothing.

Traceability belongs in the repo, not in a database that exports to it. A YAML
**profile** declares the structure: node kinds, edge kinds, ID patterns, validations.
An **adapter** reads the register's format. The review surface is `git diff`.
See [docs/glossary.md](docs/glossary.md) for the full vocabulary.

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

## Commands

| Command | Does |
|---|---|
| `validate` | report findings |
| `trace` | full per-node report with edges and findings |
| `summary` | configured rollup, or structural counts when unconfigured |
| `coverage` | per-kind node/edge connectivity counts and percentages |
| `query reaches/reached-by` | transitive reachability (`--check-resolved` for tainted-reach) |
| `query path` | shortest path between two nodes |
| `query orphans` | nodes with no incoming or outgoing edges |
| `query counts` | per-kind node and edge tallies |
| `query at` | trace a single node |
| `query diff` | structural diff between two git revisions |
| `fuse` | compose multiple sources into one graph; see [docs/fuse.md](docs/fuse.md) |
| `resolve` | print the resolved profile document |

All commands take `--format=plain|json|rich` (default: rich on TTY). Exit codes: 0 clean,
1 error-severity findings, 2 could not run. `validate` and `trace` take `--strict`.
`SUPPRESS` entries in the profile silence expected findings without hiding them from JSON.

## Adapters

Six ship here: `openspec` (Python), `entomologist`, `md`, `toml`, `github`, `gitlab`
(Rust, `crates/adapter-*/`). A suggestion sidecar (`adapters/lattice-suggest`) proposes
edges by text similarity.

## Development

Development runs through OpenSpec: `openspec/specs/` is the normative contract.
[CHANGELOG.md](CHANGELOG.md) records the released surface. Test with `cargo test` and,
after `cargo build`, `.venv/bin/python -m pytest -q`.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
