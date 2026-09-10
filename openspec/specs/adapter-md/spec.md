# adapter-md Specification

## Purpose
Reads profile-selected, pipe-delimited markdown tables into an interface document, so a
repository can keep a small typed node/edge register in ordinary `.md` files while the
profile, rather than the adapter, owns its vocabulary.

## Requirements
### Requirement: Load the resolved profile configuration
The adapter SHALL accept `--profile` and `--target`, and SHALL load the profile as the
core-resolved JSON document whose `resolved_schema` is `1`. It SHALL take its file globs
from `adapter.paths.files`.

The profile SHALL declare table configuration in one of two forms:

1. **Single-table (backward-compatible):** `adapter.table` with `id_column`,
   `column_map`, `edge_columns`, and `node_kinds` declaring exactly one kind. All
   tables in all files use this single configuration. This is the existing behavior.

2. **Multi-table:** `adapter.tables`, an ordered list of table configs. Each entry
   SHALL declare `heading` (a regex pattern matched against the nearest ancestor
   heading), `kind` (the node kind name), `id_column`, `column_map`, and
   `edge_columns`. The profile's `node_kinds` SHALL declare every kind named by any
   table config entry.

When both `adapter.table` and `adapter.tables` are present, the adapter SHALL exit 2.
When neither is present, the adapter SHALL exit 2.

A profile that cannot be read or decoded, is not resolved schema `1`, or has no usable
file glob SHALL fail the adapter with exit 2.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py`, and
`cargo test -p adapter-md`

#### Scenario: Single-table profile works as before
- **WHEN** the resolved profile declares `adapter.table` with one node kind
- **THEN** the adapter uses it for all tables, identical to pre-multikind behavior

#### Scenario: Multi-table profile with heading patterns
- **WHEN** the resolved profile declares `adapter.tables` with three entries, each
  with a heading pattern and distinct kind
- **THEN** the adapter loads all three configs and dispatches tables by heading match

#### Scenario: Both table and tables present
- **WHEN** a resolved profile declares both `adapter.table` and `adapter.tables`
- **THEN** the adapter exits 2

#### Scenario: Neither table nor tables present
- **WHEN** a resolved profile declares neither `adapter.table` nor `adapter.tables`
- **THEN** the adapter exits 2

#### Scenario: File selection is absent
- **WHEN** `adapter.paths.files` is empty or contains an empty pattern
- **THEN** the adapter exits 2 and reports that the profile must contain a file glob

### Requirement: Select configured files beneath the target
For a directory target, the adapter SHALL walk its directories recursively and read only
regular files, and symlinks that resolve to files, whose target-relative slash-separated
paths match at least one configured glob. `*` SHALL match within one path component,
`?` SHALL match one non-separator character, and `**` SHALL span directories; in
particular, `**/*.md` matches both a root `.md` file and a nested one. For a file target,
the adapter SHALL apply the same matching to that file's name. Selected files SHALL be
processed in path order, making append-only document output deterministic.

A missing or unreadable target, directory, directory entry or file SHALL produce a
`PARSE_ERROR` naming the affected path. The adapter SHALL continue wherever traversal
or reading can continue: an inaccessible selected file cannot erase nodes obtained from
the others.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k "missing_target or non_utf8"`,
and `cargo test -p adapter-md recursive_glob_also_matches_a_root_file`

#### Scenario: Directory target is filtered by glob
- **WHEN** a directory target contains matching markdown files at the root and in nested
  directories, alongside files that match none of `adapter.paths.files`
- **THEN** the adapter reads the matching files recursively and does not inspect the
  unmatched files as markdown tables

#### Scenario: Single file target is filtered by name
- **WHEN** the target is one file whose name matches a configured pattern
- **THEN** the adapter reads that file and reports its file name in provenance

#### Scenario: Target is absent
- **WHEN** `--target` names a path that does not exist or is unreadable
- **THEN** the adapter emits one `PARSE_ERROR`, emits no nodes, and exits 0

#### Scenario: Selected file cannot be read
- **WHEN** one selected file cannot be read and other selected files are readable
- **THEN** the adapter emits a `PARSE_ERROR` naming the unreadable file and continues
  with the readable files

### Requirement: Recognize markdown tables by their separator row
The adapter SHALL recognize a table only when a pipe-bearing header line is immediately
followed by a pipe-bearing separator with the same number of cells. Every separator cell
SHALL contain at least three hyphens, with an optional leading colon, trailing colon, or
both. Leading and trailing pipes are optional. After the separator, consecutive
pipe-bearing lines SHALL be data rows until a line without a pipe ends the table; the
adapter SHALL find further tables later in the file.

Cell and header whitespace SHALL be trimmed. An escaped `\|` SHALL be content in its
cell rather than a delimiter, and non-ASCII headers and content SHALL pass through
unchanged. Pipe-bearing prose without a conforming separator row is not a table and
SHALL produce neither graph content nor an issue: it was never recognized as register
input.

Verified by: `cargo test -p adapter-md parse::tests`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py -k file_without_tables`

#### Scenario: Separator declares a table
- **WHEN** a header row is followed by an equal-width separator whose cells use three or
  more hyphens and optional alignment colons
- **THEN** the adapter reads the following pipe-bearing lines as table rows and preserves
  their one-based source line numbers

#### Scenario: Pipe text has no separator
- **WHEN** a file contains pipe-bearing text not followed by a conforming separator row
- **THEN** the adapter emits no node and no issue for that text

#### Scenario: Cell contains an escaped pipe
- **WHEN** a data cell contains `\|`
- **THEN** the adapter keeps a literal `|` in that one cell rather than creating another
  column

### Requirement: Resolve configured columns against each table
For every recognized table, the adapter SHALL resolve the configured ID column either as
its zero-based numeric index or by exact header name. It SHALL resolve every
`column_map` and `edge_columns` key by exact header name. A configured attribute or edge
column absent from the headers SHALL emit a `PARSE_ERROR` naming that column, while all
present mappings continue to contribute graph content when the ID column resolves.

If the configured ID index is outside the header or the configured ID name is absent,
the adapter SHALL emit a `PARSE_ERROR` and skip the entire table. Without the configured
join key, emitting any row would invent identity. For a table with data rows, these
table-level findings SHALL carry the header's source line, derived from the first data
row; for a recognized table with no data rows they carry the header's source line number.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k "missing_id_column or missing_mapped_column"`

#### Scenario: Named ID column is present
- **WHEN** the profile names `ID` and a table has an exact `ID` header
- **THEN** that column supplies every valid row's node ID

#### Scenario: Numeric ID column is in range
- **WHEN** the profile declares a numeric ID column whose zero-based index is within the
  table header
- **THEN** the cell at that index supplies every valid row's node ID

#### Scenario: Configured ID column is absent
- **WHEN** the configured ID name is absent, or its configured numeric index is outside
  the table header
- **THEN** the adapter emits a `PARSE_ERROR` naming the configured column and emits no
  nodes or edges from that table

#### Scenario: Non-ID mapped column is absent
- **WHEN** a configured attribute or edge-column header is missing from a table
- **THEN** the adapter emits a `PARSE_ERROR` naming that header and still builds rows
  from the ID and mappings that are present

### Requirement: Build one node from every valid row
The adapter SHALL emit one node per row whose cell count equals the header count and
whose ID cell is non-empty after trimming. The node ID SHALL be that cell, its kind SHALL
be the profile's sole node kind, and every present `column_map` entry SHALL become a
string attr whose name is the mapping's value and whose value is the corresponding
trimmed cell, including an empty string. The adapter SHALL append every declared row;
duplicate-ID resolution belongs to validation and must not be hidden during ingest.

Each node SHALL carry target-relative file provenance and the row's one-based source
line. A row shorter or longer than its header SHALL emit a `PARSE_ERROR` stating both
widths and SHALL produce neither a node nor edges. A row with an empty ID SHALL likewise
emit a `PARSE_ERROR` and produce no graph content.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k "valid_nodes or short_rows or empty_ids or provenance"`

#### Scenario: Valid row becomes a typed node
- **WHEN** a table row has the header's width, a non-empty ID, and cells under mapped
  attribute columns
- **THEN** the adapter emits a node with that ID, the profile's node kind, mapped string
  attrs, and the row's file and line provenance

#### Scenario: Row has fewer columns than the header
- **WHEN** a table row contains fewer cells than its header
- **THEN** the adapter emits a `PARSE_ERROR` at that row and emits no node or edge from
  it

#### Scenario: Row has more columns than the header
- **WHEN** a table row contains more cells than its header
- **THEN** the adapter emits a `PARSE_ERROR` at that row and emits no node or edge from
  it

#### Scenario: Row has no identity
- **WHEN** the configured ID cell is empty after trimming
- **THEN** the adapter emits a `PARSE_ERROR` at that row and emits no node or edge from
  it

### Requirement: Expand mapped edge cells into references
For each valid row, the adapter SHALL turn every present `edge_columns` mapping into
edges from the row's node ID. The mapping value SHALL be the edge kind. The cell SHALL
be split on commas, each target SHALL be trimmed, and empty targets SHALL be omitted, so
one cell may declare zero, one or many edges. Every emitted edge SHALL carry the same
file and line provenance as its source row.

The adapter SHALL NOT check that a target node exists. An unresolvable target is a
`VACANCY` finding at validation, which is the core's job; an adapter that dropped
the edge would hide it.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k "edge_columns or provenance"`

#### Scenario: Comma-separated targets become edges
- **WHEN** a `Traces To` cell contains `REQ-1, REQ-2` and the profile maps that header to
  `traces_to`
- **THEN** the adapter emits two `traces_to` edges from the row ID, one to each trimmed
  target

#### Scenario: Edge cell is empty
- **WHEN** a mapped edge cell is empty or contains only commas and whitespace
- **THEN** the adapter emits no edge for that cell

#### Scenario: Target node is absent
- **WHEN** a mapped edge cell names an ID no table declares
- **THEN** the adapter still emits the edge, and `lattice validate` may report
  `VACANCY` for it

### Requirement: Report unreadable text, never raise it
A selected file whose bytes are not valid UTF-8 SHALL produce a `PARSE_ERROR` naming
the file, and the adapter SHALL continue with the remaining selected files and exit 0.
Markdown that decodes but contains no recognized table SHALL contribute no graph content
and no issue; the separator rule, not a general markdown parser, defines the input the
adapter claims to understand.

Every target-content problem the adapter reports SHALL be an error-severity
`PARSE_ERROR` with file-and-line provenance and no `node_id`. No pathway is read from this
format, so the document's `pathways` list SHALL be empty.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k "non_utf8 or file_without_tables"`

#### Scenario: Undecodable file among sound files
- **WHEN** one selected file is not valid UTF-8 and other selected files contain sound
  tables
- **THEN** the adapter emits a `PARSE_ERROR` naming the undecodable file, emits nodes
  from the sound files, and exits 0

#### Scenario: Decodable file contains no table
- **WHEN** a selected UTF-8 file contains no header-and-separator table pair
- **THEN** it contributes no nodes, edges or issues

### Requirement: Track the current heading during table discovery
The adapter SHALL track the most recent heading (lines beginning with one or more `#`
characters followed by a space) as it scans each file. When a markdown table is
recognized by its separator row, the adapter SHALL associate it with the text of the
most recent heading at any level that preceded it in the file. A table with no preceding
heading SHALL have an empty heading context.

The heading text is the content after the `#` prefix and space, trimmed of leading and
trailing whitespace.

Verified by: `cargo test -p adapter-md`

#### Scenario: Table inherits nearest heading
- **WHEN** a file contains `## User Needs` followed by a table, then `## Constraints`
  followed by another table
- **THEN** the first table's heading context is `User Needs` and the second's is
  `Constraints`

#### Scenario: Table with no preceding heading
- **WHEN** a file begins with a table before any heading
- **THEN** that table's heading context is empty

#### Scenario: Deeper heading overrides shallower
- **WHEN** `## Section` is followed by `### Subsection` then a table
- **THEN** the table's heading context is `Subsection`

### Requirement: Dispatch tables to configs by heading match
In multi-table mode, the adapter SHALL match each table's heading context against the
`heading` regex patterns in `adapter.tables`, in declared order. The first matching
entry SHALL supply the kind, id_column, column_map, and edge_columns for that table.

A table whose heading matches no entry SHALL be reported as an issue naming the file,
the heading, and the table's line number. The adapter SHALL emit no nodes or edges from
an unmatched table and SHALL continue processing.

A table with an empty heading context matches only an entry whose `heading` pattern
matches the empty string.

Verified by: `cargo test -p adapter-md`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py -k multikind`

#### Scenario: Heading matches a pattern
- **WHEN** a table appears under `## User Needs` and a table config declares
  `heading: "User Needs"`
- **THEN** that config's kind and column mappings apply to the table

#### Scenario: Heading matches a regex pattern
- **WHEN** tables appear under `## Domain 01`, `## Domain 02`, etc. and a table config
  declares `heading: "Domain \\d+"`
- **THEN** all domain tables use that config's kind and column mappings

#### Scenario: No config matches the heading
- **WHEN** a table appears under a heading that matches no entry in `adapter.tables`
- **THEN** the adapter emits an issue naming the file and heading, emits no nodes from
  that table, and continues processing

#### Scenario: First match wins
- **WHEN** a heading matches two entries in `adapter.tables`
- **THEN** the first entry in declared order supplies the config

### Requirement: Build nodes with per-config kind and mappings
In multi-table mode, each table's matched config SHALL supply the node kind, ID column,
attribute column mappings, and edge column mappings independently. A row in a table
matched to kind `user_need` SHALL produce a node of kind `user_need`; a row in a table
matched to kind `requirement` SHALL produce a node of kind `requirement`.

Column resolution (ID column lookup, mapped column lookup, missing column reporting)
SHALL apply per-table using the matched config, not a global config.

Verified by: `cargo test -p adapter-md`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py -k multikind`

#### Scenario: Different kinds from different tables in one file
- **WHEN** a file has a `User Needs` table (kind: user_need, columns: ID, Need,
  Traces to) and a `Constraints` table (kind: constraint, columns: ID, Constraint,
  Source)
- **THEN** the adapter emits user_need nodes from the first table and constraint nodes
  from the second, each with their own attribute mappings

#### Scenario: Shared kind from multiple headings
- **WHEN** eight tables under headings `Domain 01` through `Domain 08` all match a
  config with kind `requirement`
- **THEN** all eight tables produce `requirement` nodes with the same column mappings

### Requirement: Report tables under unmatched headings
In multi-table mode, a recognized markdown table whose heading context does not match
any configured pattern SHALL produce an issue at error severity naming the file, the
heading text (or "no heading" if empty), and the table's approximate line number. This
ensures a table the adapter does not understand cannot shrink the graph in silence.

In single-table mode, all recognized tables are read, so this requirement does not apply.

Verified by: `cargo test -p adapter-md`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py -k unmatched`

#### Scenario: Table under an unconfigured heading
- **WHEN** a file has a table under `## Appendix` and no table config matches `Appendix`
- **THEN** the adapter emits an issue naming the file and the heading `Appendix`

#### Scenario: Single-table mode reads all tables
- **WHEN** the profile uses `adapter.table` (singular) and a file has tables under
  various headings
- **THEN** all tables are read using the single config and no unmatched-heading issues
  are emitted

### Requirement: Emit the interface document and serve as the markdown-table gate
On target content it can report, the adapter SHALL write one newline-terminated JSON
interface document with `interface_version` `1.2`, append-only `nodes`, `edges`, and
`findings`, and an empty `pathways` list, then exit 0. Serialization or stdout failure SHALL
fail the adapter and exit 2: no valid document reached the core.

`lattice validate` run with the md profile and adapter against
`tests/fixtures/mini-md` SHALL exit 1. Its adapter findings SHALL contain exactly
two `PARSE_ERROR` locations: `malformed.md:5` for the short row and `malformed.md:6` for
the empty ID. The separate adapter assertions SHALL preserve the fixture's three valid
nodes and three edges, so an expected error exit cannot hide a graph that shrank to
nothing.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k gate`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py`

#### Scenario: Gate run over the fixture
- **WHEN** `lattice validate` runs with the md profile and adapter against
  `tests/fixtures/mini-md`
- **THEN** it exits 1, with `PARSE_ERROR` findings at exactly `malformed.md:5` and
  `malformed.md:6`

#### Scenario: Sound fixture rows survive malformed neighbors
- **WHEN** the adapter reads the complete mini-md fixture
- **THEN** its interface document carries nodes `REQ-1`, `REQ-2` and `REQ-3` and the
  three fixture-declared `traces_to` edges

#### Scenario: Adapter is a program that exits 0
- **WHEN** the adapter is invoked directly with `--profile` and `--target` against the
  fixture containing malformed rows
- **THEN** it writes an interface `1.2` document to stdout and exits 0, carrying malformed
  input as findings rather than as a non-zero exit
