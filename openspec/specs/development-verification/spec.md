# development-verification Specification

## Purpose
Define reproducible local and CI gates for Lattice's Rust, Python, documentation,
OpenSpec, self-audit, feature, and package-development contracts.
## Requirements
### Requirement: Reproducible Rust development environment
Lattice SHALL declare Rust 1.99 with rustfmt, Clippy, and LLVM tools in a repository
toolchain file. Every workspace crate SHALL inherit centralized Rust, Clippy, and
rustdoc lint policy. CI Cargo commands that resolve or build dependencies SHALL use
the committed lockfile.

Verified by: `cargo metadata --no-deps --format-version 1 --locked`,
`cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
and `rustup component list --installed`.

#### Scenario: Enter the repository with rustup available
- **WHEN** a contributor runs a Cargo command without an explicit toolchain override
- **THEN** rustup selects Rust 1.99 with the formatting, lint, and coverage instrumentation components declared by the repository

#### Scenario: Dependency resolution drifts
- **WHEN** a CI build would require changing `Cargo.lock`
- **THEN** the locked Cargo command fails instead of testing an unreviewed resolution

### Requirement: Verify documentation and supported feature configurations
The Rust gate SHALL run workspace tests, an explicit workspace doctest gate, rustdoc
with warnings denied — with all features, and for `latt-core` without default
features, so an intra-doc link to a feature-gated item cannot break unseen — and
`latt-core` tests and Clippy without default features. It SHALL also run
`latt` tests without default features, and `latt` Clippy, warnings denied, with `tui`
and without default features, because the workspace run only ever builds the binary
with `tui` beside `native-adapters`. The `latt` package's `native-adapters` feature
SHALL forward to `latt-core`'s.
It SHALL run `latt-core` and `latt` tests with `serde_json/preserve_order` enabled, because a
host's dependency graph may unify that feature in, and only that build can hold an
attr map whose keys are out of order.
The exact `docs/embedding.md` success and E0502 ownership examples SHALL be the service
module rustdoc examples and SHALL run under the explicit doctest gate. Stable rustdoc
checks that a `compile_fail` example fails, not its error code, so the ownership example
SHALL sit beside a compiling twin that differs only in the mutation. Documentation
verification SHALL remain explicit if the ordinary test runner changes.

The YAML examples in `docs/profiles.md`, `docs/adapters.md` and `docs/fuse.md` SHALL load:
a complete profile as written, a fragment laid over a base that declares the kinds it
names, and each md and TOML adapter example through its native adapter. Every YAML or
JSON example in `README.md` and under `docs/` SHALL be claimed by exactly one entry of
a per-file table naming the check applied, including where only the keys the core
reads are checked; an unclaimed example SHALL fail naming the file and line. The
interface-document example in `docs/adapters.md` SHALL ingest with the node, edge and
finding counts its text states. That is the `docs_examples` integration test, which
the workspace test gate runs.

The `latt` commands in `README.md` and `docs/cli.md` that carry no placeholder
SHALL exit 0 or 1 run from the repository root against the debug binary; every
`--flag` of a placeholder command in `README.md` or under `docs/` SHALL exist on the
named subcommand, a `browse` command being checked only by a binary built with `tui`;
the command table in `docs/cli.md` SHALL list every CLI subcommand and nothing else,
with `--format` on exactly the commands it says; and the finding-code table and per-code configuration
keys in `docs/profiles.md` SHALL match the core's defaults.

`docs/json.md` SHALL list, per command, every key its `--format json` output carries,
marking the keys that appear only in some runs and the keys that are profile data.
A pytest gate SHALL run each documented command over the fixtures and fail naming the
command and key when the output carries a key the page does not list, or the page
lists an unmarked key the output does not carry; the stderr findings envelope of
`query`, `summary` and `coverage` SHALL be checked the same way.

`README.md`, `AGENTS.md`, the pages under `docs/` and the Markdown under `skills/` ship in a release install, so outside fenced
blocks they SHALL name no tracked repository path that `tools/install-files` leaves out
of the install, and every relative link SHALL resolve to a file the install holds, its `#fragment`, if
any, naming a heading of that page. A link
to the repository on GitHub SHALL pin the tag of the workspace version and name a tracked
path. Where a page states a fact in place of pointing at a contract file, the gate SHALL
compare the two.

Verified by: `cargo test --workspace --locked`,
`.venv/bin/python -m pytest tests/test_docs_commands.py tests/test_docs_json.py tests/test_docs_paths.py`,
`cargo test --workspace --doc --locked`,
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`,
`RUSTDOCFLAGS="-D warnings" cargo doc -p latt-core --no-default-features --no-deps --locked`,
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --document-private-items --locked`,
`cargo test -p latt-core -p latt --features serde_json/preserve_order --locked`,
`cargo test -p latt-core --no-default-features --locked`,
`cargo clippy -p latt-core --all-targets --no-default-features --locked -- -D warnings`,
`cargo test -p latt --no-default-features --locked`, and
`cargo clippy -p latt --all-targets --no-default-features --features tui --locked -- -D warnings`.

#### Scenario: A public documentation example stops compiling
- **WHEN** the canonical embedding guide is incompatible with the public API or its embedded configuration
- **THEN** the named doctest gate fails on the exact guide source independently of the selected ordinary test runner

#### Scenario: Ownership protection regresses
- **WHEN** a host could mutate a graph while a later service call still borrows its view
- **THEN** the guide's compile-fail example compiles and the doctest gate fails

#### Scenario: The ownership example fails for another reason
- **WHEN** an item the compile-fail example names is renamed or removed
- **THEN** its compiling twin stops compiling and the doctest gate fails

#### Scenario: A reference example drifts from the schema
- **WHEN** a YAML example in `docs/profiles.md` carries a key the loader rejects
- **THEN** `docs_examples` fails naming the file and line

#### Scenario: A reference example is added without a check
- **WHEN** a YAML or JSON fence is added to a documented file and no claim-table entry matches it
- **THEN** `docs_examples` fails naming the file and line instead of loading it through a fallback

#### Scenario: A documented command stops working
- **WHEN** a command in `README.md` or `docs/cli.md` names a node this repository's register does not hold, or a flag the subcommand does not take
- **THEN** the pytest gate fails naming the file and line and the exit code or the unknown flag

#### Scenario: A shipped page points outside the install
- **WHEN** `README.md` or a page under `docs/` names a path such as `openspec/specs/…` or `tests/fixtures/…`, links to a file the install does not hold or to a heading its page lacks, or links to GitHub at a ref other than the workspace version's tag
- **THEN** `test_docs_paths.py` fails naming the page, the line and the path

#### Scenario: A subcommand is added without a command-table row
- **WHEN** a subcommand is added to the CLI, removed from it, or gains or loses `--format`, and the command table in `docs/cli.md` is not updated
- **THEN** the `cli_doc_command_table` unit tests fail naming the command

#### Scenario: A JSON key is added without a reference line
- **WHEN** a command's JSON output gains a key, or loses one `docs/json.md` lists as always present
- **THEN** `test_docs_json.py` fails naming the command and the key

#### Scenario: Native adapters hide a core dependency
- **WHEN** `latt-core` requires an item available only through its default native-adapter feature
- **THEN** the no-default test or Clippy gate fails

#### Scenario: The TUI leans on native adapters
- **WHEN** code behind `tui` names an item available only with `native-adapters`, or an import is used only when both are on
- **THEN** the `tui`-without-defaults Clippy gate fails

### Requirement: One complete local verification command
Lattice SHALL provide and document one repository-owned command that runs the complete
Rust build and verification layers, Python tests, strict canonical OpenSpec validation,
and the repository self-audit. It SHALL build the default-feature binaries after every
feature-restricted build and before the Python adapter tests and the self-audit, which
run them,
build the Criterion benchmark and run each of its routines for one iteration without
collecting measurements, and SHALL stop with a non-zero status at the first failed layer. It SHALL resolve
repository paths from the verifier's own location and SHALL behave equivalently when
invoked from the repository root or one of its child directories. Actual benchmark
collection SHALL use its own location-independent command because timing is an
informational development activity rather than the correctness gate.

Verified by: `./tools/verify.sh`, `cd crates/latt-core && ../../tools/verify.sh`,
`./tools/benchmark.sh`, and `.venv/bin/python -m pytest tests/test_docs_commands.py`.

#### Scenario: Run the contributor gate
- **WHEN** a prepared contributor runs `./tools/verify.sh`
- **THEN** it builds the benchmark, executes each benchmark routine once with its fixture checks, and executes the same correctness contract layers represented in CI without treating a timing movement as failure

#### Scenario: A benchmark fixture check fails in the gate
- **WHEN** a benchmark routine or a fixture's pinned result fails during the gate's single execution
- **THEN** the gate stops with a non-zero status at that layer

#### Scenario: A feature-restricted build overwrote the binary
- **WHEN** a no-default-features test run has rebuilt `target/debug/latt` without its native adapters
- **THEN** the gate rebuilds it with default features before the Python tests, and a test validating the markdown fixture with no `--adapter` fails if it did not

#### Scenario: Run from a repository child
- **WHEN** a prepared contributor invokes either repository command from `crates/latt-core`
- **THEN** every repository-relative input and artifact path resolves from the Lattice root

### Requirement: Workspace packages have explicit release intent
Every current workspace package SHALL have a description and SHALL explicitly disable
publishing until a separate change defines its public package, README, dependency,
documentation, and compatibility policy.

Verified by: `cargo metadata --no-deps --format-version 1 --locked` and inspection of
the returned package descriptions and publish lists.

#### Scenario: Accidentally publish a workspace crate
- **WHEN** a contributor attempts to publish a current crate
- **THEN** Cargo rejects the attempt because that crate's manifest explicitly disables publishing

### Requirement: CI verifies the hermetic repository contract
Both GitHub and GitLab CI SHALL run the locked Rust workspace gates, explicit doctest and
denied-warning rustdoc gates, the supported no-default core gates, a single unmeasured
execution of every benchmark routine with its fixture checks, Python tests, strict
canonical OpenSpec validation, and the repository self-audit. CI SHALL build the
default-feature Rust binaries after its feature-restricted gates, ship those as the
binary artifact, and prepare the project Python environment before running the self-audit
adapter. Both CI `rust-check` jobs SHALL run the single-crossterm guard `tools/verify.sh`
runs, failing when the `latt` package's dependency tree holds other than exactly one
`crossterm` version. The Rust steps `tools/verify.sh` runs before the tooling gate SHALL
appear in both CI `rust-check` jobs in the same order. Scheduled or manually triggered CI SHALL collect informational Criterion results
and retain raw plus normalized artifacts. Both CI implementations SHALL keep raw Criterion
results at a stable path across runs under a shared cache key so that a run on the default
branch saves the `main` baseline and every other run compares against it. Live
private-consumer qualification SHALL remain separately labeled and SHALL NOT be implied by
a green hermetic pipeline.

Verified by: inspection of `.github/workflows/ci.yml`,
`.github/workflows/benchmarks.yml`, and, in the development repository, `.gitlab-ci.yml`;
CI YAML parsing;
`openspec validate --specs --strict`; and the repository self-audit command.

#### Scenario: A repository citation drifts
- **WHEN** the repository's own OpenSpec register produces a gating self-audit finding
- **THEN** both CI implementations run that audit and expose its non-zero status

#### Scenario: A second crossterm enters the TUI tree
- **WHEN** a dependency change leaves `cargo tree -p latt` naming two `crossterm` versions
- **THEN** `tools/verify.sh` and both CI `rust-check` jobs fail naming the count

#### Scenario: Hermetic CI is green
- **WHEN** all correctness jobs pass without access to a private live consumer
- **THEN** the result qualifies repository fixtures, the benchmark routines' single execution with their fixture checks, and self-audit without claiming live-consumer or performance-threshold verification

#### Scenario: Scheduled performance collection runs
- **WHEN** an informational benchmark workflow is scheduled or manually requested
- **THEN** it runs the repository benchmark command and retains raw Criterion data and the normalized Lattice record without failing solely for a numeric movement

#### Scenario: Default-branch run saves the baseline
- **WHEN** the benchmark workflow runs on the default branch
- **THEN** it saves the `main` baseline into the cached Criterion directory, and a later run on another ref restores that cache and compares against it

### Requirement: GitHub receives release snapshots only
`tools/release-snapshot.sh <rev> [<parent>] [<message>]` SHALL build the GitHub release
commit for a revision from the paths that revision's `tools/snapshot-files` ships (`+`),
with the previous snapshot (`github/main` by default) as its only parent, print the
commit, and push nothing; neither the working tree nor the index changes. Every tracked
file SHALL fall under an entry, the longest matching entry deciding; an unclassified
file SHALL exit 2 naming it, and nothing is built. Before a snapshot is pushed,
`.github/workflows/ci.yml`'s steps SHALL be run over its tree in a fresh repository.

Verified by: `.venv/bin/python -m pytest tests/test_release_snapshot.py`.

#### Scenario: A withheld directory gains a file
- **WHEN** a file is added under a path `tools/snapshot-files` withholds, such as `notes`
- **THEN** the snapshot builds without it

#### Scenario: A new path is unclassified
- **WHEN** a revision tracks a file no entry covers, such as a new top-level `PROPOSAL.md`
- **THEN** the build exits 2 naming the file and prints no commit

### Requirement: Supported Rust boundaries reject missing documentation
The `latt-core` service facade, the reused public types it explicitly supports, and
the reusable `latt-adapter` boundary SHALL deny missing documentation. A type that a
supported accessor returns or accepts SHALL count as supported. The same boundaries
SHALL deny a fallible function without an `# Errors` section and a function with a
reachable panic without a `# Panics` section. The policy SHALL
be scoped to those boundaries and SHALL NOT hide unrelated public debt behind a
workspace-wide allowance. Every workspace crate SHALL carry crate-level documentation,
and the workspace SHALL warn on the allow-by-default rustdoc lints
`missing_crate_level_docs`, `private_doc_tests` and `unescaped_backticks` so the
denied-warning rustdoc gate sees them. All workspace packages SHALL remain private until a separate
change defines publication, README, docs.rs, packaging, and compatibility policy.

Verified by: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`,
`cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
and inspection of the scoped `missing_docs` attributes and Cargo package metadata.

#### Scenario: An undocumented supported item is added
- **WHEN** a public field, variant, method, or entry point is added beneath a supported boundary without documentation
- **THEN** the denied-warning rustdoc gate fails

#### Scenario: A type reached through a supported accessor gains a field
- **WHEN** an undocumented public field is added to `Node`, `Edge`, or `Pathway`, which `LatticeGraph` accessors return
- **THEN** the denied-warning rustdoc gate fails

#### Scenario: A supported fallible function omits its errors
- **WHEN** a function beneath a supported boundary returns `Result` and its documentation has no `# Errors` section
- **THEN** the Clippy gate fails

#### Scenario: A crate loses its front page
- **WHEN** a workspace crate root has no crate-level documentation
- **THEN** the denied-warning rustdoc gate fails

#### Scenario: No crate is selected for publication
- **WHEN** Cargo metadata is inspected after documentation policy is applied
- **THEN** every package remains explicitly private and no crate-specific publishing metadata is implied

### Requirement: Rust coverage is reported without an unearned threshold
Lattice SHALL provide a named, location-independent command that uses exactly
`cargo-llvm-cov` 0.9.1 to execute the workspace Rust tests and write an LCOV artifact.
Successful tool execution and artifact generation SHALL gate CI. The measured coverage
percentage SHALL remain informational until historical evidence supports a threshold.

Verified by: `./tools/verify-rust-tooling.sh` and inspection of
`target/coverage/lcov.info` plus both CI definitions.

#### Scenario: Generate a development coverage report
- **WHEN** a prepared contributor or CI runs the Rust tooling gate
- **THEN** the workspace tests execute under source coverage and a non-empty LCOV report is produced

#### Scenario: Coverage percentage changes
- **WHEN** the report's percentage rises or falls while report generation succeeds
- **THEN** this change alone does not fail the 0.8 development gate

### Requirement: Pre-1.0 embedding API compatibility is checked
Lattice SHALL use exactly `cargo-semver-checks` 0.51.0 to compare the supported
surfaces with the pinned baseline commit in `tools/verify-rust-tooling.sh`: `latt-core`
with all features, with default features and with only explicit features, and
`latt-adapter`. Default features are compared on their own because an item can move
between features. The comparison SHALL assume a minor release rather than derive the
release type from the two version strings, so that every breaking change fails until
the baseline is advanced deliberately. The compatibility check SHALL gate the development
repository's CI, SHALL fail
clearly when its Git baseline is unavailable, and SHALL fail when any of its comparisons
ran zero lints. A package the baseline lacks, as at the 0.8.0 crate rename, cannot be
compared: the check SHALL report it as a failing lint, `package_missing_from_baseline`,
naming the crate by its library name (`crate latt_core`), so that it must be listed like
any other break and never reads as a skipped comparison.

A commit that breaks a supported surface SHALL be followed by a commit that advances
the baseline to it, the two pushed together so that CI runs on a tip where the
comparison is green. The breaking commit is never pushed or tagged alone. No advance
waives a lint: when the baseline differs from the baseline at the start of the pushed
range (CI passes that commit in `LATTICE_RANGE_BASE`, else the parent commit) or from the
committed baseline (an uncommitted edit), the check SHALL compare the surfaces against
the superseded baseline and fail unless `tools/check-breaking.sh` finds every failing
lint listed. A lint is listed when a single bullet under a `### Breaking` heading of the
changelog's `[Unreleased]` section, or of the section of the workspace version, names
the lint in backticks and names the failing item in backticks. A Rust item is named by
its bare name or by any path ending in it, `.` and `::` alike; a module or type that
contains it does not name it, and neither does the kind word the log prints before it
(`function`, `struct`) or a word of the file location, absolute or relative. A member
the log names through its container in words, `field kinds of struct Node`, is named
as `Node.kinds`, `Node::kinds` or `kinds`. A contract item, from the contract pins' comparison, is named verbatim. For example, the bullet
``- **`query::materialize_revision` is hidden.** It fails `function_now_doc_hidden`.``
lists `function_now_doc_hidden` for `function latt_core::query::materialize_revision`.
When `refuse_patch_breaks` in `tools/verify-rust-tooling.sh` is `yes`, the
reconciliation SHALL also be given the release in `contracts/released/VERSION`
(`--released`) and SHALL refuse every failing lint, listed or not, when the workspace
version is a patch release after it, as the contract pins' check does. A pre-release
of that release is still on its way to it. The switch is `no` until 0.8.0 is
published, so that a re-cut of 0.8.0 may still carry a listed break.

A cut is two commits. The first advances the baseline to its parent, whose Rust tree
the cut ships; the second changes only version metadata and the changelog and is
tagged. The check is green at both. Before the first, every surface SHALL be compared
by hand against the previous immutable release-candidate or release tag, and every lint
that fails there SHALL be listed under Breaking for the cut.

Verified by: `./tools/verify-rust-tooling.sh`, `.venv/bin/python -m pytest
tests/test_check_breaking.py`, and inspection of both CI definitions.

#### Scenario: A supported API is broken without the required version policy
- **WHEN** a contributor removes or incompatibly changes an item present at the embedding baseline
- **THEN** the API compatibility command fails

#### Scenario: The baseline commit is absent
- **WHEN** the repository checkout cannot resolve the pinned compatibility commit
- **THEN** the tooling gate stops with a direct baseline-availability error instead of silently skipping the comparison

#### Scenario: The release snapshot skips the comparison by name
- **WHEN** the tooling gate runs with `--no-api-compat`, as the GitHub release snapshot's CI does because its history cannot hold the baseline commit
- **THEN** coverage and the advisory check run, the gate prints that the comparison was skipped, and the development repository's CI still runs it

#### Scenario: A baseline advance names a lint that is not listed
- **WHEN** a pushed range advances the baseline past a breaking commit and the changelog lists no Breaking bullet naming one of the lints and its item
- **THEN** the tooling gate fails, naming the configuration, the lint and the item

#### Scenario: A bullet names only the item's container
- **WHEN** the only Breaking bullet naming a lint names a module or type containing its item, such as `query` for `latt_core::query::reach`
- **THEN** the reconciliation fails, naming the item as unlisted

#### Scenario: A member is named in words
- **WHEN** a log names an item as `field kinds of struct Node` at a relative location, and the only Breaking bullet naming its lint names `Node`
- **THEN** the reconciliation fails, naming the item as unlisted, and a bullet naming `Node.kinds` passes

#### Scenario: A listed break on a patch line
- **WHEN** the reconciliation is given `--released 0.8.0` for workspace version `0.8.1` and a failing lint is listed under Breaking
- **THEN** it fails, naming the lint as refused; for `0.9.0`, or for `0.8.0-rc.9` after `0.8.0-rc.8`, it passes

#### Scenario: A contract break is listed by its verbatim item
- **WHEN** a contract lint's item is a CLI path or a profile and kind, such as `latt validate --profile` or `openspec.yaml requirement`, and a Breaking bullet names the lint and that item verbatim
- **THEN** the reconciliation passes, and a bullet naming only part of the item does not

#### Scenario: A failing lint names no item
- **WHEN** a log reports a failing lint whose section names no item
- **THEN** the reconciliation exits 2, naming the configuration and the lint, instead of counting zero lints as listed

#### Scenario: A package the baseline lacks
- **WHEN** a supported package has no entry in the baseline commit's `Cargo.lock`
- **THEN** its comparison fails with `package_missing_from_baseline` naming `crate <library name>`, and a baseline advance past it passes only when a Breaking bullet names that lint and the library name

#### Scenario: Every lint of a baseline advance is listed
- **WHEN** each lint the superseded comparison fails is named, with its item, in one Breaking bullet of `[Unreleased]`
- **THEN** the reconciliation passes

#### Scenario: The comparison ran no lints
- **WHEN** `cargo-semver-checks` reports zero checks for the baseline pair
- **THEN** the tooling gate fails with an error naming the skipped comparison instead of passing

### Requirement: RustSec advisories are checked against the lockfile
The Rust tooling gate SHALL use exactly `cargo-audit` 0.22.2 to check the committed
`Cargo.lock` against the RustSec advisory database, after a locked fetch has proven the
lockfile current, and SHALL fail when the report finds a vulnerability. Unmaintained,
unsound and yanked warnings SHALL be printed and SHALL NOT gate. The check fetches the
advisory database on every run, so it is the one step of the correctness gate that is not
hermetic: it SHALL be labelled as such in both CI definitions, and it can go red with no
change in the repository. When the database cannot be fetched, or the check cannot run
for any other reason, the gate SHALL exit 2 with a message saying so, and SHALL NOT pass
on a cached or missing database. A tool of another version SHALL stop the gate with exit 2
and the install command. Both CI tooling jobs SHALL install that version.

Verified by: `./tools/verify-rust-tooling.sh`, the same script with the network blocked
(`HTTPS_PROXY=http://127.0.0.1:9`), and inspection of both CI definitions.

#### Scenario: A locked dependency has an advisory
- **WHEN** `Cargo.lock` holds a crate version a RustSec vulnerability advisory covers
- **THEN** the tooling gate prints the report and exits 1

#### Scenario: The advisory database cannot be fetched
- **WHEN** the gate runs without network access
- **THEN** it exits 2 naming the failed check, instead of passing on a cached database

#### Scenario: A warning-only advisory
- **WHEN** a locked crate is reported only as unmaintained, unsound or yanked
- **THEN** the report shows it and the gate does not fail on it

#### Scenario: The wrong cargo-audit is installed
- **WHEN** `cargo audit --version` does not print the pinned version
- **THEN** the tooling gate exits 2 with the install command

### Requirement: Component benchmarks produce structured development evidence
Lattice SHALL use Criterion 0.8.2 for deterministic in-process component benchmarks of
document parsing, ingestion, ingestion from text, validation, representative graph queries, supported service
execution, output formatting, and native adapter document construction. The harness SHALL use stable benchmark IDs, fixture
scales declared in one manifest, explicit setup and destruction semantics, and fixed
sampling policy. Each scale SHALL name the deterministic generator that builds its
register. The chain generator SHALL be declared at two scales. A defect generator SHALL
be declared at least once; its register SHALL hold dangling references by target and by
source, a declared node reachable only through an undeclared endpoint, unknown node and
edge kinds, parallel edges, duplicate node IDs, missing required attributes, and
adapter-reported findings. A rules generator SHALL be declared at least once with its
own profile; that profile SHALL carry `EDGE_WHERE` entries over more than one edge
kind, `EDGE_COUNT` entries without `group_by`, and `SUPPRESS` entries both code-wide
and by node list, and the harness SHALL check the number of `EDGE_WHERE`,
`EDGE_COUNT` and suppressed findings against the counts the generator's cadences give. A located
generator SHALL be declared at least once; its nodes SHALL carry a provenance for every attr
on a line other than the node's, and its edges SHALL name their declaring node entry.
Ingestion from text SHALL be timed twice over the same text, each including its graph's
destruction: through `ingest_text`, and through `parse_document` then `ingest_document`; the
harness SHALL refuse to run when the two build different graphs. Scale labels SHALL be unique across
generators and across both scale lists. Each operation SHALL declare its scale set: a graph operation
SHALL run at every graph scale, and an adapter operation SHALL name a native adapter and run at every
adapter scale naming that adapter, and at no other scale; each adapter operation SHALL have at least one.
A scale's node and edge
counts SHALL be the counts of the ingested graph. Benchmarks whose work scales with the fixture SHALL declare
node-count throughput; counting visits every node and edge, so
both counts operations scale. The set of benchmark operations SHALL be declared once and read
by both the harness and the normalizer, so a group added to one cannot be absent from
the other. Each operation SHALL declare there a body version, separate from its ID, and
its timing policy: the setup excluded from timing, and whether destroying its input and
its result falls inside the timed interval. The hermetic gate SHALL hash each
operation's marked harness source and fail when the hash differs from the one a
checked-in list records for the operation's current body version.

The manifest SHALL declare adapter scales apart from the graph scales. Each SHALL name
the native adapter it serves and a deterministic generator, which writes the register
and its resolved profile into a scratch directory once, before any timing. The TOML
adapter SHALL be timed over arrays of tables mapping a list attr and scalar and array
edge keys at the end-to-end scenario's row count; the md adapter over pipe tables with
escaped pipes, multi-target edge cells and a fenced example table; the prose-xref
adapter over `§` references with carried-forward file context and self-references,
beside a `§` in inline code and a reference in fenced code that are not read. Each
adapter operation SHALL time `write_native_document` as the CLI's in-process run calls
it, into a fresh buffer dropped inside the iteration, including the resolved profile's
load and excluding ingestion; it is a warm-filesystem measurement, not cold I/O. Its
throughput SHALL be the ingested document's node count. Each adapter scale SHALL pin
the document's byte length and digest and the node, edge and per-code finding counts
of the graph the document ingests to, computed outside timing, and the harness SHALL
refuse to run when the document names the scratch directory.

Each graph scale SHALL pin its expected results in the manifest: the finding codes in the
order validation returns them, the adapter-reported issues in stored order, the
ingested nodes and edges in insertion order with their attributes and provenance,
each node followed by its attrs' provenance, the
JSON findings output, the reach result from the chain origin with each node's kind,
the same reach with resolved-path checking including which nodes are tainted, the
counts result, and the counts result the service entry point returns for the request
its benchmark sends, where an answer of another variant is a differing value. Digests
SHALL use a documented, stable encoding. The harness SHALL
compute these from the same library entry points the benchmarks time, outside every
timed routine, and SHALL refuse to run when any differs, naming the scale, the field,
and the expected and actual values. When a scale has no pinned results the harness
SHALL refuse and print the computed results in manifest form. The harness SHALL NOT
rewrite a pinned value.

A location-independent repository command SHALL retain raw Criterion results and emit a
non-empty, versioned Lattice JSON record containing environment, fixture, policy, benchmark
identity, absolute estimate, uncertainty, dispersion (standard deviation and median
absolute deviation), the Tukey outlier fences, declared throughput data, and each
measurement's body version and timing policy. Measurement
IDs SHALL be the operation's group and function and the scale label. The command
SHALL accept a baseline name in one of two modes: save mode records the run under that
name; compare mode measures against a previously saved run of that name and the record
carries Criterion's change estimates. When compare mode names a baseline that
does not exist, the command SHALL run without comparison and say so instead of failing.
The record SHALL state which mode ran and the baseline name. The record's shape SHALL be
machine-checked against a versioned schema, and that check SHALL run in the hermetic
gate over a synthetic Criterion tree without measuring anything. The component schema
SHALL accept every earlier record version it has published under the same major
version, so retained records stay valid.

The record SHALL describe only the run that produced it. Before measuring, the command
SHALL discard every earlier run's results and comparisons while keeping saved
baselines, and SHALL refuse a baseline name it would discard. The normalizer SHALL
refuse, with the command's exit 2, raw results whose IDs are not exactly the declared
operations at the declared scales, an ID recorded twice, a scaled measurement whose
element count is not its scale's node count, and a scale label declared twice.

Verified by: `./tools/benchmark.sh`, `cargo bench -p latt-core --bench core --locked
-- --test`, inspection of `target/benchmarks/lattice-benchmark-v2.json`,
`.venv/bin/python -m pytest tests/test_benchmark_tools.py -k component`, and
benchmark-source review.

#### Scenario: Collect a development benchmark record
- **WHEN** a prepared contributor runs the repository benchmark command with no baseline argument
- **THEN** every named component benchmark runs and both raw Criterion results and the normalized Lattice record are non-empty, and the record states that no baseline mode ran

#### Scenario: Save a named baseline
- **WHEN** the command runs in save mode with a baseline name
- **THEN** raw results are retained under that name, the normalized record reads from them, and the record names the baseline and the save mode

#### Scenario: Compare against a saved baseline
- **WHEN** the command runs in compare mode and a baseline of that name exists
- **THEN** every measurement the baseline covers carries Criterion's mean and median change estimates against it, a benchmark the baseline does not yet cover records none, and a numeric movement does not fail the command

#### Scenario: Compare against a missing baseline
- **WHEN** the command runs in compare mode and no baseline of that name exists
- **THEN** the command prints that it is running without comparison, collects a plain run, and the record states that no comparison ran

#### Scenario: Record shape is validated in the hermetic gate
- **WHEN** the normalizer runs over the checked-in synthetic Criterion tree
- **THEN** the emitted record validates against the component report schema, and a record missing a dispersion field, a fence, or a throughput declaration is rejected

#### Scenario: A benchmark does not run although an older result of it is on disk
- **WHEN** the command runs and one declared benchmark produces no result, while a previous run's result for that ID and a saved baseline remain
- **THEN** the command fails naming the missing ID, publishes no record, and the saved baseline is still present

#### Scenario: Raw results contradict the declarations
- **WHEN** the raw results hold an undeclared ID or one ID twice, a scaled measurement's element count differs from its scale's node count, or the manifest repeats a scale label
- **THEN** normalization exits 2 naming the ID, the count or the label, and publishes no record

#### Scenario: A timed body changes without its version
- **WHEN** an operation's marked harness source changes and its body version does not
- **THEN** the hermetic gate fails, naming the operation and the hash its new body needs listed beside a bumped version

#### Scenario: Criterion output is absent or malformed
- **WHEN** normalization cannot read a required estimate, a fence file, or finds no benchmark results
- **THEN** collection fails instead of publishing an empty or misleading record

#### Scenario: A pinned fixture result changes
- **WHEN** a change to parsing, ingestion, validation, querying or output alters any pinned result at any scale
- **THEN** the benchmark refuses to run, naming the scale, the field, and the expected and actual values, and timing does not begin

#### Scenario: A pinned field is added or removed
- **WHEN** a computed result gains a field absent from the pin or loses a field present in the pin, including a field whose value is `null`
- **THEN** the benchmark refuses, showing the missing side as `absent`

#### Scenario: A pin's whole block differs in type
- **WHEN** a scale's `expected` is not an object while its computed results are
- **THEN** the benchmark refuses, naming the field `<root>`

#### Scenario: A scale's pin is null
- **WHEN** a scale's `expected` is `null`
- **THEN** the benchmark treats the scale as unpinned, refuses, and prints its computed results in manifest form beside any other refusal

#### Scenario: The defect fixture exercises error paths
- **WHEN** the benchmark builds the defect generator's register
- **THEN** its pinned findings include `VACANCY`, `UNKNOWN_KIND` for a node and for an edge, a duplicate-ID `PARSE_ERROR`, `ATTR_REQUIRED` and the adapter's own finding; its edge count includes each parallel edge; and every operation completes on it

#### Scenario: The rules fixture exercises configured validation paths
- **WHEN** the benchmark builds the rules generator's register
- **THEN** its findings include `EDGE_WHERE` from entries on two edge kinds and ungrouped `EDGE_COUNT`, a code-wide and a node-list `SUPPRESS` each suppress at least one finding, no `SUPPRESS_UNUSED` is produced, and the harness refuses when any of those counts departs from the cadences

#### Scenario: The located fixture exercises per-attr provenance
- **WHEN** the benchmark builds the located generator's register
- **THEN** both readers ingest it from text into the same graph, its pinned graph digest changes when any attr's line changes, and every operation completes on it

#### Scenario: Adapter operations run at their adapter's scales only
- **WHEN** the normalizer reads a Criterion tree for a manifest with graph scales and adapter scales
- **THEN** it expects each graph operation at every graph scale and each adapter operation at exactly the adapter scales naming its adapter, records each adapter measurement's throughput as the ingested node count, and fails naming the ID when an expected adapter run is absent or naming the operation when no adapter scale serves it

#### Scenario: An adapter scale's document is pinned
- **WHEN** the benchmark builds an adapter scale's register and runs its adapter outside timing
- **THEN** the ingested node and edge counts equal the scale's declared counts, and a change to the document's bytes, its digest or its per-code finding counts refuses before timing, so an adapter that read nothing cannot pass as a faster one

#### Scenario: Reach through a vacancy is pinned
- **WHEN** the defect register declares a node reachable from the origin only through an undeclared endpoint
- **THEN** the pinned reach result includes that node, and the pinned resolved-path reach marks it tainted

#### Scenario: Adding the defect scale keeps existing identities
- **WHEN** a record that includes the defect scale is compared against a baseline saved before it existed
- **THEN** every chain-scale benchmark ID is unchanged and carries its change estimate, and each defect-scale ID records no change

#### Scenario: Retained records survive the schema revision
- **WHEN** a record of an earlier published version of the same major schema is validated against the current component schema
- **THEN** it validates

### Requirement: Benchmark machinery is excluded from release dependencies
Criterion SHALL be a `latt-core` development dependency at version 0.8.2 with default
features disabled and only `cargo_bench_support` enabled. Criterion, Rayon, and plotting
libraries SHALL NOT appear in normal workspace dependency edges or be linked into release
binaries. Production parallel acceleration remains outside the 0.8 contract.

Verified by: `cargo tree -p latt-core --edges normal --locked`,
`cargo tree -p latt-core --edges dev --locked`, manifest inspection, and
`cargo build --workspace --release --locked`.

#### Scenario: Build a production release
- **WHEN** the workspace release binaries are built through normal dependency edges
- **THEN** Criterion, Rayon, and plotting support are absent from the production graph and artifacts

#### Scenario: Compile the benchmark target
- **WHEN** the benchmark is compiled with its development dependencies
- **THEN** Criterion is present with cargo-bench support while its Rayon and plotting features remain disabled

### Requirement: End-to-end performance records compare revisions on one runner
The repository SHALL provide a deterministic end-to-end scenario that crosses process,
profile, filesystem, adapter, graph, validation, and output boundaries. Its collector
SHALL resolve requested Git refs to immutable commits, build their locked release
binaries outside measured regions, and execute all compared revisions on the same host
and fixture. The default comparison SHALL use `v0.7.1` as the previous release and
`main` as the developing candidate. Release qualification SHALL run `v0.7.1`, an exact
development-baseline main commit, and an immutable `v0.8.0-rc.N` tag in the same job
rather than replacing main or combining separate-run records.

The structured record SHALL identify the commits, toolchain, host, scenario and fixture
hashes, measurement policy, command shape, and release build configuration. For every
revision it SHALL retain raw wall-time and peak-RSS samples, absolute summaries, exit
status, output digest and size, and parsed finding counts by code and severity. It SHALL
report every pairwise movement in argument order without enforcing a numeric threshold
during 0.8. Samples SHALL be retained per requested ref even when two refs resolve to
the same commit. Per-revision evidence SHALL identify the compiler resolved in that
revision's exported source tree, and each sample SHALL match the scenario's declared
expected exit status.

The scenario manifest SHALL declare the graph and findings the fixture produces: node
counts per kind, edge counts per kind, and finding tallies per code and severity, whose
sums agree with its declared node and edge counts. Before measuring, the collector SHALL
check every revision against them in an untimed preflight through that revision's
`summary --format json`, so the measured command carries no extra work, and each
measured output's finding tallies SHALL match them too. A revision that violates one
SHALL fail collection with a message naming the revision, the invariant, the expected
value and the observed one. The record SHALL carry the scenario's expectations and each
revision's observed preflight. Cross-revision output identity SHALL remain reported,
not enforced, since output bytes may change legitimately between versions.

New records SHALL declare schema version 1.2 and validate against
`benchmarks/e2e/report.schema.json`. They SHALL embed available CI provider,
pipeline/run, job, and runner identity. Retained schema-1.1 records predate the
preflight and SHALL still validate against the schema. Retained schema-1.0 records that
predate per-revision compiler and expected-exit enforcement SHALL remain historical
provenance and SHALL NOT qualify a release candidate.

Verified by: a short local three-revision alias collection, scheduled/manual CI
configuration, record inspection, malformed/inconsistent sample traps, and preflight
traps over stub summaries with a dropped node and a wrong finding tally.

#### Scenario: Compare previous release with main
- **WHEN** the collector runs with its default refs
- **THEN** release binaries for `v0.7.1` and `main` run sequentially on the same host and produce one comparison record

#### Scenario: Qualify the release candidate
- **WHEN** release qualification supplies the eventual 0.8.0 candidate ref
- **THEN** the unchanged scenario compares `v0.7.1`, an exact main baseline commit, and the immutable candidate tag in one job and reports all three pairwise movements

#### Scenario: Release qualification receives moving or untagged refs
- **WHEN** release-qualification mode receives a main branch name, a candidate that is not a `v0.8.0-rc.N` tag, or a ref count other than three
- **THEN** collection fails before building or measuring revisions

#### Scenario: Two labels resolve to one commit
- **WHEN** two requested refs resolve to the same immutable commit
- **THEN** each requested ref retains its own warmups and samples without sharing or doubling another ref's measurements

#### Scenario: A repeated output is inconsistent
- **WHEN** one sample for a revision changes exit status, output bytes, digest, or parsed finding summary
- **THEN** collection fails without presenting the samples as comparable evidence

#### Scenario: A sample exits unexpectedly
- **WHEN** a benchmark command's exit status differs from the scenario manifest
- **THEN** collection fails rather than treating the output as valid performance evidence

#### Scenario: A revision processes less than the scenario's graph
- **WHEN** a revision's untimed preflight reports node counts, edge counts, or finding tallies that differ from the scenario manifest, even though it exits as expected
- **THEN** collection fails before measuring with exit 2, naming the revision, the invariant, the expected value and the observed one

#### Scenario: A schema-1.2 record is produced in CI
- **WHEN** end-to-end collection runs in GitHub or GitLab CI
- **THEN** the record validates against the checked-in schema and carries the execution identity exposed by that CI provider

#### Scenario: Performance movement is observed during 0.8
- **WHEN** the candidate timing or memory summary differs from the baseline
- **THEN** the record reports absolute samples and relative movement but collection success is not threshold-dependent

### Requirement: Retain performance evidence by purpose
Routine scheduled performance records SHALL remain CI artifacts with the configured
30-day retention. Reviewed milestone and release-candidate records SHALL be committed
under `notes/perf-0.8/` of the development repository (the GitHub release withholds
`notes/`) with filenames identifying refs, date, execution class, and a CI
job or run ID when one exists. Criterion SHALL describe only the checked-out revision;
cross-version qualification SHALL use the end-to-end collector.

#### Scenario: Routine scheduled collection completes
- **WHEN** a scheduled run is not selected as milestone or release evidence
- **THEN** its raw and normalized records remain CI artifacts and need not be committed

#### Scenario: Release-candidate evidence is accepted
- **WHEN** maintainers review and accept a release-candidate comparison
- **THEN** its schema-valid structured record is committed under `notes/perf-0.8/` with the previous release, exact main SHA, candidate tag, runner, job/run ID, and date in its filename

### Requirement: GitLab CI runs the documented recipes against a release-style install
GitLab CI runs in the development repository; the GitHub release withholds
`.gitlab-ci.yml`. GitLab CI SHALL run a `consumer` job in an image that carries no Rust toolchain. The job
SHALL build a release-style install with `tools/make-install.sh` from the default-feature
binaries `rust-check` ships, and run the CI recipe guard and the install entry-point
checks against it with only that install's `bin/`, the image's Python directory and the
system directories on PATH. The job SHALL fail, naming the program, when `cargo`,
`rustc` or `rustup` is on that PATH, so that the toolchain-free claim is checked rather
than assumed, and SHALL log which `python3` the recipes resolve. The GitHub workflow SHALL NOT be required to carry
this job; the GitHub mirror is a release step. A green `consumer` job qualifies the install
layout and the documented recipes over fixture revisions, and SHALL NOT be described as a
real-data or live-consumer gate.

Verified by: inspection of `.gitlab-ci.yml` and the `consumer` job's log;
`.venv/bin/python -m pytest tests/test_ci_recipes.py`

#### Scenario: Consumer job on a pushed commit
- **WHEN** a pipeline runs the code gates
- **THEN** the `consumer` job runs after `rust-check`, builds the install from its
  artifacts, and runs the guard with `latt` resolved from the install's `bin/`

#### Scenario: A toolchain leaks into the image
- **WHEN** `cargo` is on the `consumer` job's PATH
- **THEN** the job fails before running the guard, naming the program found

#### Scenario: Consumer green is labelled
- **WHEN** every job in the pipeline passes
- **THEN** the `.gitlab-ci.yml` header says the consumer job proves the install and
  recipes over fixtures, and not live data

### Requirement: Profile load refusals are pinned by a corpus
The repository SHALL hold a corpus of rejected profiles, one case per load refusal the
profile and fuse-profile loaders make, each with its expected exit code and message.
A test SHALL discover the cases when it runs, not when it compiles, so a case added
without a code change is still checked. It SHALL fail when the corpus is empty, and SHALL
name the case in every failure. The expected messages SHALL be regenerated with
`LATTICE_REGEN_BASELINES=1`, the repository's one regeneration switch, and the diff is the
review.

The corpus SHALL hold at least: each retired finding code, the retired operator `not`,
each lint-level severity word (`warn`, `deny`, `allow`), an unknown placeholder and an
unmatched brace in `message`, and a node placeholder on an entry whose findings carry no
node.

Verified by: `cargo test -p latt --locked --test rejected_profiles`

#### Scenario: A case's message drifts
- **WHEN** a loader's refusal text changes and the corpus is not regenerated
- **THEN** the corpus test fails naming the case and showing both messages

#### Scenario: A new case needs no code change
- **WHEN** a case file is added to the corpus and nothing else changes
- **THEN** the next test run checks it

#### Scenario: An empty corpus fails
- **WHEN** the corpus directory holds no case
- **THEN** the corpus test fails rather than passing over nothing

### Requirement: Validators are cross-checked against an independent engine
The repository SHALL hold a development-only crate that expresses lattice's check shapes as
Trustfall queries over the same graph. For every validation entry it can translate, the
crate SHALL check that lattice's findings and the query's results name the same nodes. It
SHALL run over every shipped profile with its fixture register, and over generated graphs
that include self-loops, cycles, dangling edges, absent attrs and null attrs.

A mismatch SHALL fail the test and name:
- the profile and the entry;
- the node IDs only lattice reported;
- the node IDs only the query returned.

A difference that is deliberate SHALL be listed in the crate with the reason, and the test
SHALL assert that it still differs, so the list cannot go stale. Lattice's `present`, which
reads whether an attr exists, is one such difference: an attr holding null or a value of
another type is present to lattice and null to the typed property. (`ne` on an absent attr
is not a difference: Trustfall's `!=` holds for a null property as lattice's `ne` holds for
an absent attr.)

The crate SHALL use only items `latt-core` makes public. It SHALL NOT be published or
installed, and its dependencies SHALL NOT enter `latt-core`'s dependency graph.

Verified by: `cargo test -p latt-trustfall --locked`, and `./tools/verify.sh`, which runs
it through `cargo test --workspace`

#### Scenario: A validator disagreeing with its translation fails
- **WHEN** lattice's `EDGE_COUNT` reports a node the translated query does not return
- **THEN** the cross-check fails naming the profile, the entry and the node ID

#### Scenario: A deliberate difference is held, not hidden
- **WHEN** a `CONSTRAINT` uses `present` and a node holds the attr as null or as a value of
  another type
- **THEN** the cross-check expects lattice and the query to differ on that node, and the
  listed difference must occur across the generated graphs; it fails if it never does, so
  the row is removed when it stops being true

#### Scenario: A dangling edge is visible to both engines
- **WHEN** a fixture edge names a missing target
- **THEN** lattice's `VACANCY` and the query over `EdgeRecord { resolved: false }` name the
  same source node

#### Scenario: The core's dependency graph is unchanged
- **WHEN** `cargo tree -p latt-core --locked -e normal,build` is printed
- **THEN** it names no `trustfall` crate

### Requirement: Property tests run in the ordinary test gate
A parser or graph walk whose contract is a law over its inputs (a round-trip, a
conservation, a parse/unparse agreement, agreement with a reference implementation, or
"unreadable input is reported, never dropped") SHALL be checked by a property test
stating that law, beside the example tests that pin chosen inputs. Property tests SHALL
use the workspace's dev-only `proptest` dependency, SHALL run as ordinary `#[test]`s
in the crate's `cargo test`, and SHALL use the default case count unless the test
documents why it needs another.

A failing seed proptest persists under the crate's `proptest-regressions/` directory SHALL
be committed when it records a defect or a regression, so every run replays it first; a
seed produced by a generator mistake SHALL be deleted when the generator is fixed. A
property that finds a defect SHALL NOT be weakened to pass: the generator is narrowed to
exclude the defective class, the exclusion is named in the property's doc comment, and
the defect is recorded as a failing `#[ignore]`d `#[test]` whose doc comment names the
reason, fixed in a separate change.

Verified by: `cargo test -p latt-core --locked --lib adapters::md`, `cargo test -p latt-core --locked --lib adapters::prose_xref`
and `cargo test -p latt-core --locked`, each of which runs the crate's properties, and
`./tools/verify.sh`, which runs them through `cargo test --workspace`; a committed
`proptest-regressions/` file is replayed by the same commands.

#### Scenario: A property fails on a generated input
- **WHEN** a property test's generator finds an input that breaks the stated law
- **THEN** the test fails naming the minimal shrunken input, and proptest writes its seed under the crate's `proptest-regressions/` directory

#### Scenario: A regression seed is replayed
- **WHEN** a committed `proptest-regressions/` file names a seed and the crate's tests run
- **THEN** that seed is run before any newly generated case

#### Scenario: A property finds a defect in the implementation
- **WHEN** the failing input is a real defect rather than a generator mistake
- **THEN** the property's generator is narrowed with the exclusion documented, the seed is committed, and a failing `#[ignore]`d test records the defect for a separate change

