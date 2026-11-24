# Contributing to Lattice Auditing & Tracing Tool

## Reporting bugs and requesting features

Open an issue on [GitHub Issues](https://github.com/jonelay/lattice/issues).
Include enough detail to reproduce the problem: the profile, register
format, adapter invocation, and the full output or traceback.

## Development setup

Requires rustup, Python 3.12+, and the pinned OpenSpec CLI. The repository's
`rust-toolchain.toml` selects Rust 1.99 with rustfmt and Clippy automatically.

```bash
git clone https://github.com/jonelay/lattice.git
cd lattice
cargo build --workspace --locked
cargo install cargo-llvm-cov --version 0.9.1 --locked
cargo install cargo-semver-checks --version 0.51.0 --locked
cargo install cargo-audit --version 0.22.2 --locked
python3 -m venv .venv
.venv/bin/pip install -e ".[test]"
npm install --global @fission-ai/openspec@1.7.0
```

In a checkout, `./adapters/<name>` is a shim. For a Rust adapter it runs
`latt-adapter-<name>` from PATH, else the debug build (`$CARGO_TARGET_DIR/debug/` or
`target/debug/`), so a `cargo build --release` alone leaves it nothing to run: build
without `--release` as well, or install. The built-in names `md`, `toml` and
`prose-xref` need neither. The Python entry points use the checkout's `.venv` when
there is one, else `python3`. [docs/cli.md](docs/cli.md) has commands to try against
this repository's own register.

A build without the default `native-adapters` feature still runs external adapter
programs. In such a build a profile whose `adapter.native` names a built-in adapter
fails with a message naming the disabled feature, and an explicit `--adapter md` runs
an external `md` from PATH.

Native and external adapters alike are handed the resolved profile through a scratch
file the core creates exclusively; on Unix it asks for mode 0600, which the umask may
restrict further. The core removes it after success, errors and unwinding; an abort, a
killed process or a failed removal can leave it behind.

## Where a change goes

- The core (`crates/latt-core`) knows no domain vocabulary: kinds, ID shapes and
  which attrs carry text are profile data. Reading a register format belongs in an
  adapter: a built-in one under `crates/latt-core/src/adapters/`, the entomologist
  adapter in `crates/latt/src/bin/latt-adapter-entomologist/`, or a program under
  `adapters/`.
- `openspec/specs/` is the normative contract. A behaviour change updates its
  capability spec in the same commit.
- A change to an interface updates its user page in the same commit:
  [docs/profiles.md](docs/profiles.md) for the profile schema,
  [docs/adapters.md](docs/adapters.md) for an adapter's configuration or the adapter
  contract, [docs/fuse.md](docs/fuse.md) for fuse manifests and profiles,
  [docs/embedding.md](docs/embedding.md) for the `latt_core::service` API,
  [docs/cli.md](docs/cli.md) for commands, output formats and exit codes, and
  [docs/json.md](docs/json.md) for `--format json` output.
- A user-visible change gets an entry in [CHANGELOG.md](CHANGELOG.md) under
  Unreleased; a breaking one goes under Breaking.

## Running tests

Run the complete repository gate before submitting a change:

```bash
./tools/verify.sh
```

In a GitHub release snapshot, run `./tools/verify.sh --snapshot`. Snapshot history
omits the API compatibility baseline, so this passes `--no-api-compat` to the Rust
tooling gate. Coverage and RustSec checks still run, and the gate reports that API
compatibility was skipped. Use the unflagged command in a development checkout.

The script resolves the repository root from its own path, so it may also be invoked
from a child directory, for example as `../../tools/verify.sh`.

The gate's `cargo test --workspace` includes `latt-trustfall`, which runs every
validator shape with a Trustfall spelling as queries over the same graphs and compares
the `(code, node_id)` sets (`crates/latt-trustfall/README.md`). A disagreement names
the profile, the entry, the seed and the IDs each side alone reported; fix Lattice with a
failing test first, or add a row to `deliberate::RULES` with the reason.

Property tests (`proptest`, dev-only) run inside the same `cargo test` as every other
test, at the default 256 cases; `PROPTEST_CASES=4096 cargo test -p latt-core --locked properties`
runs them deeper. They live in a `*properties` test module beside the example tests
of `latt-core`'s `adapters/md/mod.rs` and `adapters/md/parse.rs`, `adapters/prose_xref/parse.rs`, `query/mod.rs`, `query/traverse.rs`,
`profile/raw.rs`, `algo.rs` and `document/wire.rs`; `cargo test -p latt-core properties` runs them alone.
A failure prints the minimal shrunken input and writes its
seed under the crate's `proptest-regressions/`; that file is replayed first on every later
run. Commit it when it records a defect or a regression — with a failing `#[ignore]`d test
naming the defect, fixed in a separate change — and delete it when the fault was the
generator's (`openspec/specs/development-verification/spec.md`).

Native-document and rejected-profile baselines regenerate only with
`LATTICE_REGEN_BASELINES=1` outside CI. The rejected-profile suite refuses to
record a case that no longer exits 2; review any regenerated diff before committing.

For a fast Rust edit loop, use:

```bash
cargo check --workspace --all-targets --locked
cargo test -p latt-core --locked
```

`tools/verify.sh` is the step list. It ends with Lattice's self-audit: this repository's
own register, which must exit 0 (the warnings and the coverage hints it prints gate
nothing):

```bash
"${CARGO_TARGET_DIR:-target}/debug/latt" validate --profile profiles/openspec.yaml \
    --adapter ./adapters/openspec --target .
```

Coverage is written to
`target/coverage/lcov.info`; generating the report is required, but its percentage is
informational. API compatibility is checked on four surfaces against the `api_baseline` commit
`tools/verify-rust-tooling.sh` pins, and every baseline a change supersedes is
reconciled against the CHANGELOG's Breaking entries, so shallow clones must fetch
repository history; the GitHub release snapshot cannot, so its CI passes
`--no-api-compat`, which skips the comparison and says so. RustSec advisories are checked against `Cargo.lock` with
`cargo audit`, which fetches the advisory database: a vulnerability fails the layer,
unmaintained, unsound and yanked warnings are printed only, and with no network the
layer exits 2 rather than passing.

Run only the coverage, compatibility and advisory layer with:

```bash
./tools/verify-rust-tooling.sh
```

A cut also compares every surface by hand against the previous release-candidate or
release tag. `--baseline-rev` cannot compare across a package rename, so compare
rustdoc JSON instead, one surface at a time (`--all-features`, none, or
`--no-default-features`; `-p latt-adapter` likewise):

```bash
git worktree add --detach /tmp/prev v0.8.1
for tree in /tmp/prev .; do
    (cd "$tree" && RUSTC_BOOTSTRAP=1 \
        RUSTDOCFLAGS='-Z unstable-options --output-format json --document-hidden-items' \
        cargo doc --no-deps -p latt-core --all-features)
done
cargo semver-checks --release-type minor \
    --baseline-rustdoc /tmp/prev/target/doc/latt_core.json \
    --current-rustdoc target/doc/latt_core.json
```

Semver-checks puts the crate name in every path, so across a rename first rename the
baseline's packages to the current names (with `package =` aliases where its
dependents name them), or every item reads as missing. Without
`--document-hidden-items` a newly hidden item fails as `*_missing` rather than
`*_now_doc_hidden`, and the Breaking bullet would name the wrong lint.

GitHub receives one snapshot commit per release and none of the development history.
`tools/release-snapshot.sh <tag>` builds it from the paths `tools/snapshot-files`
ships, on `github/main` as parent, and prints the commit without pushing. Every
tracked file must be classified there, so a new path fails the build by name rather
than shipping or dropping out unseen. Before pushing, run `.github/workflows/ci.yml`'s
steps over `git archive <commit>` in a fresh repository.

The release binaries come from `tools/release-binaries.sh <tag> <out-dir>`. It builds
both architectures against glibc 2.28 with `cargo-zigbuild`; a plain `cargo build`
links against the builder's glibc. It names the versions of `cargo-zigbuild` and zig it
expects and how to install them, remaps source paths, writes `SHA256SUMS`, and ends
with `tools/check-release-binaries.sh`, which refuses a binary needing a newer glibc,
weak versions included, or one that names the builder's home. Then run the `x86_64`
and `aarch64` binaries in a glibc 2.28 image, such as `debian:buster-slim`, and check
that stderr stays empty. The benchmarks build with the host's glibc.

### The recipes in `docs/ci.md`

Each runnable block in [docs/ci.md](docs/ci.md) has an HTML comment on the line before
it, which does not render:

```text
<!-- recipe: <name> posture=<posture> kind=<sh|hook|gitlab|mr> clean=<r> warning=<r> broken=<r> unloadable=<r> -->
```

`<r>` is the expected outcome at each fixture revision: an exit status for `sh` and `mr`,
`commit` or `reject` for `hook`, and `pass`, `warn` or `fail` for `gitlab`. The revisions
are:

- `clean`: edges and no finding above hint; the profile test's expected file is
  recorded here and carried through every revision;
- `warning`: exactly one warning;
- `broken`: exactly one error;
- `unloadable`: a profile Lattice cannot load.

Every loadable revision also holds one suppressed error, which a recipe must not count.

`tests/ci_recipes.py` runs every marked block at every revision from a release install
with no Rust toolchain, and fails when an outcome differs from the marker or from the
posture table in `docs/ci.md`. A `sh` or `mr` block runs twice, under `sh` and under
`bash -eo pipefail` as a GitLab `script` line does, and must give the same outcome both
ways. It also checks the setup gates and the advisory recipe with
`latt` missing from PATH. A `sh` or `yaml` block with no marker fails that check. An
illustration that is deliberately not run carries `<!-- recipe: none -->`. Keep each
command on one physical line, `python3 -c` payloads included. A `gitlab` block must stay
a `script` list of plain commands with `exit_codes` in brackets, so the check can read it.

## Submitting changes

1. Open an issue first for anything beyond a small bug fix.
2. Fork the repo and create a branch from `main`.
3. Make your changes, add or update tests as needed.
4. Make sure all checks above pass.
5. Open a pull request against `main`.

Keep PRs focused - one logical change per PR.

## Benchmarks

Run the deterministic in-process component benchmarks separately:

```bash
./tools/benchmark.sh
```

The command is location-independent.

- **Output:** raw Criterion data beneath `target/criterion/` and the normalized
  `target/benchmarks/lattice-benchmark-v2.json`, whose shape the hermetic gate validates
  against `benchmarks/component/report.schema.json`.
- **Measures:** parsing, ingestion, validation, representative queries, supported service
  execution, output formatting and native adapter documents, at the scales declared in
  `crates/latt-core/benches/fixture.json`: a chain at two sizes, a register carrying one
  of each defect class, and a register whose profile carries `EDGE_WHERE`, `EDGE_COUNT`
  and `SUPPRESS` entries.
- **`adapter` group:** times each native adapter writing its document
  (`write_native_document`: profile load and serialization, not ingestion) at the
  manifest's `adapter_scales` only. The register is generated once into the bench's
  scratch directory, so the files are warm and this is not cold I/O.
- **Pins:** each scale pins its expected results, and the benchmark refuses to run when
  they differ. A pin is edited by hand from the block the benchmark prints.
- **Baselines:** `--save-baseline main` keeps the run under that name and `--baseline main`
  compares against it; the record then carries Criterion's change ratios per measurement.
  Each run first discards the previous run's raw results, keeping saved baselines, so the
  record holds only what this run measured. Without a saved baseline the compare form runs
  plain and says so.

Each operation in `crates/latt-core/benches/operations/mod.rs` carries a `body`
version and its timing policy, and the record repeats both on every measurement.
Measurements of one ID compare as one series only while their body versions agree.
The source between an operation's `// body: <group>/<function>` and `// body-end`
markers in `benches/core.rs` and `benches/adapters/mod.rs` is hashed by
`tests/test_benchmark_tools.py` against `benchmarks/component/bodies.json`; a change
there bumps the operation's `body` and appends the new version and the hash the failing
test prints. A marker may name several operations that share one closure, as the three
adapter operations do. Comment lines and whitespace are not hashed.

Criterion is pinned exactly to 0.8.2 as a development-only dependency with Rayon and
plotting disabled. Updating it requires reviewing the raw-output normalizer and its
versioned record.
Benchmark compilation is a correctness gate; successful scheduled/manual collection is
required, but numeric performance changes are informational. Compare
candidate and baseline revisions only on the same runner and fixture. The repeated warm
component suite does not measure cold filesystem, process startup, external adapter, or complete
CLI latency.

Routine scheduled collection compares the previous release, the one
`contracts/released/VERSION` names, and current main through the complete Linux CLI
scenario. With no refs the collector runs exactly that:

```bash
python3 tools/benchmark-revisions.py v0.8.1 main
```

This builds locked release binaries outside the measured region, alternates every
revision on one runner, and writes `target/benchmarks/lattice-e2e-v1.json`. The record
contains raw wall-time and peak-RSS samples, summaries, fixture hashes, output
semantics, each exported revision's resolved compiler, and every pairwise comparison
in argument order. A sample whose exit status differs from the scenario manifest is
rejected. Before measuring, an untimed preflight runs each revision's
`summary --format json` over the fixture and refuses the revision, with exit 2, when its
node counts, edge counts or finding tallies differ from the manifest's `expected` block;
the measured outputs' finding tallies are held to the same block. Byte differences
between revisions' outputs stay reported, not enforced.

`--scenario <manifest>` measures another scenario in `benchmarks/e2e/`. Its `command`
template is the measured argv (`validate` or `trace`), and its optional `profile` names
the profile beside it. Two trace scenarios measure `trace --format json` over the same
chain with 1 KiB of text per node, to show trace peak RSS:
`trace-large-text-v1.json` (ASCII text) and `trace-sparse-non-ascii-v1.json` (each text
starting with one `é`):

```bash
python3 tools/benchmark-revisions.py --scenario benchmarks/e2e/trace-large-text-v1.json v0.8.1 main
```

A trace output's tallies count the findings under its entries and its unattachable ones,
and its entry and outgoing-edge counts must equal the manifest's node and edge counts.
Each record goes to `target/benchmarks/lattice-e2e-<scenario id>.json`, so the routine
record is left alone. Routine scheduled collection runs both trace scenarios after the
default one, over the same two refs. Release qualification refuses any scenario but the default.

A process the collector starts reports at least the collector's own peak RSS, so a
sample whose peak RSS does not exceed the collector's own fails the collection: it
measured the collector.

Peak RSS of one binary can fall in two modes from run to run, because glibc's mmap
threshold moves with timing. A median of five samples can land on either mode, so each
comparison also reports the minimum's and the maximum's movement. When the median moves and both extremes hold,
the samples changed mode and neither mode moved. The collector does not pin the
threshold, since consumers do not run that way;
`GLIBC_TUNABLES=glibc.malloc.mmap_threshold=131072` pins it to show the low mode.

Release qualification keeps main as a distinct development baseline and adds the
immutable candidate tag in the same invocation:

```bash
python3 tools/benchmark-revisions.py --release-qualification \
  <released-tag> <40-character-main-commit> <candidate-tag>
```

All three revisions must be rerun in that one job; results from another job or host were
taken on a different runner and serve only as provenance. Record the exact main commit
before cutting the candidate rather than passing the moving branch name. The result then
reports previous release → main, previous release → RC, and `main → RC` movement.
Release-qualification mode rejects a baseline other than the previous release, a moving
main name, a non-tag candidate, a candidate that is not a `vX.Y.Z-rc.N` tag on the same
or a later numeric release line, or any ref count other than three. A reopened release
line may qualify against its existing frozen release: for example, `v0.8.1-rc.6`
against `v0.8.1`. This compares the `X.Y.Z` numbers, not SemVer prerelease precedence;
`contracts/released/` stays frozen until a release cut. Set the matching GitHub
`main_ref`/`release_candidate_ref` inputs when dispatching the job.

Routine scheduled records remain 30-day CI artifacts. Reviewed milestone and
release-candidate records are kept in the development repository, which the GitHub
release does not include.

New records use schema 1.2, defined by
`benchmarks/e2e/report.schema.json`, carry the manifest's expectations and each
revision's observed preflight, and embed their CI provider, pipeline/run, job, and
runner identity when the host exposes it. Records before schema 1.2 still validate, but
are not sufficient as release-candidate evidence.

Criterion measures only the checked-out development revision; the cross-version
comparison is the end-to-end collector's.

All workspace crates remain private (`publish = false`). None carries docs.rs or
packaging metadata; a crate README such as `latt-trustfall`'s holds development notes.

## AI-assisted development

Contributors may use AI tools provided they review, understand, and accept
responsibility for the resulting contribution. Disclose material assistance
in the pull-request description or with a commit trailer:

```text
Assisted-by: <tool>
```

## License

By contributing you agree that your contributions are dual licensed under
the [Apache License, Version 2.0](LICENSE-APACHE) and the
[MIT license](LICENSE-MIT), without any additional terms or conditions.
