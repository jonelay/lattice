# lattice

Validate and query typed node/edge registers — requirements, the specs that satisfy
them, the tests that verify them — held as plain text in the repo that owns them. A
Rust core runs validation (dangling references, orphans, coverage, ID format) and
traversal queries (reachability, path, diff between two git revisions), in three
output formats, with three-valued exit codes. Domain vocabulary lives in YAML
**profiles**: node kinds, edge kinds, ID patterns, validations — see
[docs/profiles.md](docs/profiles.md) for how to write one. **Adapters**
are standalone programs that read a register's own format and emit a serialized graph
over a versioned contract. Derived state — coverage, status rollups, orphans — is
computed on demand and never stored.

grep will find a string, but not that REQ-0042 has no test covering it, or that a
cross-reference points at an ID that no longer resolves. Those questions need a typed
graph. Git stays the storage layer and the authority: lattice reads the repo's files —
the working tree, a data branch, or two revisions for a diff — builds the graph in
memory, checks it against the rules the profile declares, answers, and exits. It has no store of its own, and it never writes
back into the repo.

It addresses what the commercial requirements-management suites address — traceability
from requirements through specs to test evidence — with the architecture inverted.
Those are databases that own the data and export to git; lattice is a lens over files
git already owns, so requirements stay as markdown tables, or TOML, or whatever the
repo already uses. That gives up managed workflow states, role-based access,
baselines-as-snapshots, and the GUI a regulated environment pays a license for. In
exchange, the review surface is `git diff` and the authority is the file you edited.

The problem it exists for: every in-house requirements system surveyed drifted at the
same point — hand-maintained derived state. See [PROPOSAL.md](PROPOSAL.md) for
architecture, profiles, sequencing and the three scope tests.

## Usage

```sh
cargo build --release
target/release/lattice validate --profile profiles/requirements-rm.yaml \
    --adapter ./adapters/openspec --target .
```

The core is a Rust binary; an adapter is any program that takes `--profile` and
`--target` and writes a contract document to stdout. The shipped adapters are
stdlib-only Python behind entry-point scripts that resolve the project venv
themselves — nothing to install to run them; `uv pip install -e '.[test]'` is
only for the adapter test suite.

`validate` reports findings, `summary` the configured status rollup, `trace` the full
per-node report with edges and attached findings, `query` the traversals —
`reaches`, `reached-by`, `path`, `orphans`, `counts`, and `diff` between two
revisions, with the adapter run live at each — and `resolve` prints the resolved
profile document an adapter or sidecar reads. All take
`--format=plain|json|rich` (default: rich on a TTY, plain otherwise) and share the
exit-code contract: 0 clean, 1 error-severity findings, 2 lattice could not run at
all. `validate` and `trace` take `--strict`, promoting warnings to errors.

Four adapters ship here: `consumer-adapter` (the pilot consumer, the a consumer repo repo's
requirements register), `openspec` (this repo's own register — lattice audits
itself on every verify run), `entomologist` (a public git-backed issue tracker
whose register lives on an orphan branch, so this adapter reads a branch rather
than a worktree), and `tomlreg` (a generic TOML register, a worked second format
rather than a consumer). A suggestion sidecar, `adapters/lattice-suggest`, ranks
candidate `verifies` edges by text similarity; the core renders its output as
hints, and a human decides what becomes a marker edit.

## Development

Development runs through the OpenSpec `opsx` workflow: `openspec/specs/` is the
normative contract, and in-flight changes live under `openspec/changes/`.
[CHANGELOG.md](CHANGELOG.md) records the released surface and the pre-1.0 versioning
policy. Run the test suites with `cargo test` and, after `cargo build`,
`.venv/bin/python -m pytest -q`.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
