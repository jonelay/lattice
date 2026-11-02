# describe-command Specification

## Purpose
`latt describe` answers, in one call, what a profile declares and what the adapter read
under it, so a person or an agent can orient on a register without reading the profile
file and running several counting commands.

## Requirements

### Requirement: Describe command
`latt describe --profile <p> --target <dir> [--adapter <a>] [--format plain|json|rich]`
SHALL run the adapter as `validate` does and report, without validating:

- the adapter that ran and the profile's `adapter:` block as configured: in `json` the
  object the adapter receives in the resolved profile, keys sorted at every depth; in
  `plain` and `rich` the block as YAML;
- each node kind in name order, with its `id_pattern`, its number of
  declared nodes, and the distinct source files those nodes came from, in path order;
- each edge kind in name order, with its allowed `[source, target]`
  pairs and its number of edges;
- each validation entry by code, in code order;
- the adapter's own findings, as `validate` would report them: the profile's
  severities, suppressions and messages applied.

The output SHALL route through the output dispatcher in all three formats. `describe`
SHALL exit 0 whenever it ran, whatever the adapter's findings, and 2 when it could not
run, under the same rules as the other register commands. It takes `--profile-version`.

Verified by: `cargo test -p latt --locked --test cli describe`, and
`.venv/bin/python -m pytest tests/test_docs_json.py tests/test_docs_commands.py`.

#### Scenario: Orientation over a register
- **WHEN** `latt describe --format json` runs over a register whose profile declares
  `requirement` and `test` and whose adapter read two requirement files and no tests
- **THEN** the output names both kinds with their ID patterns, `requirement` with its
  count and two files, `test` with 0 nodes and no files, and the exit code is 0

#### Scenario: Adapter findings do not change the exit
- **WHEN** the adapter reports an error-severity `PARSE_ERROR`
- **THEN** `describe` reports it and exits 0

#### Scenario: The adapter block is structured in JSON
- **WHEN** the profile's `adapter:` block sets `paths: {files: ['*.md']}` and
  `latt describe --format json` runs
- **THEN** `adapter.config.paths.files` is the array `["*.md"]`, and `plain` and `rich`
  show the block as YAML

#### Scenario: A profile message reaches an adapter finding
- **WHEN** the profile sets a `message` on `PARSE_ERROR` and the adapter reports one
- **THEN** `describe` shows the finding with that message in every format, as
  `validate` does

#### Scenario: A profile that does not load
- **WHEN** the profile does not load
- **THEN** `describe` reports why and exits 2
