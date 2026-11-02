# release-install Specification

## Purpose

The release-style install: what it holds, how it is built from a directory of built
binaries, and the guarantee that every shipped entry point runs from it with no checkout,
no Rust toolchain and no virtual environment.
## Requirements
### Requirement: The install is built from a binary directory
`tools/make-install.sh <bin-dir> <dest>` SHALL write `<dest>/latt/` holding `bin/` and
the shipped repository files, and SHALL work from any current directory. `bin/` SHALL
hold one file for each binary target of the `latt` package (`latt` and
`latt-adapter-entomologist`), copied from `<bin-dir>` executable, and the list SHALL
be read from the package's targets, not from crate directory names. A binary in
`<bin-dir>` that the `latt` package does not build SHALL NOT be copied. The shipped repository paths SHALL be the ones `tools/install-files` lists, one
per line, and the agent-docs evaluation's user sandbox SHALL read the same file.

From a git checkout (a `.git` directory or a worktree's `.git` file) the script SHALL take
the tracked files under those paths. From an extracted archive with no `.git` it SHALL
take every file under them except `__pycache__` directories. A listed path or tracked
file that is absent on disk SHALL fail the script naming it, never be dropped. File names
containing spaces SHALL be copied intact.

The script SHALL check every input before writing, and SHALL exit non-zero naming the
file when `<bin-dir>` lacks one of those binaries, or when the `latt` manifest lists
no `[[bin]]` target. It SHALL exit non-zero
when `<dest>/latt` already exists, rather than merge into an earlier install. A run
that fails SHALL leave no `<dest>/latt`.

Verified by: `.venv/bin/python -m pytest tests/test_make_install.py` and, in the
development repository, which holds the eval crate the GitHub release withholds,
`cargo test --manifest-path crates/latt-eval/Cargo.toml --locked user_files`

#### Scenario: Install from the debug build
- **WHEN** `make-install.sh target/debug <dest>` runs after `cargo build --workspace`
- **THEN** `<dest>/latt/bin/` holds every binary target of the `latt` package, and
  `<dest>/latt/` holds exactly the manifest's files beside it

#### Scenario: A binary is missing
- **WHEN** `<bin-dir>` lacks `latt-adapter-entomologist`
- **THEN** the script exits non-zero naming `latt-adapter-entomologist` and no `<dest>/latt` exists

#### Scenario: A stale binary is present
- **WHEN** `<bin-dir>` holds a `latt-adapter-md` left by an earlier build, and the `latt`
  package has no `latt-adapter-md` binary target
- **THEN** it is not copied into `bin/`

#### Scenario: A tracked file is missing on disk
- **WHEN** a tracked file under a manifest path has been deleted from the working tree
- **THEN** the script exits non-zero naming it and no `<dest>/latt` exists

#### Scenario: Install from an extracted archive
- **WHEN** the script runs in a `git archive` extraction with no `.git`
- **THEN** it ships every file under the manifest's paths except `__pycache__`

#### Scenario: The evaluation reads the manifest
- **WHEN** a path is added to `tools/install-files`
- **THEN** the evaluation's user sandbox ships it with no other edit

#### Scenario: The binary list cannot be read
- **WHEN** `crates/latt/Cargo.toml` lists no `[[bin]]` target
- **THEN** the script exits non-zero naming the missing `[[bin]]` target and no `<dest>/latt` exists

### Requirement: Agent material ships in the install
The install SHALL hold `AGENTS.md` at its root, pointing an agent at the skill, the core
docs and the rules for using Lattice, and `skills/`: one directory per agent skill, each
with a `SKILL.md` whose front matter gives a `name` equal to the directory's name, in
lowercase letters, digits and single hyphens, and a `description` of 1 to 1024
characters. The repository's own `CLAUDE.md` describes developing Lattice and SHALL stay
out of the install and the release snapshot.

Verified by: `.venv/bin/python -m pytest tests/test_agent_pack.py`.

#### Scenario: A skill an agent can load
- **WHEN** an agent's skills directory holds a copy of `skills/latt/`
- **THEN** its `SKILL.md` front matter names it `latt` and describes when to use it

### Requirement: Python entry points run without a virtual environment
`adapters/openspec` and `adapters/latt-suggest` SHALL run `../.venv/bin/python`,
relative to the entry point, when that file is executable, and otherwise `python3` from
PATH. When neither exists they SHALL exit 127 with one line on stderr naming both. The
caller's working directory SHALL NOT decide which interpreter runs, nor which `adapters`
package it imports. They SHALL write no bytecode: an install is read, never written.

Verified by: `.venv/bin/python -m pytest tests/test_shims.py -k python`

#### Scenario: Checkout with a venv
- **WHEN** `../.venv/bin/python` exists beside `adapters/`
- **THEN** the entry point runs it, whatever `python3` PATH names

#### Scenario: Install without a venv
- **WHEN** no `.venv` exists beside `adapters/` and `python3` is on PATH
- **THEN** the entry point runs the adapter with that `python3`

#### Scenario: Run from another checkout's directory
- **WHEN** the install's entry point runs with the current directory inside a different
  lattice checkout
- **THEN** it imports the install's `adapters` package, not the checkout's

#### Scenario: No bytecode in the install
- **WHEN** an entry point runs from an install with no `__pycache__` beside its modules
- **THEN** none exists after it exits

#### Scenario: No interpreter
- **WHEN** neither the venv interpreter nor `python3` on PATH exists
- **THEN** the entry point exits 127 with one stderr line naming both places it looked

### Requirement: Every shipped entry point runs from the install
From an install whose `bin/` is the only lattice directory on PATH, run from a directory
outside any checkout, with `CARGO_TARGET_DIR`, `PYTHONPATH` and `PYTHONHOME` unset, and
with no `target/` tree and no `.venv` in the install, every `adapters/<name>` entry point
SHALL reach its program, shown by `--help` exiting 0. The core SHALL resolve a bare
external adapter name to `latt-adapter-<name>` on PATH, shown by one bare
`--adapter entomologist` run over a fixture register with its known node count. The
native shims (`md`, `toml`, `prose-xref`) reach `bin/latt` as `latt adapter <name>`; the
general bare-name retry is covered by `builtin-adapter-dispatch`'s own tests.

Verified by: the consumer job (`development-verification` › GitLab CI runs the documented
recipes against a release-style install) and `.venv/bin/python -m pytest
tests/test_ci_recipes.py -k install`

#### Scenario: Shim from the install
- **WHEN** `<install>/adapters/md` runs with `<install>/bin` on PATH
- **THEN** it runs `latt adapter md` from `<install>/bin`

#### Scenario: The checkout cannot rescue a broken install
- **WHEN** an install's `bin/latt-adapter-entomologist` is removed, and the caller's
  environment sets `CARGO_TARGET_DIR` to a checkout's `target` holding
  `debug/latt-adapter-entomologist`
- **THEN** the probe fails, because it clears `CARGO_TARGET_DIR` before running the shim

#### Scenario: Bare external adapter name
- **WHEN** `latt validate --adapter entomologist` runs from the install
- **THEN** the core runs `latt-adapter-entomologist` from PATH and reports the fixture's nodes

