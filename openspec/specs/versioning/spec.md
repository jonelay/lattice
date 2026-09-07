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

#### Scenario: Version matches changelog
- **WHEN** the workspace declares `version = "0.1.0"` and `CHANGELOG.md` contains `## [0.1.0]`
- **THEN** the version-agreement test passes

#### Scenario: Version with no changelog entry
- **WHEN** the workspace declares `version = "0.2.0"` but `CHANGELOG.md` has no `## [0.2.0]` heading
- **THEN** the version-agreement test fails

#### Scenario: Extra changelog entries are allowed
- **WHEN** `CHANGELOG.md` contains `## [0.2.0]` and `## [0.3.0]` and the workspace declares `version = "0.2.0"`
- **THEN** the version-agreement test passes (the changelog may document future or older releases)

### Requirement: Pre-1.0 semver policy
While the package version is below 1.0.0, minor bumps (0.x.0) MAY include breaking
changes to any public surface. Patch bumps (0.x.y) SHALL NOT change public surfaces.
This policy SHALL be stated in `CHANGELOG.md` so consumers can read it without
consulting project internals.
Verification: inspection of `CHANGELOG.md` header text.

#### Scenario: Policy text present
- **WHEN** `CHANGELOG.md` is read
- **THEN** it contains a statement that minor bumps may break public surfaces before 1.0
