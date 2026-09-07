# adapter-tomlreg Specification

## Purpose
A minimal, domain-free TOML register adapter kept in lattice as the standing evidence
that the core and the profile schema are format-agnostic — that nothing in them assumes
a markdown register. It reads a fixture, not a live repository, and its only consumer is
lattice's own regression harness.
## Requirements
### Requirement: Build register nodes from the register table
The adapter SHALL read each `*.toml` file in the profile-configured register directory
(`adapter.paths.register_dir`) and emit one `register` node per file from its
`[register]` table. The node ID SHALL be the file stem, because that is what a sibling
item's `register` reference names, and so is the join key that must resolve.

A file whose `[register]` table is absent SHALL emit a `PARSE_ERROR` naming the file and
be skipped, rather than producing a node with no identity.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_tomlreg.py -k register_node`

#### Scenario: Register node per file
- **WHEN** the adapter runs against a directory holding `alpha.toml` and `beta.toml`,
  each with a `[register]` table
- **THEN** the graph contains register nodes `alpha` and `beta`

#### Scenario: Register table absent
- **WHEN** a `*.toml` file in the directory has no `[register]` table
- **THEN** the adapter emits a `PARSE_ERROR` naming that file, emits no node for it,
  and continues reading the remaining files

### Requirement: Build item nodes with register-qualified IDs
The adapter SHALL emit one `item` node per `[[item]]` row. The node ID SHALL be
`<file-stem>/<row id>`, because row IDs are local slugs and collide across files; an
unqualified ID would let one register's row silently displace another's.

A row with no `id` SHALL emit a `PARSE_ERROR` carrying the file and the row's index, and
SHALL NOT produce a node.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_tomlreg.py -k item_node`

#### Scenario: Item ID is register-qualified
- **WHEN** `alpha.toml` holds a row with `id = "one"`
- **THEN** the graph contains an item node `alpha/one`

#### Scenario: Same row ID in two registers
- **WHEN** `alpha.toml` and `beta.toml` each hold a row with `id = "one"`
- **THEN** the graph contains both `alpha/one` and `beta/one`, and neither is dropped

#### Scenario: Row without an id
- **WHEN** a `[[item]]` row carries no `id` key
- **THEN** the adapter emits a `PARSE_ERROR` naming the file and the row index, and
  emits no node for that row

### Requirement: Resolve cross-register references as edges
The adapter SHALL read a row's `register` key as a `belongs_to` edge from the item to
the named register node, and each element of its `refs` list as a `references` edge from
the item to another item node.

The adapter SHALL NOT check that an endpoint exists. An unresolvable reference is a
`DANGLING_REF` finding at validation, which is the core's job; an adapter that dropped
the edge would hide it.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_tomlreg.py -k edge`

#### Scenario: Reference becomes an edge
- **WHEN** an item in `alpha.toml` declares `refs = ["beta/two"]`
- **THEN** the graph contains a `references` edge from `alpha/one` to `beta/two`

#### Scenario: Reference to an absent item
- **WHEN** an item declares a `refs` element naming an item no file declares
- **THEN** the adapter still emits the edge, and `lattice validate` reports
  `DANGLING_REF` for it

### Requirement: Read the ordering axis from the register
The adapter SHALL read `stage_order` and `current_stage` from the `[register]` table of
the profile-configured axis file (`adapter.paths.axis_file`) and attach them to the graph
as the `stage` axis. A profile declaring no axis file has no axis, which is not a finding.

The register owns both values. The adapter SHALL NOT default, infer, or carry its own
copy of either — a target that stops declaring them has no axis, and that is the honest
answer. The axis is named `stage` rather than `phase` deliberately: it is a second,
differently-named axis, which is what demonstrates the name is profile data and not
something the core knows.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_tomlreg.py -k axis`

#### Scenario: Axis attached
- **WHEN** the axis file declares `stage_order = ["s1", "s2", "s3"]` and
  `current_stage = "s2"`
- **THEN** the graph carries a `stage` axis with that order and `s2` as current

#### Scenario: Register declares no axis
- **WHEN** the axis file's `[register]` table declares neither key
- **THEN** the graph carries no `stage` axis and the adapter emits no node or edge for it

#### Scenario: Current stage not in the order
- **WHEN** `current_stage` names a value absent from `stage_order`
- **THEN** the adapter emits an `AXIS_INVALID` and attaches no axis

#### Scenario: Only one of the pair declared
- **WHEN** the axis file declares `stage_order` but no `current_stage`
- **THEN** the adapter emits an `AXIS_INVALID` naming the missing key and
  attaches no axis

### Requirement: Unreadable TOML is reported, never raised
A file the TOML parser rejects SHALL produce a `PARSE_ERROR` issue naming the file, and
the adapter SHALL continue with the remaining files and exit 0. The adapter SHALL NOT
raise, and SHALL NOT skip the file in silence.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_tomlreg.py -k malformed`

#### Scenario: Malformed file among sound ones
- **WHEN** the directory holds one file with invalid TOML syntax and two sound ones
- **THEN** the adapter emits a `PARSE_ERROR` naming the malformed file, emits the nodes
  from the two sound files, and exits 0

### Requirement: Report unread row-bearing tables
The adapter reads `[register]` and `[[item]]`. Any other top-level array of tables SHALL
produce an issue naming the file and the key, so that a row shape the adapter does not
understand cannot shrink the graph in silence.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_tomlreg.py -k unread`

#### Scenario: Unrecognized array of tables
- **WHEN** a file declares `[[note]]` rows alongside its `[[item]]` rows
- **THEN** the adapter emits an issue naming that file and the key `note`

### Requirement: Report a recognized field carrying the wrong shape
A key the adapter reads that holds a value of the wrong type SHALL produce a
`PARSE_ERROR` naming the file, the key and the shape found, and the adapter SHALL leave
that key's contribution out of the graph while keeping the rest of the row.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_tomlreg.py -k wrong_shape`

#### Scenario: refs is a string, not a list
- **WHEN** a row declares `refs = "beta/two"` rather than a list
- **THEN** the adapter emits a `PARSE_ERROR` naming the file and `refs`, emits the item
  node, and emits no `references` edge from it

### Requirement: Serve as the standing non-markdown gate
`lattice validate` run with the tomlreg profile and adapter against the
`tests/fixtures/mini-tomlreg` fixture SHALL exit 0 or 1 — never 2 — and SHALL produce a
non-zero node count and a non-zero edge count.

The edge count is normative. A finding count alone proves nothing: zero dangling
references over zero edges is satisfied by an adapter that built no edges at all.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_tomlreg.py -k gate`, and
`target/debug/lattice validate --profile profiles/tomlreg.yaml --adapter ./adapters/tomlreg --target tests/fixtures/mini-tomlreg --format json`

#### Scenario: Gate run over the fixture
- **WHEN** `lattice validate` runs with the tomlreg profile and adapter against
  `tests/fixtures/mini-tomlreg`
- **THEN** it exits 0 or 1, and the contract document carries at least one node and at
  least one edge

#### Scenario: Adapter is a program that exits 0
- **WHEN** the adapter is invoked directly with `--profile` and `--target` against a
  fixture containing malformed input
- **THEN** it writes a contract document to stdout and exits 0, carrying the malformed
  input as issues rather than as a non-zero exit

