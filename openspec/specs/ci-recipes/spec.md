# ci-recipes Specification

## Purpose

The recipes `docs/ci.md` gives for running lattice in another repository's hooks and CI,
and the guard that runs each one against fixture revisions so a recipe that stops doing
what it says fails a test.
## Requirements
### Requirement: Recipes cover four postures and state their outcomes
`docs/ci.md` SHALL hold recipes in four postures, each labelled with what it lets through
and what stops it:

- **advisory** — never fails. It prints findings, and prints a distinct line saying lattice
  could not run on any status other than 0 or 1.
- **setup gate** — findings print and pass. Any status other than 0 or 1 stops: lattice's
  exit 2, and the shell's 126/127 for an unexecutable or missing `latt`. It is labelled
  as an opt-in gate on a broken setup.
- **error gate** — error-severity findings stop; warnings print and pass.
- **strict gate** — `--strict`; warnings stop too.

It SHALL hold at least: a pre-commit hook in the setup-gate posture, a GitLab CI job using
`allow_failure: exit_codes: [1]` in the setup-gate posture, an advisory variant, an error
gate, a strict gate, a `--format json` check that needs no tool beyond `python3`, and a
merge-request report using `query diff`. `docs/ci.md` SHALL say that a repository whose
blocking gates must stay hermetic keeps lattice advisory, and that `query diff` never exits
1, so the merge-request recipe reports and stops only when lattice could not run.

Every runnable recipe SHALL be a fenced block preceded by a marker comment naming the
recipe and its expected outcome at each of four fixture revisions: `clean` (nonzero edges,
no finding above hint), `warning` (exactly one warning), `broken` (exactly one error) and
`unloadable` (a profile the core cannot load). The marker convention SHALL be described in
`docs/ci.md` itself.

Verified by: `.venv/bin/python -m pytest tests/test_ci_recipes.py -k "markers or
inventory"`

#### Scenario: A runnable block without a marker
- **WHEN** `docs/ci.md` holds a `sh` or `yaml` fence with no marker comment and no explicit
  not-runnable marker
- **THEN** the guard fails naming the block's line

#### Scenario: A marker without a block
- **WHEN** a marker comment is not followed by a fenced block
- **THEN** the guard fails naming the marker's line

#### Scenario: A required recipe is missing
- **WHEN** `docs/ci.md` lacks a recipe the guard's inventory names
- **THEN** the guard fails naming it

### Requirement: The guard runs every recipe against every revision
The guard SHALL build a git repository from the fixture revisions, one commit each, and
run every marked recipe at each revision with `latt` from PATH. It SHALL compare each
outcome with the recipe's marker and with an outcome matrix the guard holds per posture,
and fail naming the recipe and revision on any difference, so a command and its marker
cannot drift together. Before running recipes it SHALL check that the `clean` revision
yields a nonzero edge count, that each revision yields exactly the unsuppressed finding
codes and severities its name states, and that each loadable revision also holds a
suppressed error, which no recipe may count. A marked recipe the guard cannot run SHALL be a failure, never
a skip.

The guard SHALL isolate git from the caller's configuration: a fixture identity, no
signing, and no global or system hooks or templates. Each recipe runs in its own clone.

- A shell recipe's outcome is its exit status. It SHALL run under `sh` and again under
  `bash -eo pipefail`, as a GitLab `script` line runs, and SHALL give the same outcome both
  ways.
- A pre-commit recipe SHALL be installed as the clone's hook and exercised with a real
  `git commit` of a staged change. Git reports any hook failure as exit 1, so the outcome
  is `commit` (HEAD advanced) or `reject` (HEAD unchanged, and the hook's own stderr line
  saying why present).
- A GitLab CI recipe's outcome SHALL be the job outcome its `script:` lines and
  `allow_failure` setting imply, with the lines run as GitLab's bash executor runs them
  (`bash -eo pipefail`): `pass` for exit 0, `warn` for an exit its `exit_codes` list
  allows, `fail` otherwise. GitLab itself is not run.
- A merge-request recipe SHALL run with the `clean` revision as its diff base, and its
  output SHALL name the change that revision introduced.

The guard SHALL also run the setup-gate and advisory recipes with `latt` missing from
PATH and with a non-executable `latt`, and check that the setup gate stops and the
advisory recipe passes with its could-not-run line.

Verified by: `.venv/bin/python -m pytest tests/test_ci_recipes.py`

#### Scenario: Setup gate tolerates findings but not a broken setup
- **WHEN** the GitLab CI recipe runs at `broken` and at `unloadable`
- **THEN** its outcome is `warn` at `broken` and `fail` at `unloadable`

#### Scenario: Hook rejects only a broken setup
- **WHEN** the pre-commit recipe is installed and a commit is made at `broken`, at
  `unloadable`, and with no `latt` on PATH
- **THEN** the commit lands at `broken`, and is rejected in the other two cases with the
  hook's line saying lattice could not run

#### Scenario: Error gate and strict gate differ at warning
- **WHEN** the error-gate and strict-gate recipes run at `warning`
- **THEN** the error gate exits 0 and the strict gate exits 1

#### Scenario: Advisory never fails
- **WHEN** the advisory recipe runs at `unloadable`, under `sh` or under `bash -eo pipefail`
- **THEN** it exits 0 and prints its could-not-run line

#### Scenario: A suppressed error does not fail a JSON recipe
- **WHEN** the `--format json` recipe runs at `clean`, whose register holds a suppressed error
- **THEN** it exits 0, as `latt validate` does

#### Scenario: JSON check keeps exit 2
- **WHEN** the `--format json` recipe runs at `unloadable`
- **THEN** it exits 2, not the parser's status on empty input

#### Scenario: Merge-request report compares the right revisions
- **WHEN** the merge-request recipe runs at `broken` against base `clean`
- **THEN** its output names the edge `broken` added, and it exits 0

#### Scenario: A recipe drifts
- **WHEN** a recipe's command is edited so its outcome no longer matches its marker or the
  posture's matrix
- **THEN** the guard fails naming the recipe and the revision

#### Scenario: Zero edges at clean
- **WHEN** the `clean` revision's register yields no edges
- **THEN** the guard fails before running any recipe

### Requirement: A consumer can test its own profile
`docs/ci.md` SHALL hold a recipe for a consumer testing its own profile: it runs
`latt validate --format json` over a small fixture register kept beside the profile,
projects the findings to their codes and node IDs with `python3` alone, and compares the
projection with an expected file kept beside the fixture. It SHALL fail when the
projection differs and SHALL say that message text is left out of the projection so a
wording change does not fail it. The recipe SHALL carry a marker like every runnable
recipe and SHALL be in the guard's inventory.

`docs/ci.md` SHALL also say how lattice's exit codes sit beside cargo-semver-checks'
(lattice 0 clean, 1 error-severity findings, 2 could not run; cargo-semver-checks 0 pass,
100 lint failure, 101 could not complete), and that a `warning` stops a `--strict` lattice
run where a cargo-semver-checks `warn` never stops its run. That text SHALL hold no
runnable block, so the guard needs no cargo-semver-checks install.

Verified by: `.venv/bin/python -m pytest tests/test_ci_recipes.py`

#### Scenario: The projection matches at the clean revision
- **WHEN** the profile-test recipe runs at the `clean` fixture revision with the expected
  file recorded there
- **THEN** it exits 0

#### Scenario: A new finding fails the recipe
- **WHEN** the recipe runs at the `warning` revision against the `clean` expected file
- **THEN** it exits non-zero and prints the differing code and node ID

#### Scenario: The inventory names the recipe
- **WHEN** `docs/ci.md` lacks the profile-test recipe
- **THEN** the guard fails naming it

