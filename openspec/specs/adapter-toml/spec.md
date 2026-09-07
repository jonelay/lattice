## Purpose

Reads profile-selected TOML files into a contract document, dispatching multiple
array-of-tables to distinct node kinds via profile configuration, so a repository can
keep typed registers in `.toml` files while the profile owns all vocabulary.

## Requirements

### Requirement: Load the resolved TOML profile configuration
The adapter SHALL accept `--profile` and `--target`, and SHALL load the profile as the
core-resolved JSON document whose `resolved_schema` is `1`. It SHALL take its file globs
from `adapter.paths.files` and its table-to-kind mappings from `adapter.tables`.

`adapter.tables` SHALL be a map keyed by TOML array-of-tables name. Each entry SHALL
declare `kind` (the node kind name), `id_key` (the TOML key supplying the node ID),
`key_map` (a map from TOML key names to attribute names), and `edge_keys` (a map from
TOML key names to edge kind names).

A profile that cannot be read or decoded, is not resolved schema `1`, has no usable file
glob, or declares no table entries SHALL fail the adapter with exit 2. These are broken
adapter inputs, not malformed target content.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py`

#### Scenario: Resolved profile supplies the table vocabulary
- **WHEN** the resolved toml profile declares an `item` table entry with kind `item`,
  `id_key` of `id`, a `key_map`, and `edge_keys`
- **THEN** the adapter uses those names and mappings without carrying vocabulary in code

#### Scenario: Profile has no table entries
- **WHEN** a resolved profile declares an empty `adapter.tables` map
- **THEN** the adapter exits 2 and does not write a contract document

#### Scenario: File selection is absent
- **WHEN** `adapter.paths.files` is empty or contains an empty pattern
- **THEN** the adapter exits 2 and reports that the profile must contain a file glob

### Requirement: Select configured TOML files beneath the target
The adapter SHALL walk the target directory recursively and read only regular files whose
target-relative paths match at least one configured glob in `adapter.paths.files`.
Selected files SHALL be processed in sorted path order, making append-only document
output deterministic.

A missing or unreadable target or file SHALL produce a `PARSE_ERROR` naming the affected
path. The adapter SHALL continue wherever reading can continue.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k missing_target`

#### Scenario: Directory target is filtered by glob
- **WHEN** a directory target contains `*.toml` files alongside non-TOML files
- **THEN** the adapter reads only the files matching the configured globs

#### Scenario: Target is absent
- **WHEN** `--target` names a path that does not exist
- **THEN** the adapter emits one `PARSE_ERROR`, emits no nodes, and exits 0

### Requirement: Dispatch array-of-tables to configured node kinds
For each selected TOML file, the adapter SHALL iterate the file's top-level keys. For
each key that names an array of tables and matches an entry in `adapter.tables`, the
adapter SHALL emit one node per element of that array using the entry's `kind`, `id_key`,
and `key_map`.

The node ID SHALL be the value of the element's `id_key`. A row whose `id_key` is absent
or not a string SHALL emit a `PARSE_ERROR` carrying the file and the row's index, and
SHALL NOT produce a node.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k node`

#### Scenario: Array-of-tables rows become nodes
- **WHEN** a file declares `[[item]]` rows and the profile maps `item` to kind `item`
  with `id_key: "id"`
- **THEN** each row with a string `id` key becomes a node of kind `item`

#### Scenario: Multiple table kinds in one file
- **WHEN** a file declares both `[[item]]` and `[[note]]` rows and the profile maps both
- **THEN** the adapter emits nodes of kind `item` from `[[item]]` and nodes of kind
  `note` from `[[note]]`

#### Scenario: Row without an id key
- **WHEN** an array-of-tables element has no `id_key` field or its value is not a string
- **THEN** the adapter emits a `PARSE_ERROR` naming the file and row index, and emits no
  node for that row

### Requirement: Preserve native TOML types in attributes
Mapped attribute values SHALL preserve their TOML types when serialized to the contract
document's JSON. Strings become JSON strings, integers become JSON numbers, booleans
become JSON booleans, floats become JSON numbers, and TOML datetimes become JSON strings
in their TOML-canonical form. A mapped key whose value is an array or inline table SHALL
emit a `PARSE_ERROR` — the adapter reads flat values, not nested structures.

Verified by: `cargo test -p adapter-toml`

#### Scenario: Integer attribute is preserved
- **WHEN** a row declares `register_version = 0` and the profile maps `register_version`
- **THEN** the node's attr value is the JSON number `0`, not the string `"0"`

#### Scenario: Boolean attribute is preserved
- **WHEN** a row declares `active = true` and the profile maps `active`
- **THEN** the node's attr value is the JSON boolean `true`

#### Scenario: Nested value is rejected
- **WHEN** a mapped key's value is an inline table or array
- **THEN** the adapter emits a `PARSE_ERROR` and omits that attribute from the node

### Requirement: Expand edge keys into references
For each valid row, the adapter SHALL turn every present `edge_keys` mapping into edges.
The mapping value SHALL be the edge kind. The TOML value SHALL be handled by type:
a string produces one edge to that target; an array of strings produces one edge per
element. An `edge_keys` value of any other type SHALL emit a `PARSE_ERROR` and produce
no edges for that key.

The adapter SHALL NOT check that a target node exists.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k edge`

#### Scenario: String edge key becomes one edge
- **WHEN** a row declares `register = "beta"` and the profile maps `register` to
  `belongs_to`
- **THEN** the adapter emits one `belongs_to` edge from the row's node to `beta`

#### Scenario: Array edge key becomes multiple edges
- **WHEN** a row declares `refs = ["alpha/one", "beta/two"]` and the profile maps `refs`
  to `references`
- **THEN** the adapter emits two `references` edges, one to each target

#### Scenario: Edge key has wrong type
- **WHEN** a row declares `refs = 42` (not a string or array of strings)
- **THEN** the adapter emits a `PARSE_ERROR` and produces no edges for that key

### Requirement: Optional ID prefix for register-qualified IDs
When the profile declares `adapter.id_prefix: "file_stem"`, the adapter SHALL prepend
`<file-stem>/` to every node ID produced from array-of-tables rows. When `id_prefix` is
absent or null, IDs SHALL be the raw value of `id_key`.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k qualified`

#### Scenario: ID prefix active
- **WHEN** `id_prefix` is `file_stem` and `alpha.toml` holds a row with `id = "one"`
- **THEN** the node ID is `alpha/one`

#### Scenario: ID prefix absent
- **WHEN** `id_prefix` is not set and a row has `id = "E1.01"`
- **THEN** the node ID is `E1.01`

### Requirement: Optional header-table reader
When the profile declares `adapter.header`, the adapter SHALL read a singleton top-level
TOML table (not an array of tables) from each file and emit one node from it.
`adapter.header` SHALL declare `table` (the TOML table name), `kind`, and `key_map`.
The node ID SHALL be determined by `adapter.header.id`: when `"file_stem"`, the file stem
is the ID; when a string naming a key, that key's value is the ID.

A file missing the declared header table SHALL emit a `PARSE_ERROR` and skip the header
node while continuing with array-of-tables rows.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k header`

#### Scenario: Header table produces a node
- **WHEN** the profile declares `adapter.header` with `table: "register"`,
  `kind: "register"`, `id: "file_stem"`
- **THEN** a file `alpha.toml` with a `[register]` table produces a node `alpha` of kind
  `register` with mapped attributes

#### Scenario: Header table absent from file
- **WHEN** a file lacks the declared header table
- **THEN** the adapter emits a `PARSE_ERROR` naming the file, emits no header node, and
  continues reading the file's array-of-tables rows

### Requirement: Optional axis reader
When the profile declares `adapter.axis`, the adapter SHALL read ordering-axis values
from a specific TOML file. `adapter.axis` SHALL declare `source_file` (path relative to
target), `name` (the axis name), `order_key` (dot-path to the ordered list), and
`current_key` (dot-path to the current position string).

The register owns both values. The adapter SHALL NOT default or infer either. A profile
declaring no axis block has no axis, which is not a finding.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k axis`

#### Scenario: Axis attached
- **WHEN** the axis source file declares `stage_order = ["s1", "s2", "s3"]` and
  `current_stage = "s2"`, and the profile maps these via `order_key` and `current_key`
- **THEN** the graph carries the named axis with that order and current position

#### Scenario: Profile declares no axis
- **WHEN** `adapter.axis` is absent
- **THEN** the graph carries no axis

#### Scenario: Current position not in order
- **WHEN** `current_key` names a value absent from the `order_key` list
- **THEN** the adapter emits an `AXIS_INVALID` and attaches no axis

#### Scenario: Only one of the pair declared in the source
- **WHEN** the source file declares the order list but not the current position
- **THEN** the adapter emits an `AXIS_INVALID` naming the missing key and attaches no axis

### Requirement: Report unread array-of-tables
Any top-level TOML key that is an array of tables and is not declared in
`adapter.tables` SHALL produce an issue naming the file and the key, so that a row shape
the adapter does not understand cannot shrink the graph in silence.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k unread`

#### Scenario: Unrecognized array of tables
- **WHEN** a file declares `[[note]]` rows and the profile has no `note` entry in
  `adapter.tables`
- **THEN** the adapter emits an issue naming that file and the key `note`

### Requirement: Report unreadable TOML, never raise
A file the TOML parser rejects SHALL produce a `PARSE_ERROR` issue naming the file, and
the adapter SHALL continue with the remaining files and exit 0.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k malformed`

#### Scenario: Malformed file among sound ones
- **WHEN** the directory holds one file with invalid TOML syntax and two sound ones
- **THEN** the adapter emits a `PARSE_ERROR` naming the malformed file, emits the nodes
  from the two sound files, and exits 0

### Requirement: Report a recognized key carrying the wrong shape
A key the adapter reads that holds a value of an unexpected type SHALL produce a
`PARSE_ERROR` naming the file, the key and the type found. The adapter SHALL leave that
key's contribution out of the graph while keeping the rest of the row.

Verified by: `cargo test -p adapter-toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k wrong_shape`

#### Scenario: Edge key is an integer instead of a string or array
- **WHEN** a row declares `register = 42` rather than a string
- **THEN** the adapter emits a `PARSE_ERROR` naming the file and `register`, emits the
  node, and emits no `belongs_to` edge from it

### Requirement: Emit the contract document and serve as the TOML gate
On target content it can report, the adapter SHALL write one newline-terminated JSON
contract document with `contract_version` `1.1`, append-only `nodes`, `edges`, `axes`,
and `issues`, then exit 0. Serialization or stdout failure SHALL fail the adapter with
exit 2.

`lattice validate` run with the toml profile and adapter against
`tests/fixtures/mini-toml` SHALL exit 0 or 1 — never 2 — and SHALL produce a non-zero
node count and a non-zero edge count.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_toml.py -k gate`, and
`target/debug/lattice validate --profile profiles/toml.yaml --adapter ./adapters/toml --target tests/fixtures/mini-toml --format json`

#### Scenario: Gate run over the fixture
- **WHEN** `lattice validate` runs with the toml profile and adapter against
  `tests/fixtures/mini-toml`
- **THEN** it exits 0 or 1, and the contract document carries at least one node and at
  least one edge

#### Scenario: Adapter is a program that exits 0
- **WHEN** the adapter is invoked directly with `--profile` and `--target` against a
  fixture containing malformed input
- **THEN** it writes a contract document to stdout and exits 0, carrying the malformed
  input as issues rather than as a non-zero exit
