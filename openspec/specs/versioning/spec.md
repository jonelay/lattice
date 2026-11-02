# versioning Specification

## Purpose
The versioning policy — changelog format, version-changelog agreement guard, and pre-1.0
semver semantics — so consumers can pin lattice and read what changed between releases.
## Requirements
### Requirement: Keep a Changelog format
The repository SHALL maintain a `CHANGELOG.md` at its root in Keep a Changelog format.
It SHALL contain an `[Unreleased]` section at the top for accumulating changes, followed
by release sections headed `## [X.Y.Z] — YYYY-MM-DD` in reverse chronological order.
Each release section groups entries under Added, Changed, or Removed subsections as
applicable.

#### Scenario: Baseline entry exists
- **WHEN** `CHANGELOG.md` is read
- **THEN** it contains a `## [0.1.0]` section describing the initial capabilities

#### Scenario: Unreleased section present
- **WHEN** `CHANGELOG.md` is read
- **THEN** it contains a `## [Unreleased]` section above all versioned sections

### Requirement: Version-changelog agreement
The shipped version string — the Cargo workspace `version`, since the core moved to
Rust — SHALL have a matching `## [X.Y.Z]` heading in `CHANGELOG.md`. A test SHALL
enforce this so that a version bump without a changelog entry, or a changelog entry
without a version bump, fails the test suite.

The Python adapter package's `pyproject.toml` version SHALL be the workspace version in
PEP 440's normalized spelling (`0.8.0-rc.8` is `0.8.0rc8`), and a test SHALL enforce it,
so a release bump cannot leave the Python package at an older version.

#### Scenario: Version matches changelog
- **WHEN** the workspace declares `version = "0.1.0"` and `CHANGELOG.md` contains `## [0.1.0]`
- **THEN** the version-agreement test passes

#### Scenario: Version with no changelog entry
- **WHEN** the workspace declares `version = "0.2.0"` but `CHANGELOG.md` has no `## [0.2.0]` heading
- **THEN** the version-agreement test fails

#### Scenario: Python package version lags the workspace
- **WHEN** the workspace declares `version = "0.8.0-rc.8"` and `pyproject.toml` declares `version = "0.6.0"`
- **THEN** the version-agreement test fails

#### Scenario: Extra changelog entries are allowed
- **WHEN** `CHANGELOG.md` contains `## [0.2.0]` and `## [0.3.0]` and the workspace declares `version = "0.2.0"`
- **THEN** the version-agreement test passes (the changelog may document future or older releases)

### Requirement: Wire version axes
Every document lattice reads or writes across a process or repository boundary SHALL
carry one version axis governing its envelope, from this inventory:

| Axis | Document | Form | Accepted or written |
|---|---|---|---|
| `profile_version` | profile | X.Y.Z | major `1` |
| `manifest_version` | fuse manifest | X.Y.Z | major `1` |
| `interface_version` | adapter interface document | MAJOR.MINOR | `1.0`, `1.1`, `1.2`, `1.3`, `1.4` |
| `suggestion_version` | suggestion document from the `latt-suggest` sidecar | MAJOR.MINOR | `1.0`, `1.1` |
| `resolved_schema` | resolved profile handed to adapters | integer string | `"1"` |
| `trace_version` | trace JSON | integer string | `"2"` |
| `fuse_version` | fuse JSON | integer string | `"1"` |

An X.Y.Z axis SHALL be checked as three non-negative integers without leading zeros,
and a major above the supported one refused. A reader SHALL refuse a document whose
axis it does not read rather than guess at its shape. `lattice_version` is provenance,
not an axis. The notes file has no axis: it follows the profile major.

Verified by: `cargo test -p latt --test fuse manifest_order_paths_and_invalid_inputs`,
`cargo test -p latt-core --test contract resolved` and
`cargo test -p latt-core --lib adapters::prose_xref only_a_resolved_profile_document_loads`; the
inventory itself is read off this spec.

#### Scenario: A new wire document names its axis
- **WHEN** a change adds a document lattice reads or writes across a boundary
- **THEN** the same change adds its axis to this inventory

### Requirement: Pre-1.0 semver policy
While the package version is below 1.0.0, minor bumps (0.x.0) MAY include breaking
changes to any public surface. Patch bumps (0.x.y) SHALL NOT change public surfaces.
This policy SHALL be stated in `CHANGELOG.md` so consumers can read it without
consulting project internals.

The public surfaces are the following.
- The supported Rust items `docs/embedding.md` lists. These include the dependency
  types a supported signature names or the crate re-exports: `serde_norway`,
  `serde_json`. The `latt_core::adapters` module is unstable, not a public surface.
- The CLI, the finding codes and their default severities, and the shipped profiles'
  ID patterns, as pinned in `contracts/`.
- The `json` output format.
- The wire version axes (Requirement: Wire version axes).
- Node IDs.

Raising the minimum supported Rust version (`rust-version`) SHALL be a minor-version
event.

A break in a `contracts/` artifact, measured against the copy frozen at the last
release, SHALL be named under Breaking, in `[Unreleased]` or the workspace version's
section, with its contract lint. It SHALL
be refused when the pending release is a patch. A pre-release of a version is still
on its way to that version. Between `0.8.0-rc.8` and `0.8.0-rc.9` the pending release
is `0.8.0`, so the bump is minor, not a patch.

Verification: inspection of `CHANGELOG.md` header text, and
`cargo test -p latt --test contracts` under default and `--no-default-features`.

#### Scenario: Policy text present
- **WHEN** `CHANGELOG.md` is read
- **THEN** it contains a statement that minor bumps may break public surfaces before 1.0

#### Scenario: A contract break is unlisted
- **WHEN** a flag is renamed or a code is removed relative to `contracts/released/`, and no Breaking bullet names it
- **THEN** the contracts test fails, and passes once a bullet names the lint and the item

#### Scenario: A required input is added to a released command
- **WHEN** an option or positional argument marked required appears under a command
  `contracts/released/` already lists, and the release did not have it
- **THEN** it is a `cli_now_required` break: every released invocation of that
  command now fails. The same input under a new command is an addition

#### Scenario: A contract break on a patch line
- **WHEN** the last release is `0.8.0` and the workspace is `0.8.1`
- **THEN** any contract break fails, listed or not

### Requirement: Release-candidate identity agrees across release metadata
Ordinary development toward minor release `X.Y.0` SHALL remain on the last shipped
workspace version until a release candidate is cut; it SHALL NOT create a patch merely
to represent that development. Before tagging a candidate, maintainers SHALL set the
Cargo workspace and lockfile version to the exact SemVer prerelease `X.Y.0-rc.N`, set
`pyproject.toml` to its PEP 440 spelling `X.Y.0rcN`, and promote the candidate's
changelog entries into a matching dated `## [X.Y.0-rc.N]` heading. The immutable Git tag
SHALL use the same `vX.Y.0-rc.N` identity. When `X.Y.0` itself is cut, its candidate
sections SHALL be folded into the one `## [X.Y.0]` section, stated as the net change
since the previous release, so the changelog a release ships carries no candidate
headings; the candidates' entries remain readable at their tags.

#### Scenario: Ordinary minor-release development continues
- **WHEN** public-surface work toward the next minor exists but no release candidate is
  being cut
- **THEN** the workspace remains at the last shipped version and no patch is created
  for that work

#### Scenario: A release candidate is tagged
- **WHEN** maintainers create candidate number N
- **THEN** Cargo metadata, the lockfile, `pyproject.toml` (as `X.Y.0rcN`), changelog heading, and immutable Git tag agree on `X.Y.0-rc.N`

#### Scenario: The release folds its candidates
- **WHEN** the workspace version is `X.Y.0` and `CHANGELOG.md` still holds a `## [X.Y.0-rc.N]` heading
- **THEN** `cargo test -p latt-core --locked --test version` fails naming the heading

### Requirement: Development benchmark dependencies are exact
Dependencies that interpret private benchmark output SHALL use exact requirements when
the documentation and normalizer name one supported version. Criterion SHALL remain an
exactly pinned development-only dependency, and changing it SHALL require reviewing the
normalizer and versioned benchmark record.

#### Scenario: Criterion is upgraded
- **WHEN** maintainers change the supported Criterion version
- **THEN** the exact Cargo requirement, lockfile, documentation, normalizer behavior, and benchmark verification are reviewed together

