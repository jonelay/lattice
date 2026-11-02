## Purpose

Reads profile-selected TOML files into an interface document, dispatching multiple
array-of-tables to distinct node kinds via profile configuration, so a repository can
keep typed registers in `.toml` files while the profile owns all vocabulary.
## Requirements
### Requirement: Load the resolved TOML profile configuration
The adapter SHALL accept `--profile` and `--target`, and SHALL load the profile as the
core-resolved JSON document whose `resolved_schema` is `1`. It SHALL take its file globs
from `adapter.paths.files` and its table-to-kind mappings from `adapter.tables`.
`adapter.mode` SHALL accept `tables` or `directory` and SHALL default to `tables` when
absent.

`adapter.tables` SHALL be a map keyed by TOML array-of-tables name. Each entry SHALL
declare `kind` (the node kind name) and `id_key` (the TOML key supplying the node ID).
`key_map` (a map from TOML key names to attribute names) and `edge_keys` (a map from
TOML key names to edge entries) are optional and SHALL default to empty maps: a
table of IDs alone is a register. An edge entry SHALL be an edge kind name, or a table
with `kind` and optional `table` and `fallback`; any other key in it, a value of another
shape, or a `table` naming no `adapter.tables` entry SHALL fail the adapter with exit 2
naming the entry. A table entry MAY declare `id_prefix: kind`.

A profile that cannot be read or decoded, is not resolved schema `1`, has no usable file
glob, or declares no table entries SHALL fail the adapter with exit 2. So SHALL an
`adapter` block, `paths`, table, `header`, or `pathway` entry carrying a key the adapter
does not read, with an error naming the key; the core-owned `native` and
`timeout_seconds` keys are admitted, since the resolved document carries them through.
These are broken adapter inputs, not malformed target content. A load error SHALL
begin `could not load profile <path>: ` where `<path>` is the `--profile` argument, so
the core can name the user's profile in place of its scratch copy.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py`

#### Scenario: Resolved profile supplies the table vocabulary
- **WHEN** the resolved toml profile declares an `item` table entry with kind `item`,
  `id_key` of `id`, a `key_map`, and `edge_keys`
- **THEN** the adapter uses those names and mappings without carrying vocabulary in code

#### Scenario: Maps omitted
- **WHEN** a table entry declares `kind` and `id_key` and neither `key_map` nor
  `edge_keys`
- **THEN** the adapter loads it with both maps empty

#### Scenario: Profile has no table entries
- **WHEN** a resolved profile declares an empty `adapter.tables` map
- **THEN** the adapter exits 2 and does not write an interface document

#### Scenario: File selection is absent
- **WHEN** `adapter.paths.files` is empty or contains an empty pattern
- **THEN** the adapter exits 2 and reports that the profile must contain a file glob

#### Scenario: Mode is absent
- **WHEN** a resolved profile does not declare `adapter.mode`
- **THEN** the adapter reads configured array-of-tables using `tables` mode

#### Scenario: Unknown adapter key
- **WHEN** a resolved profile's table entry carries `edge_key` (mistyped) in place of
  `edge_keys`
- **THEN** the adapter exits 2 naming `edge_key`

### Requirement: Select configured TOML files beneath the target
The adapter SHALL walk the target directory recursively and read only regular files, and
symlinks that resolve to files, whose target-relative paths match at least one configured
glob in `adapter.paths.files`.
Selected files SHALL be processed in sorted path order, making append-only document
output deterministic.

A symlink that matches a glob but resolves to nothing SHALL produce a `PARSE_ERROR`
naming it, as SHALL a matching path that is not valid UTF-8; neither is read, and neither
is skipped in silence. Symlinked directories SHALL NOT be followed.

A missing or unreadable target or file SHALL produce a `PARSE_ERROR` naming the affected
path. The adapter SHALL continue wherever reading can continue.

When the target exists and is readable but no path matches the configured globs, the
adapter SHALL emit one `NO_MATCHING_FILES` issue at `info` severity whose message names
the globs, so a mistyped glob never reads as an empty register. A path that is not
valid UTF-8 and matches a glob is a match: its `PARSE_ERROR` stands alone.

Verified by: `cargo test -p latt-core --lib adapters::toml`,
`cargo test -p latt-adapter recursive_glob_also_matches_a_root_file`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k "missing_target or dangling_symlink or non_utf8_path or no_matching_files"`

#### Scenario: Directory target is filtered by glob
- **WHEN** a directory target contains `*.toml` files alongside non-TOML files
- **THEN** the adapter reads only the files matching the configured globs

#### Scenario: Target is absent
- **WHEN** `--target` names a path that does not exist
- **THEN** the adapter emits one `PARSE_ERROR`, emits no nodes, and exits 0

#### Scenario: No files match
- **WHEN** the target is a directory holding only `notes.md`, or is that file, and
  `adapter.paths.files` is `["registers/*.toml"]`
- **THEN** the adapter emits no nodes and one `NO_MATCHING_FILES` issue at `info`
  severity naming `registers/*.toml`

#### Scenario: Dangling symlink matches a glob
- **WHEN** `registers/gone.toml` is a symlink to a missing file
- **THEN** the adapter emits a `PARSE_ERROR` naming `registers/gone.toml` and continues with the other
  files

#### Scenario: Path is not valid UTF-8
- **WHEN** a file whose name is not valid UTF-8 matches a configured glob
- **THEN** the adapter emits a `PARSE_ERROR` "path is not valid UTF-8" naming the path
  with the invalid bytes replaced by U+FFFD, carrying the path with those bytes escaped
  in its message so two such names stay distinct, in path order, and does not read the
  file

#### Scenario: Only non-UTF-8 paths match
- **WHEN** the only paths matching the configured globs are not valid UTF-8
- **THEN** the adapter emits their `PARSE_ERROR`s, emits no nodes, and emits no
  `NO_MATCHING_FILES`

### Requirement: Optional directory mode for one-file-per-record registers
When `adapter.mode` is `directory`, each selected TOML file's root table SHALL be one
record and SHALL produce at most one node. `adapter.tables` SHALL declare exactly one
entry, which is the record configuration; a directory-mode profile declaring more
SHALL fail the adapter with exit 2, since only one could ever be read. The table entry's
map key SHALL be ignored; its `kind`, `id_key`, `key_map`, and `edge_keys` SHALL have the
same meanings they have for an array-of-tables row.

Directory mode SHALL continue to apply `adapter.id_prefix` and `adapter.pathway` when
configured. It SHALL NOT support `adapter.header`, because the file root is already the
record; a directory-mode profile declaring a header SHALL fail the adapter with exit 2.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k directory_mode`

#### Scenario: Directory mode declares two tables
- **WHEN** a directory-mode profile declares two `adapter.tables` entries
- **THEN** the adapter fails with exit 2, naming the count

#### Scenario: Each file root becomes one node
- **WHEN** directory mode selects three TOML files whose root tables each contain a
  string `id` and the one table entry declares `id_key: id`
- **THEN** the adapter emits three nodes using that entry's kind and attribute mappings

#### Scenario: Directory record emits edges
- **WHEN** a file-root record contains `refs = ["DEV-001", "DEV-002"]` and the
  table entry maps `refs` to `references`
- **THEN** the record node emits one `references` edge to each target

#### Scenario: Directory record ID is file-qualified
- **WHEN** directory mode sets `id_prefix: file_stem` and `device-001.toml` contains
  `id = "DEV-001"`
- **THEN** the node ID is `device-001/DEV-001`

#### Scenario: Malformed directory record does not stop the register
- **WHEN** one selected file is malformed TOML and later selected files are sound
- **THEN** the adapter emits a `PARSE_ERROR` for the malformed file and nodes from the
  sound files, then exits 0

#### Scenario: Header is rejected in directory mode
- **WHEN** a directory-mode profile also declares `adapter.header`
- **THEN** the adapter exits 2 and reports that the header is unsupported in directory mode

### Requirement: Dispatch array-of-tables to configured node kinds
For each selected TOML file, the adapter SHALL iterate the file's top-level keys. For
each key that names an array of tables and matches an entry in `adapter.tables`, the
adapter SHALL emit one node per element of that array using the entry's `kind`, `id_key`,
and `key_map`.

The node ID SHALL be the value of the element's `id_key`. A row whose `id_key` is absent
or not a string SHALL emit a `PARSE_ERROR` carrying the file and the row's index, and
SHALL NOT produce a node.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
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
Mapped attribute values SHALL preserve their TOML types when serialized to the interface
document's JSON. Strings become JSON strings, integers become JSON numbers, booleans
become JSON booleans, floats become JSON numbers, and TOML datetimes become JSON strings
in their TOML-canonical form. An array of scalars SHALL become a JSON array of its
entries, each preserved the same way, so a declared `list` attr reads. An array holding
any array or table entry SHALL yield no attr, and each such entry SHALL be one
`UNREAD_ENTRIES` warning at its own line, naming the item, the key, the entry's
position and count, the entry as written (on one line, cut at 120 characters), its
type, and that the attr is not set. A count alone tells the user that something was
missed but not what; the entry is the gap they came to find. Its scalar entries SHALL NOT be read alone: a list is one value, and a subset
of it is a value the register never declared. The TOML is valid, so the finding is a
warning; an edge array differs because each string entry is a true edge on its own.
A mapped key whose value is an inline table SHALL emit a `PARSE_ERROR` and yield no
attr, since no row's value can be read under that mapping.

A `key_map` value MAY instead be a table of shapes: `scalars` names the attr for the
array's scalar entries, and `tables` maps a field of table entries to the attr for that
field's values; several shapes MAY name one attr, filled in entry order. Each declared
attr SHALL be set to the values of its shape, `[]` when none appear. An entry no
declared shape claims SHALL be one `UNREAD_ENTRIES` warning at its line, as written,
and SHALL NOT withhold any attr: each attr's claim is its own shape, which the entry
does not falsify. A claimed entry that cannot be read SHALL be a `PARSE_ERROR` at its
line and withhold only the attr it would fill. A value that is not an array SHALL be a
`PARSE_ERROR`. A shapes table with neither key, or with another key, SHALL fail the
adapter with exit 2.

An `edge_keys` entry's `entry_key` SHALL make a table entry, or a table value, whose
named field is a string an edge to that string; any other non-string entry remains a
`PARSE_ERROR`.

A `key_map` key SHALL be looked up as an edge key is: as written first, then as a
dotted path through nested tables, with an absent component an absent key and a
component that is not a table a `PARSE_ERROR` naming the key and that component.
Read only as written, a nested key's attr was dropped with no finding.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k "dotted_key_map or unread or shape"`

#### Scenario: Integer attribute is preserved
- **WHEN** a row declares `register_version = 0` and the profile maps `register_version`
- **THEN** the node's attr value is the JSON number `0`, not the string `"0"`

#### Scenario: Boolean attribute is preserved
- **WHEN** a row declares `active = true` and the profile maps `active`
- **THEN** the node's attr value is the JSON boolean `true`

#### Scenario: Dotted mapped key follows nested tables
- **WHEN** the profile maps `status.state` and a record declares `[status]` with
  `state = "live"`
- **THEN** the node carries the mapped attr with value `"live"`
- **AND WHEN** another record declares `status = "flat"`
- **THEN** the adapter emits a `PARSE_ERROR` naming `status.state` and `status`, and
  that node has no such attr

#### Scenario: Array of scalars is a list attr
- **WHEN** a mapped key's value is `["adb", "ads"]`
- **THEN** the node's attr is the JSON array `["adb", "ads"]` and no finding is emitted

#### Scenario: A table among array entries leaves the attr unset
- **WHEN** a mapped key's value is `["a", { glob = "b" }, "c"]`
- **THEN** the node has no such attr, and one `UNREAD_ENTRIES` warning, at the entry's
  line, names the item, the key, `entry 2 of 3`, `{ glob = "b" }`, that it is a table,
  and that the attr is not set

#### Scenario: Each unread entry is its own finding
- **WHEN** a mapped key's array holds two unread entries on lines 13 and 15
- **THEN** the adapter emits two `UNREAD_ENTRIES` warnings, at lines 13 and 15, each
  showing its own entry

#### Scenario: Declared shapes keep what they claim
- **WHEN** `file-types` is mapped with `scalars: file_types` and `tables: { glob:
  file_globs }`, and a row's value is `["a", { glob = "b" }, "c", { regex = "r" }]`
- **THEN** the node has `file_types: ["a", "c"]` and `file_globs: ["b"]`, and one
  `UNREAD_ENTRIES` warning shows `{ regex = "r" }` as entry 4 of 4

#### Scenario: Two shapes fill one attr
- **WHEN** `language-servers` is mapped with `scalars: servers` and `tables: { name:
  servers }`, and a row's value is `["a", { name = "b", except-features = ["f"] }, "c"]`
- **THEN** the node has `servers: ["a", "b", "c"]` and no finding

#### Scenario: An entry key reads a table entry's target
- **WHEN** an `edge_keys` entry declares `entry_key: name` and a row's value is
  `["g", { name = "h" }, { other = "q" }]`
- **THEN** the adapter emits edges to `g` and `h`, and one `PARSE_ERROR` shows
  `{ other = "q" }` as entry 3 of 3

#### Scenario: A scalar JSON cannot hold is not dropped from a list
- **WHEN** a mapped key's value is `["h", inf]`
- **THEN** the node has no such attr and the adapter emits a `PARSE_ERROR` naming the key

#### Scenario: Nested value is rejected
- **WHEN** a mapped key's value is an inline table
- **THEN** the adapter emits a `PARSE_ERROR` and omits that attribute from the node

### Requirement: Expand edge keys into references
For each valid row, the adapter SHALL turn every present `edge_keys` mapping into edges.
The entry's kind SHALL be the edge kind. When the edge key is absent and the entry
declares `fallback`, the adapter SHALL read the `fallback` key in its place, looked up
the same way. When the entry declares `table`, each target SHALL be the ID that table
gives a row with that ID value in the same file, its `id_prefix` and the file stem
included; otherwise the target is the value as written. The TOML value SHALL be
handled by type:
a string produces one edge to that target; an array produces one edge per string
element. An array element that is not a string SHALL NOT produce an edge, and each
such element SHALL be one `PARSE_ERROR` at its own line, naming the item, the key, the
element's position and count, the element as written and its type; the string
elements SHALL still produce edges. An `edge_keys` value that is neither a string nor an array SHALL emit
a `PARSE_ERROR` and produce no edges for that key.

An edge key SHALL be looked up first as written. When the row has no such key and the
key contains dots, the adapter SHALL read it as a dotted path through nested tables, so
`people.leads` names `leads` inside the row's `people` table. Read only as written, a
nested key matched nothing and its edges were dropped with no finding. A path that ends
early because a component is absent SHALL be an absent key, as an undeclared key is.
A path that runs into a value that is not a table SHALL emit a `PARSE_ERROR` naming the
key and the component that blocked it, and produce no edges for that key.

The adapter SHALL NOT check that a target node exists.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k edge`

#### Scenario: Edge target built by its table, with a fallback key
- **WHEN** tables `language` and `grammar` both declare `id_prefix: kind`, the
  `language` entry maps `grammar` to `{kind: uses_grammar, table: grammar, fallback:
  name}`, and a language row named `ada` has no `grammar` key
- **THEN** the edge runs from `language/ada` to `grammar/ada`

#### Scenario: Dotted edge key follows nested tables
- **WHEN** a record declares `[people]` with `leads = ["DEV-002"]` and the profile maps
  `people.leads` to `references`
- **THEN** the adapter emits one `references` edge to `DEV-002`

#### Scenario: Dotted edge key runs into a non-table
- **WHEN** the profile maps `people.leads` and a record declares `people = "not a table"`
- **THEN** the adapter emits a `PARSE_ERROR` naming `people.leads` and `people` and
  produces no edges for that key

#### Scenario: String edge key becomes one edge
- **WHEN** a row declares `register = "beta"` and the profile maps `register` to
  `belongs_to`
- **THEN** the adapter emits one `belongs_to` edge from the row's node to `beta`

#### Scenario: Array edge key becomes multiple edges
- **WHEN** a row declares `refs = ["alpha/one", "beta/two"]` and the profile maps `refs`
  to `references`
- **THEN** the adapter emits two `references` edges, one to each target

#### Scenario: Array edge key mixes strings and tables
- **WHEN** a row declares `members = ["DEV-002", { github = "x" }, "DEV-003", 7]` and
  the profile maps `members` to `references`
- **THEN** the adapter emits `references` edges to `DEV-002` and `DEV-003` and one
  `PARSE_ERROR` naming `members`, 2 of 4 entries, `table` and `integer`

#### Scenario: Edge key has wrong type
- **WHEN** a row declares `refs = 42` (not a string or array of strings)
- **THEN** the adapter emits a `PARSE_ERROR` and produces no edges for that key

### Requirement: Optional ID prefix for register-qualified IDs
When the profile declares `adapter.id_prefix: "file_stem"`, the adapter SHALL prepend
`<file-stem>/` to every node ID produced from array-of-tables rows. When a `tables`
entry declares `id_prefix: kind`, the adapter SHALL prepend `<kind>/` to its rows' IDs,
inside the file stem when both are set (`<file-stem>/<kind>/<id>`), so two tables
whose rows share names do not collide in the one ID namespace. When neither is set,
IDs SHALL be the raw value of `id_key`.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k qualified`

#### Scenario: ID prefix active
- **WHEN** `id_prefix` is `file_stem` and `alpha.toml` holds a row with `id = "one"`
- **THEN** the node ID is `alpha/one`

#### Scenario: Kind prefixes keep shared names apart
- **WHEN** tables `language` and `grammar` declare `id_prefix: kind` and each has a row
  named `rust`
- **THEN** nodes `language/rust` and `grammar/rust` both exist and no duplicate is reported

#### Scenario: ID prefix absent
- **WHEN** `id_prefix` is not set and a row has `id = "E1.01"`
- **THEN** the node ID is `E1.01`

### Requirement: Optional header-table reader
When the profile declares `adapter.header`, the adapter SHALL read a singleton top-level
TOML table (not an array of tables) from each file and emit one node from it.
`adapter.header` SHALL declare `table` (the TOML table name), `kind` and `id`; its
`key_map` is optional and SHALL default to an empty map.
The node ID SHALL be determined by `adapter.header.id`: when `"file_stem"`, the file stem
is the ID; when a string naming a key, that key's value is the ID.

A file missing the declared header table SHALL emit a `PARSE_ERROR` and skip the header
node while continuing with array-of-tables rows.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
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

### Requirement: Optional pathway reader
When the profile declares `adapter.pathway`, the adapter SHALL read ordering-pathway values
from a specific TOML file. `adapter.pathway` SHALL declare `source_file` (path relative to
target), `name` (the pathway name), `order_key` (dot-path to the ordered list), and
`current_key` (dot-path to the current position string).

The register owns both values. The adapter SHALL NOT default or infer either. A profile
declaring no pathway block has no pathway, which is not a finding.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k pathway`

#### Scenario: Pathway attached
- **WHEN** the pathway source file declares `stage_order = ["s1", "s2", "s3"]` and
  `current_stage = "s2"`, and the profile maps these via `order_key` and `current_key`
- **THEN** the graph carries the named pathway with that order and current position

#### Scenario: Profile declares no pathway
- **WHEN** `adapter.pathway` is absent
- **THEN** the graph carries no pathway

#### Scenario: Current position not in order
- **WHEN** `current_key` names a value absent from the `order_key` list
- **THEN** the adapter emits an `PATHWAY_INVALID` and attaches no pathway

#### Scenario: Only one of the pair declared in the source
- **WHEN** the source file declares the order list but not the current position
- **THEN** the adapter emits an `PATHWAY_INVALID` naming the missing key and attaches no pathway

### Requirement: Report unread array-of-tables
Any top-level TOML key that is an array of tables and is not declared in
`adapter.tables` SHALL produce an issue naming the file and the key, so that a row shape
the adapter does not understand cannot shrink the graph in silence.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k unread`

#### Scenario: Unrecognized array of tables
- **WHEN** a file declares `[[note]]` rows and the profile has no `note` entry in
  `adapter.tables`
- **THEN** the adapter emits an issue naming that file and the key `note`

### Requirement: Provenance names the line
Every node, edge and finding the adapter emits SHALL carry the one-based line it was
written on: a row's node the line of its `[[table]]` header, a header node its
`[table]` header, an edge the line of its target value (each array entry its own), and
a finding about a key the line of that key's value, or of the array entry it is about. A `directory`-mode record is the
whole file, so its node SHALL be at line 0. At line 0 for everything, a finding could
say what was wrong but not where.

Verified by: `cargo test -p latt-core --lib adapters::toml::tree`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k line_they_were_written`

#### Scenario: Rows, edges and findings at their lines
- **WHEN** `[[language]]` is on line 2 with `grammar = "g"` on line 4, a `file-types`
  array starting on line 5 holds a table entry on line 7, and `[[grammar]]` is on line 10
- **THEN** the language node is at line 2, the grammar node at line 10, the edge at
  line 4, and the finding for the unread entry at line 7

### Requirement: Report unreadable TOML, never raise
A file the TOML parser rejects SHALL produce a `PARSE_ERROR` issue naming the file, and
the adapter SHALL continue with the remaining files and exit 0.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k malformed`

#### Scenario: Malformed file among sound ones
- **WHEN** the directory holds one file with invalid TOML syntax and two sound ones
- **THEN** the adapter emits a `PARSE_ERROR` naming the malformed file, emits the nodes
  from the two sound files, and exits 0

### Requirement: Report a recognized key carrying the wrong shape
A key the adapter reads that holds a value of an unexpected type SHALL produce a
`PARSE_ERROR` naming the file, the key and the type found. The adapter SHALL leave that
key's contribution out of the graph while keeping the rest of the row.

Verified by: `cargo test -p latt-core --lib adapters::toml`, and
`.venv/bin/python -m pytest tests/test_adapter_toml.py -k wrong_edge_shape`

#### Scenario: Edge key is an integer instead of a string or array
- **WHEN** a row declares `register = 42` rather than a string
- **THEN** the adapter emits a `PARSE_ERROR` naming the file and `register`, emits the
  node, and emits no `belongs_to` edge from it

### Requirement: Emit the interface document and serve as the TOML gate
On target content it can report, the adapter SHALL write one newline-terminated JSON
interface document with `interface_version` `1.3`, append-only `nodes`, `edges`, `pathways`,
and `findings`, then exit 0. Serialization or stdout failure SHALL fail the adapter with
exit 2.

`latt validate` run with the toml profile and adapter against
`tests/fixtures/mini-toml` SHALL exit 0 or 1 — never 2 — and SHALL produce a non-zero
node count and a non-zero edge count.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_toml.py -k gate`, and
`target/debug/latt validate --profile profiles/toml.yaml --adapter ./adapters/toml --target tests/fixtures/mini-toml --format json`

#### Scenario: Gate run over the fixture
- **WHEN** `latt validate` runs with the toml profile and adapter against
  `tests/fixtures/mini-toml`
- **THEN** it exits 0 or 1, and the interface document carries at least one node and at
  least one edge

#### Scenario: Adapter is a program that exits 0
- **WHEN** the adapter is invoked directly with `--profile` and `--target` against a
  fixture containing malformed input
- **THEN** it writes an interface document to stdout and exits 0, carrying the malformed
  input as findings rather than as a non-zero exit

### Requirement: Resolve declared source addresses
The adapter SHALL read `node_kinds` from the resolved profile and SHALL apply the
source-existence check to the nodes it builds — table, directory-mode and header nodes
alike — against the `--target` directory. A kind without `id_names` SHALL contribute no `SOURCE_MISSING`
finding.

Verified by: `cargo test -p latt-core --locked --lib adapters::toml sources`

#### Scenario: TOML row naming a deleted file
- **WHEN** a table's kind declares `id_names: file` and a row's ID names no file in the target
- **THEN** the document carries a warning `SOURCE_MISSING` for that node at the row's provenance

#### Scenario: Existing fixture carries no source findings
- **WHEN** the adapter runs over `tests/fixtures/mini-toml`, whose profile declares no `id_names`
- **THEN** its document carries no `SOURCE_MISSING` finding

### Requirement: No matching files names the target and a fix
The `NO_MATCHING_FILES` message SHALL also name the target as given, and say that
`adapter.paths.files` patterns are relative to it. When a pattern begins with literal
directory segments equal to the target's own trailing path segments, and dropping
those segments makes the pattern match at least one file beneath the target, the
message SHALL name the directory as many levels above the target as the segments
dropped, as the target to use. The adapter SHALL look only beneath the target, and
severity and exit code are unchanged.

Verified by: `cargo test -p latt-adapter --locked no_matching_files`.

#### Scenario: Target one level too deep
- **WHEN** `adapter.paths.files` is `["registers/*.toml"]`, `--target` is
  `registers`, and `registers/requirements.toml` exists
- **THEN** the `NO_MATCHING_FILES` message names the target `registers`, says the
  patterns are relative to it, and names `.`, the directory above it, as the target to use

#### Scenario: No fix to offer
- **WHEN** the target holds no file any pattern would match, with or without dropping
  segments
- **THEN** the message names the globs and the target, and suggests no other target
