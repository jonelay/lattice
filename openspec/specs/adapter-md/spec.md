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

1. **Single-table (backward-compatible):** `adapter.table` with `id_column`, and
   `node_kinds` declaring exactly one kind. All tables in all files use this single
   configuration. This is the existing behavior.

2. **Multi-table:** `adapter.tables`, an ordered list of table configs. Each entry
   SHALL declare `heading` (a regex pattern matched against the nearest ancestor
   heading), `kind` (the node kind name), and `id_column`. The profile's `node_kinds`
   SHALL declare every kind named by any table config entry.

In both forms `column_map` and `edge_columns` are optional and SHALL default to empty
maps: a table of IDs alone is a register, and omitting the maps is not an error.

When both `adapter.table` and `adapter.tables` are present, the adapter SHALL exit 2.
When neither is present, the adapter SHALL exit 2.

A profile that cannot be read or decoded, is not resolved schema `1`, or has no usable
file glob SHALL fail the adapter with exit 2. So SHALL an `adapter` block, `paths`,
`table`, or `tables` entry carrying a key the adapter does not read, with an error naming
the key; the core-owned `native` and `timeout_seconds` keys are admitted, since the
resolved document carries them through. A load error SHALL begin `could not load
profile <path>: ` where `<path>` is the `--profile` argument, so the core can name the
user's profile in place of its scratch copy.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py`, and
`cargo test -p latt-core --lib adapters::md`

#### Scenario: Single-table profile works as before
- **WHEN** the resolved profile declares `adapter.table` with one node kind
- **THEN** the adapter uses it for all tables, identical to pre-multikind behavior

#### Scenario: Multi-table profile with heading patterns
- **WHEN** the resolved profile declares `adapter.tables` with three entries, each
  with a heading pattern and distinct kind
- **THEN** the adapter loads all three configs and dispatches tables by heading match

#### Scenario: Maps omitted
- **WHEN** a `table` or a `tables` entry declares `id_column` and neither `column_map`
  nor `edge_columns`
- **THEN** the adapter loads it with both maps empty, and a row yields a node with no
  mapped attrs and no edges

#### Scenario: Both table and tables present
- **WHEN** a resolved profile declares both `adapter.table` and `adapter.tables`
- **THEN** the adapter exits 2

#### Scenario: Neither table nor tables present
- **WHEN** a resolved profile declares neither `adapter.table` nor `adapter.tables`
- **THEN** the adapter exits 2

#### Scenario: File selection is absent
- **WHEN** `adapter.paths.files` is empty or contains an empty pattern
- **THEN** the adapter exits 2 and reports that the profile must contain a file glob

#### Scenario: Unknown adapter key
- **WHEN** the resolved profile's `adapter` block carries `tabel` (mistyped) beside a
  valid `table`
- **THEN** the adapter exits 2 naming `tabel`

### Requirement: Select configured files beneath the target
For a directory target, the adapter SHALL walk its directories recursively and read only
regular files, and symlinks that resolve to files, whose target-relative slash-separated
paths match at least one configured glob. `*` SHALL match within one path component,
`?` SHALL match one non-separator character, and `**` SHALL span directories; in
particular, `**/*.md` matches both a root `.md` file and a nested one. For a file target,
the adapter SHALL apply the same matching to that file's name. Selected files SHALL be
processed in path order, making append-only document output deterministic.

A symlink that matches a glob but resolves to nothing SHALL produce a `PARSE_ERROR`
naming it, as SHALL a matching path that is not valid UTF-8; neither is read, and neither
is skipped in silence. Symlinked directories SHALL NOT be followed.

A missing or unreadable target, directory, directory entry or file SHALL produce a
`PARSE_ERROR` naming the affected path. The adapter SHALL continue wherever traversal
or reading can continue: an inaccessible selected file cannot erase nodes obtained from
the others.

When the target exists and is readable but no path matches the configured globs, the
adapter SHALL emit one `NO_MATCHING_FILES` issue at `info` severity whose message names
the globs, so a mistyped glob never reads as an empty register.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k "missing_target or non_utf8 or dangling_symlink or backslash or no_matching_files"`,
and `cargo test -p latt-adapter recursive_glob_also_matches_a_root_file`

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

#### Scenario: No files match
- **WHEN** the target is a directory holding only `notes.markdown`, or is that file,
  and `adapter.paths.files` is `["*.md"]`
- **THEN** the adapter emits no nodes and one `NO_MATCHING_FILES` issue at `info`
  severity naming `*.md`, and `latt validate` prints it as one line and exits 0

#### Scenario: Selected file cannot be read
- **WHEN** one selected file cannot be read and other selected files are readable
- **THEN** the adapter emits a `PARSE_ERROR` naming the unreadable file and continues
  with the readable files

#### Scenario: Dangling symlink matches a glob
- **WHEN** `gone.md` is a symlink to a missing file
- **THEN** the adapter emits a `PARSE_ERROR` naming `gone.md` and continues with the other
  files

#### Scenario: Path is not valid UTF-8
- **WHEN** a file whose name is not valid UTF-8 matches a configured glob
- **THEN** the adapter emits a `PARSE_ERROR` "path is not valid UTF-8" naming the path
  with the invalid bytes replaced by U+FFFD, carrying the path with those bytes escaped
  in its message so two such names stay distinct, in path order, and does not read the
  file

### Requirement: Recognize markdown tables by their separator row
The adapter SHALL recognize a table only when a pipe-bearing header line is immediately
followed by a pipe-bearing separator with the same number of cells. Every separator cell
SHALL contain at least one hyphen and nothing else but an optional leading colon,
trailing colon, or both, as GitHub Flavored Markdown accepts. Leading and trailing
pipes are optional. After the separator, consecutive pipe-bearing lines SHALL be data
rows until a line without a pipe ends the table; the adapter SHALL find further tables
later in the file.

Cell and header whitespace SHALL be trimmed. An escaped `\|` SHALL be content in its
cell rather than a delimiter, and non-ASCII headers and content SHALL pass through
unchanged. Pipe-bearing prose without a conforming separator row is not a table and
SHALL produce neither graph content nor an issue: it was never recognized as register
input. A header followed by a separator row whose cells all conform but whose width
differs is plainly meant as a table: when its heading would select a table
configuration, or the profile is multi-table, the adapter SHALL emit a
`PARSE_ERROR` at the header line naming both widths, and SHALL read none of the
pipe-bearing lines beneath it.

Fenced code is not register input. A fence opens on a line of three or more backticks
or tildes at any indentation — except that, as in CommonMark, a backtick run whose info
string holds a backtick is inline code and opens nothing — and closes only on a line holding a run of the same
character at least as long and nothing else. Lines inside a fence SHALL NOT be read as
tables or as headings, and a fence line SHALL end a table like any line without a pipe.
A fence still open at the end of the file SHALL be a `PARSE_ERROR` at the line that
opened it, since nothing after that line was read.

A blockquote does not hide register input. Before any line is classified, the adapter
SHALL strip its blockquote markers — each a `>` preceded by up to three spaces and
followed by an optional space, nested markers included — so a table, heading or fence
inside a blockquote is read as it would be outside one. Read as written, a quoted
header's first cell is `>`, its separator never conforms, and the table vanished with
no finding. Quote depth does not delimit a table: as outside a blockquote, a blank line
ends it.

Verified by: `cargo test -p latt-core --lib adapters::md::parse::tests`, `cargo test -p latt-adapter fence`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py -k "file_without_tables or separator_of_another_width or fenced_tables or blockquoted"`

#### Scenario: A blockquoted table is read
- **WHEN** a table's header, separator and rows each begin with `> `, under a `> ## `
  heading
- **THEN** the adapter reads the table as if unquoted: its rows become nodes at their own
  source lines, and the quoted heading selects its configuration

#### Scenario: Separator declares a table
- **WHEN** a header row is followed by an equal-width separator whose cells use one or
  more hyphens and optional alignment colons, as in `|-|`, `|--|` or `| :-: |`
- **THEN** the adapter reads the following pipe-bearing lines as table rows and preserves
  their one-based source line numbers

#### Scenario: Pipe text has no separator
- **WHEN** a file contains pipe-bearing text not followed by a conforming separator row
- **THEN** the adapter emits no node and no issue for that text

#### Scenario: Separator row has another width
- **WHEN** a three-column header is followed by a two-cell separator row
- **THEN** the adapter emits one `PARSE_ERROR` at the header line naming both widths and
  no node for the rows beneath it

#### Scenario: A table inside fenced code is an example
- **WHEN** a file holds a table, or a heading, between two fence lines
- **THEN** the adapter emits no node and no issue for it, and a heading inside the fence
  does not select the configuration for a table after it

#### Scenario: A fence is never closed
- **WHEN** a fence opens and no closing fence follows before the end of the file
- **THEN** the adapter emits one `PARSE_ERROR` at the opening line and reads no table
  after it

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
whose ID cell is non-empty after trimming. The node ID SHALL be that cell, with wrapping
emphasis stripped when its table sets `strip_emphasis`, its kind SHALL
be the profile's sole node kind, and every present `column_map` entry SHALL become a
string attr whose name is the mapping's value and whose value is the corresponding
trimmed cell, including an empty string, never stripped. The adapter SHALL append every declared row;
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
one cell may declare zero, one or many edges. A comma inside square brackets or
parentheses SHALL NOT split the cell: it belongs to a Markdown link's text or
destination, and splitting there emits fragments that name nothing in place of the
edge the row declares. A link is kept as written, not reduced to its text. When the
table sets `strip_emphasis`, each trimmed target SHALL lose its wrapping emphasis as
the ID does. Every
emitted edge SHALL carry the same file and line provenance as its source row.

The adapter SHALL NOT check that a target node exists. An unresolvable target is a
`VACANCY` finding at validation, which is the core's job; an adapter that dropped
the edge would hide it.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k "edge_columns or provenance"`
and `cargo test -p latt-core --lib adapters::md a_comma_inside_link`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py -k strip_emphasis`

#### Scenario: Comma-separated targets become edges
- **WHEN** a `Traces To` cell contains `REQ-1, REQ-2` and the profile maps that header to
  `traces_to`
- **THEN** the adapter emits two `traces_to` edges from the row ID, one to each trimmed
  target

#### Scenario: A comma inside a link does not split the cell
- **WHEN** a mapped edge cell contains `[REQ-2, REQ-3](http://x/a,b), REQ-4`
- **THEN** the adapter emits two edges, one to `[REQ-2, REQ-3](http://x/a,b)` and one
  to `REQ-4`, and no edge to a fragment such as `[REQ-2`

#### Scenario: Edge cell is empty
- **WHEN** a mapped edge cell is empty or contains only commas and whitespace
- **THEN** the adapter emits no edge for that cell

#### Scenario: Target node is absent
- **WHEN** a mapped edge cell names an ID no table declares
- **THEN** the adapter still emits the edge, and `latt validate` may report
  `VACANCY` for it

### Requirement: Report unreadable text, never raise it
A line of a selected file that is not valid UTF-8 SHALL produce a `PARSE_ERROR` at that
line, shown as written, as the adapter contract states; the adapter SHALL read the rest
of the file and the remaining selected files, and exit 0. A row on such a line is not
read. A table whose header or separator row is on such a line, and in multi-table form a
table under a heading on such a line, is not read, and a second `PARSE_ERROR` at the
table names the line and how many rows it withheld.
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
- **THEN** the adapter emits a `PARSE_ERROR` at the undecodable line, emits nodes from
  the sound files, and exits 0

#### Scenario: Undecodable row among sound rows
- **WHEN** one row of a table holds a byte that is not UTF-8
- **THEN** that row is a `PARSE_ERROR` at its line, shown as written, and the table's
  other rows become nodes

#### Scenario: Undecodable table header
- **WHEN** a table's header row holds a byte that is not UTF-8
- **THEN** the header line is a `PARSE_ERROR`, a second `PARSE_ERROR` at the table says
  how many rows were not read, and none of its rows becomes a node

#### Scenario: Undecodable multi-table heading
- **WHEN** in multi-table form a heading holds a byte that is not UTF-8
- **THEN** the heading line is a `PARSE_ERROR`, each table under it is a `PARSE_ERROR`
  naming the heading's line, and tables under other headings are read

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

Verified by: `cargo test -p latt-core --lib adapters::md`

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

Verified by: `cargo test -p latt-core --lib adapters::md`, and
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

Verified by: `cargo test -p latt-core --lib adapters::md`, and
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

Verified by: `cargo test -p latt-core --lib adapters::md`, and
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
interface document with `interface_version` `1.3`, append-only `nodes`, `edges`, and
`findings`, and an empty `pathways` list, then exit 0. Serialization or stdout failure SHALL
fail the adapter and exit 2: no valid document reached the core.

`latt validate` run with the md profile and adapter against
`tests/fixtures/mini-md` SHALL exit 1. Its adapter findings SHALL contain exactly
two `PARSE_ERROR` locations: `malformed.md:5` for the short row and `malformed.md:6` for
the empty ID. The separate adapter assertions SHALL preserve the fixture's three valid
nodes and three edges, so an expected error exit cannot hide a graph that shrank to
nothing.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k gate`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py`

#### Scenario: Gate run over the fixture
- **WHEN** `latt validate` runs with the md profile and adapter against
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
- **THEN** it writes an interface `1.3` document to stdout and exits 0, carrying malformed
  input as findings rather than as a non-zero exit

### Requirement: Resolve declared source addresses
The adapter SHALL apply the source-existence check to the nodes it builds, using each
table kind's `id_names` from the resolved profile's `node_kinds`, against the `--target`
directory. A kind without `id_names` SHALL contribute no `SOURCE_MISSING` finding.

`latt validate` run with the md sources profile and adapter against
`tests/fixtures/mini-md-sources` SHALL exit 0, and SHALL report warning
`SOURCE_MISSING` for exactly the fixture's stale rows — a renamed Rust test, a Rust
test whose module was renamed, a renamed Python test, a Python nested-class method, a
renamed Go test, and a deleted file — at their row provenance, besides one `PARSE_ERROR` for its
unreadable Python test file. The fixture's kinds are `orphan_ok`, so those are its only
warnings. The adapter assertions SHALL also show that the
resolvable rows produced no finding, and that the edge count is non-zero.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k sources`

#### Scenario: Verification table with stale rows
- **WHEN** `latt validate` runs over `tests/fixtures/mini-md-sources`
- **THEN** it exits 0, with `SOURCE_MISSING` exactly at the six stale rows and one
  `PARSE_ERROR` at the unreadable test file

#### Scenario: Strict promotes stale rows
- **WHEN** the same run adds `--strict`
- **THEN** it exits 1, and its error-severity findings are exactly those six
  `SOURCE_MISSING` and the one `PARSE_ERROR`

### Requirement: Strip wrapping emphasis when a table asks
`adapter.table` and every `adapter.tables` entry SHALL accept an optional boolean
`strip_emphasis`, default `false`. When it is `false` or absent, IDs and edge targets
SHALL be the cell text as written, emphasis included. When it is `true`, the adapter
SHALL remove from the ID and from each edge target every emphasis marker that wraps the
whole value — `**`, `__`, `*`, `_`, or a run of backticks of equal length at both ends —
trimming inside each, and repeating until no marker wraps it. A pair with nothing
inside, or whose marker recurs inside as a run of the marker's own length
(`**a** b **c**`, two spans), SHALL be kept as written; a run of another length does
not count (`*_**X**_*` strips to `X`). A Markdown link is not emphasis and SHALL be kept.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_md.py -k strip_emphasis`,
`cargo test -p latt-core --lib adapters::md strip_emphasis`, and
`cargo test -p latt-core --lib adapters::md wrapped_value_strips_back`

#### Scenario: A bold ID and a bold target are stripped
- **WHEN** a table sets `strip_emphasis: true` and a row's ID cell is `**1.1.1**` and its
  edge cell is `` **1.1.2**, `1.1.3` ``
- **THEN** the node ID is `1.1.1`, its edges target `1.1.2` and `1.1.3`, and a mapped
  attr cell `**bold**` stays `**bold**`

#### Scenario: Emphasis is kept by default
- **WHEN** a table does not declare `strip_emphasis`
- **THEN** a row whose ID cell is `**1.1.1**` has the ID `**1.1.1**`

#### Scenario: Nested markers strip to the value
- **WHEN** a table sets `strip_emphasis: true` and an ID cell is `` **`X`** ``
- **THEN** the node ID is `X`

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
- **WHEN** `adapter.paths.files` is `["docs/*.md"]`, `--target` is
  `docs`, and `docs/requirements.md` exists
- **THEN** the `NO_MATCHING_FILES` message names the target `docs`, says the
  patterns are relative to it, and names `.`, the directory above it, as the target to use

#### Scenario: No fix to offer
- **WHEN** the target holds no file any pattern would match, with or without dropping
  segments
- **THEN** the message names the globs and the target, and suggests no other target
