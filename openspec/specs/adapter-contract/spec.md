# adapter-contract Specification

## Purpose
The interchange between a per-repo adapter and the core: an adapter program emits its
register as a serialized graph plus issues, and the core ingests it. This keeps ingest in
the host repo's own language while the core is free to be written in another.
## Requirements
### Requirement: Adapter is a program emitting a serialized graph
An adapter SHALL be an executable program or a named native adapter. Given the profile
and the target path, it SHALL produce a single document containing the nodes,
edges, any ordering pathway, and the issues it collected while reading the
register. An external adapter writes its document to stdout; a native adapter
produces the same JSON interface document in memory for ingestion through the
shared document validator.

The core SHALL NOT execute external adapter code in its own process. A native
adapter is linked at compile time and produces its document through the same
JSON interface, so the contract boundary is semantic (the document schema) rather
than mechanical (process isolation). Shared ingestion ensures that native and
external adapters are subject to identical schema validation, version checking,
and duplicate-ID detection.

The adapter's own reading errors travel as issues inside the document, preserving
the standing contract that unreadable input is reported and never dropped.
Concretely: a recognized candidate the adapter cannot read or represent SHALL
produce an issue — a failed target-file read (undecodable bytes, missing
permission, a directory where a file was expected) is register input failing,
not the adapter failing, and SHALL NOT become a non-zero exit. When the
candidate's identity remains readable, the adapter SHOULD preserve the node and
omit only the unreadable contribution.

Verified by: `cargo test --test contract` and `cargo test --test native`

#### Scenario: Adapter emits a graph the core ingests
- **WHEN** an adapter program emits a document with two nodes and one edge between them
- **THEN** the core's graph contains those two nodes and that edge, with the provenance
  each carried in the document

#### Scenario: Adapter-collected issues survive ingest
- **WHEN** an adapter emits a document carrying a `PARSE_ERROR` issue and zero nodes
- **THEN** the core reports that `PARSE_ERROR` rather than reporting an empty register

#### Scenario: Read failure is an issue, not an adapter crash
- **WHEN** a path the adapter would read as register input cannot be read — undecodable
  bytes, or a directory where a file was expected
- **THEN** the adapter reports a `PARSE_ERROR` for that path, continues with the
  remaining input, and exits 0

#### Scenario: Adapter written in any language
- **WHEN** an adapter program is not written in the core's implementation language
- **THEN** the core ingests its output identically, because only the document is the
  interface

#### Scenario: Native adapter uses the same ingest path
- **WHEN** a native adapter produces a document in memory
- **THEN** the core applies the same schema validation, version checking, and
  duplicate-ID detection as it does to a parsed external interface document

### Requirement: Document schema
The document SHALL be a JSON object carrying `interface_version`, `nodes`, `edges`, `pathways`
and `findings`. A node SHALL carry `id`, `kind`, `attrs` and `provenance`; an edge `src`,
`tgt`, `kind`, optional `attrs`, `provenance`, and optional `declared_by`; a pathway `name`, `order` and `current`; a finding
`severity`, `code`, `message`, `provenance` and `node_id`, and optionally `state`. A
provenance SHALL carry `file` and `line`; `line` SHALL be an integer from 0 to
4294967295, one-based or zero when no exact line is available. A negative `line`, or
one above 4294967295, is a schema failure naming the line, wherever the provenance
sits: on a node, an edge, a finding, or a value of a node's `attr_provenance`. The
bound SHALL hold for every accepted `interface_version`. Each of the
four arrays MAY be absent, and
the core reads an absent one as empty; a present one SHALL be a list. The requirement
"Document keys are closed" lists the keys each object may carry.

A provenance's `file` SHALL name a path relative to the target root whenever the file
lies inside the target, and SHALL be left as the adapter wrote it otherwise, so that a
path outside the target and a placeholder such as `<profile>` still say what they mean.
An absolute path records where a checkout happens to sit, and two runs of one commit
under different paths would then disagree byte for byte — which defeats any comparison
of two runs, the trace baselines included.

A node's `attrs` SHALL be an arbitrary JSON object, nesting objects and arrays to any
depth. An adapter whose source format has types JSON does not carry SHALL encode them
before emitting; a value that is not JSON is a schema failure, not a finding.
An edge's optional `attrs` SHALL obey the same rule and default to an empty object when
absent; an explicit non-object value is a schema failure.

An edge's optional `declared_by` (interface 1.3) SHALL be the index in `nodes` of the
node entry whose source row declared the edge. That entry need not be the one for the
edge's `src`: a capability's `contains` edge to a requirement is declared by the
requirement's row. A `declared_by` that is not a non-negative integer, or that is not
less than the number of node entries, is a schema failure.

A node's optional `attr_provenance` (interface 1.4) SHALL map an attr name to the
provenance where that attr's value was written, with a provenance's closed keys. Each
name SHALL be an attr the node's `attrs` holds; a name it does not hold, a value that is
not a provenance, or an `attr_provenance` that is not an object is a schema failure. An
attr it does not name was written at the node's `provenance`.

Each issue SHALL carry the severity its adapter chose. The core SHALL NOT recompute an
adapter issue's severity from its own defaults, because an adapter may emit a code the
core has never heard of and recomputation would silently reduce it to a fallback. The
profile's severity overrides still apply after ingest.

Verified by: `cargo test --test contract refused`, `cargo test --test contract declared_by`,
`cargo test -p latt-core --locked --test provenance_line`, and
`cargo test -p latt --locked --test cli provenance_line`

#### Scenario: Nested attribute values survive ingest
- **WHEN** a document declares a node whose `attrs` contain a nested object and an array
  of objects
- **THEN** the graph's node carries those values unchanged

#### Scenario: A negative provenance line is refused
- **WHEN** an external adapter emits a provenance whose `line` is below zero
- **THEN** ingest fails with a contract error, and `latt validate` exits 2

#### Scenario: A line beyond 32 bits is refused
- **WHEN** an external adapter emits a node, an edge, a finding or an `attr_provenance` value whose `line` is 4294967296, or an integer above 9223372036854775807
- **THEN** ingest fails with a contract error naming the line as written, and `latt validate` exits 2

#### Scenario: The largest 32-bit line is accepted
- **WHEN** an external adapter emits a provenance whose `line` is 4294967295
- **THEN** ingest accepts it and the node, edge, finding or attr carries that line

#### Scenario: Line zero still means no line
- **WHEN** an external adapter emits a provenance whose `line` is 0
- **THEN** ingest accepts it, and the output prints it as it does today

#### Scenario: A declared_by naming no node entry is refused
- **WHEN** an adapter emits an edge whose `declared_by` is negative, not an integer, or
  not less than the number of node entries
- **THEN** ingest fails with a contract error, and `latt validate` exits 2

#### Scenario: Provenance is relative to the target
- **WHEN** an adapter runs with a `--target` given as an absolute path and emits
  provenance for a file inside it
- **THEN** the document's `file` names that file relative to the target root, so the
  same commit checked out at two paths emits the same bytes

#### Scenario: Adapter severity is preserved
- **WHEN** a document carries an issue with a code the core has no default severity for,
  at severity `error`, and the profile overrides nothing
- **THEN** the core reports that issue at `error` rather than at its warning fallback

### Requirement: Document order is significant
The order of `nodes` and of `edges` in the document SHALL be preserved by ingest. Node
order determines which occurrence of a repeated ID is the first and therefore becomes the
node. Edge order is the order in which edges enter the graph, which the trace report's
within-entry ordering tie-breaks against.

An occurrence that is not the first is not read, and neither are the edges it declared:
the core SHALL NOT ingest an edge whose `declared_by` names a node entry refused as a
repeated ID, and that entry's duplicate finding SHALL say how many such edges were not
read. An edge without `declared_by` SHALL be ingested whatever declared it, as under
interface 1.2, since the core cannot tell which row it came from; each duplicate finding
for its `src` SHALL count it as read though the repeated entry may have declared it,
so the finding never claims a rejection the core could not make total. Every adapter this repository ships SHALL set
`declared_by` on each edge a node row declares.

Verified by: `cargo test --test contract order`, `cargo test --test contract declared`,
and `.venv/bin/python -m pytest tests -k "duplicate_rows or repeated"`

#### Scenario: Node order decides the surviving duplicate
- **WHEN** a document declares `REQ-1` with attrs A before declaring it with attrs B
- **THEN** the graph's `REQ-1` carries attrs A

#### Scenario: A repeated row's edges are not read
- **WHEN** a TOML register declares item `A` with `refs = "X"` and again with
  `refs = "Y"`, and the adapter emits each edge with its row's `declared_by`
- **THEN** the graph holds `A -> X` and not `A -> Y`, and the duplicate finding for `A`
  ends `its 1 edge was not read`

#### Scenario: An edge without declared_by is still read
- **WHEN** a document repeats node `A` and carries an edge from `A` with no `declared_by`
- **THEN** the graph holds that edge, and the duplicate finding for `A` ends `1 edge
  from 'A' has no declared_by and was read, though this entry may have declared it`

### Requirement: A node may say where each attr was written
An adapter that knows where an attr's value was written, when that is not the node's own
line, SHALL say so in the node's `attr_provenance`, so a finding about that value lands on
it rather than on the record holding it. The TOML adapter SHALL name each mapped attr's
key line; the entomologist adapter each attr's file in the issue directory, at line 1. The
core SHALL check the names against the node's attrs at ingest.

Verified by: `cargo test --test contract attr_provenance`, and
`.venv/bin/python -m pytest tests -k "where_it_was_written or lands_on_the_key or gate_run_exits_1"`

#### Scenario: A TOML attr is located at its key
- **WHEN** an `[[item]]` table at line 10 holds `stage = "s2"` at line 12
- **THEN** the node's provenance is line 10 and its `attr_provenance` names `stage` at
  line 12

#### Scenario: A location for an attr the node does not hold
- **WHEN** a node's `attr_provenance` names `b` and its `attrs` hold only `a`
- **THEN** the core refuses the document (exit 2) with `node 0 attr_provenance: names
  attr 'b', which the node's attrs do not hold`

### Requirement: Document keys are closed
Under every accepted interface version (`1.0`, `1.1`, `1.2`, `1.3`, `1.4`), each object in the
document SHALL carry only the keys listed here, and the core SHALL refuse a document
in which any object carries another key, naming every such key and the object it is
on (exit 2). A misspelt key read as an absent one would be a silent answer: `node`
for `nodes` reads as an empty register. The core SHALL check the version before the
keys, so a document of an unsupported version is refused for its version.

| Object | Keys |
|--------|------|
| document | `interface_version`, `contract_version`, `nodes`, `edges`, `pathways`, `findings`, `issues` |
| node | `id`, `kind`, `attrs`, `provenance`, `attr_provenance` |
| edge | `src`, `tgt`, `kind`, `attrs`, `provenance`, `declared_by` |
| pathway | `name`, `order`, `current` |
| finding | `severity`, `code`, `message`, `provenance`, `node_id`, `state` |
| provenance | `file`, `line` |

The contents of a node's or an edge's `attrs` are domain data and stay open.

`issues` is the pre-1.2 name of `findings`. The core SHALL validate both when present,
SHALL ingest whichever is non-empty, and SHALL refuse a document in which both are
non-empty (exit 2) rather than merge or choose between them.

Verified by: `cargo test --test contract closed_keys`, `cargo test --test contract alias`,
`cargo test --test cli closed_keys`, and `cargo test --test cli alias`

#### Scenario: Unknown top-level key
- **WHEN** an adapter emits `{"interface_version": "1.2", "node": []}`
- **THEN** the core exits 2 with `document: unknown key 'node'`

#### Scenario: Unknown key on a node
- **WHEN** a node carries `attr` in place of `attrs`
- **THEN** the core exits 2 with `node 0: unknown key 'attr'`

#### Scenario: Unknown key on a nested object, under every version
- **WHEN** a node, edge, pathway, finding or provenance carries a key outside its set,
  in a document declaring `interface_version` `1.0`, `1.1`, `1.2`, `1.3` or `1.4`, or legacy
  `contract_version` `1.2`
- **THEN** the core refuses the document naming the object and the key

#### Scenario: Attrs contents stay open
- **WHEN** a node's or an edge's `attrs` carries keys of any name
- **THEN** the core ingests them unchanged

#### Scenario: Populated issues beside empty findings
- **WHEN** a document carries `"findings": []` and a non-empty `issues`
- **THEN** the core ingests the `issues` entries, so an error among them gates the exit

#### Scenario: Populated findings beside empty issues
- **WHEN** a document carries `"issues": []` and a non-empty `findings`
- **THEN** the core ingests the `findings` entries

#### Scenario: Both aliases populated
- **WHEN** a document carries non-empty `findings` and non-empty `issues`
- **THEN** the core exits 2 naming both keys

### Requirement: Core invokes the adapter with profile and target paths
The core SHALL run the adapter program with a profile path and the target path, and
SHALL read the document from the program's stdout. The profile path SHALL name the
core's **resolved profile document** — not the user's profile file. The adapter SHALL
exit 0 when it has emitted a document, including when that document's only content is
issues.

Verified by: `cargo test --test cli adapter` and `.venv/bin/python -m pytest tests -k resolved`

#### Scenario: Adapter receives both paths
- **WHEN** the core runs an adapter program
- **THEN** the program receives a profile path and the target path and writes its
  document to stdout

#### Scenario: The profile path names the resolved document
- **WHEN** the core runs an adapter program with `--profile profiles/example.yaml`
- **THEN** the path the adapter receives resolves to a JSON resolved profile document,
  not to `profiles/example.yaml` itself

#### Scenario: A YAML-reading adapter survives the handoff
- **WHEN** an adapter parses the profile path it receives with a YAML library
- **THEN** it obtains the same profile data it would have read from the user's file,
  because canonical JSON is a YAML subset and resolution preserves every declared field

#### Scenario: A register the adapter could not read still exits 0
- **WHEN** an adapter cannot parse any of the register and emits a document of issues
  with no nodes
- **THEN** the adapter exits 0 and the core reports those issues, because a read failure
  is a finding while an adapter failure is exit 2

### Requirement: Resolved profile document
The resolved profile document SHALL be a canonical JSON object carrying a
`resolved_schema` version field and the entire profile as loaded and validated: every
node kind with its raw `id_pattern` source, `summary_attr`, and attribute schemas;
every edge kind with its allowed endpoint pairs; validations; pathways; `name`;
`profile_version`; and every section the profile declared that the core does not
itself consume, including the `adapter:` namespace, reproduced unchanged. The core
SHALL refuse to run the adapter if the resolved document cannot be produced, exiting 2.
Every adapter SHALL refuse a profile document whose `resolved_schema` is missing or
is not a version it reads, as a configuration error (exit 2), rather than reading a
profile's YAML source or a future schema as if it were the one it knows.

Verified by: `cargo test --test contract resolved`,
`cargo test -p latt-adapter load_resolved_profile`,
`cargo test -p latt-core --lib adapters::prose_xref only_a_resolved_profile_document_loads` and
`.venv/bin/python -m pytest tests -k resolved`

#### Scenario: Declared fields round-trip
- **WHEN** the core resolves a profile declaring node kinds, edge kinds, validations,
  and an `adapter:` section with `paths` and `cited_path_prefixes`
- **THEN** the resolved document carries every one of those fields with the values the
  profile declared, and `id_pattern` values are the raw pattern sources

#### Scenario: Resolved document declares its schema version
- **WHEN** the core produces a resolved profile document
- **THEN** the document carries a `resolved_schema` field identifying the schema
  version, distinct from `profile_version` and from the interface version

#### Scenario: Adapter refuses a document without its schema marker
- **WHEN** an adapter is handed a profile document with no `resolved_schema`, or one
  naming a schema it does not read
- **THEN** it fails as a configuration error naming the profile as not a resolved
  profile document

#### Scenario: Unresolvable profile is a broken setup
- **WHEN** the profile fails validation at load
- **THEN** the core exits 2 without running the adapter

#### Scenario: Adapter rejects an unparseable resolved document
- **WHEN** an adapter's profile reader is handed a resolved document that is not valid JSON
- **THEN** the reader raises a profile error naming the read failure rather than guessing at the profile

### Requirement: Pathway validity is the adapter's responsibility
An adapter SHALL validate an ordering pathway before emitting it: positions unique, and
`current` among them. An adapter that reads an invalid pathway SHALL emit an `PATHWAY_INVALID`
issue and omit the pathway, rather than emitting the pathway as read. A pathway in the document
that is structurally invalid SHALL be a schema failure.

Verified by: `cargo test --test contract pathway`

#### Scenario: Invalid pathway becomes a finding, not a pathway
- **WHEN** the target declares a `current` position that is not in the declared order
- **THEN** the adapter emits `PATHWAY_INVALID` naming the file and emits no pathway

#### Scenario: Invalid pathway in the document is refused
- **WHEN** a document carries a pathway whose `current` is not in its `order`
- **THEN** the core exits 2 rather than ingesting it

### Requirement: Interface document declares its version
The document SHALL carry an interface version identifying the interchange format. The core
SHALL refuse a document whose interface version it does not support, rather than ingesting
it partially. The core SHALL read `interface_version` whenever that key is present.
Only when it is absent SHALL the core accept the legacy `contract_version` key.
The chosen value SHALL be a supported string version. A malformed or unsupported
`interface_version` SHALL NOT fall back to `contract_version`; a legacy value is
ignored when `interface_version` is present.

Verified by: `cargo test --test contract` and `cargo test --test cli interface_version`

#### Scenario: Unsupported interface version is refused
- **WHEN** an adapter emits a document declaring an interface version the core does not
  support
- **THEN** the core exits 2 naming the version it received and the versions it supports

#### Scenario: Legacy version fallback
- **WHEN** a document omits interface_version and declares supported contract_version "1.2"
- **THEN** the core ingests it using that version

#### Scenario: Primary version takes precedence
- **WHEN** interface_version is present and malformed or unsupported, alongside supported contract_version
- **THEN** the core refuses the primary version rather than falling back

#### Scenario: Legacy value ignored beside a supported primary version
- **WHEN** interface_version is supported and contract_version is malformed
- **THEN** the primary version governs ingestion

### Requirement: Duplicate node IDs are resolved at ingest
The core SHALL detect duplicate node IDs when ingesting the document. The first
occurrence SHALL become the node; each subsequent occurrence of that ID SHALL become a
finding carrying both provenances, and SHALL NOT replace the node already ingested.

Verified by: `cargo test --test contract duplicate` and `cargo test --test contract occurrence`

#### Scenario: Second occurrence becomes a finding
- **WHEN** a document declares node `REQ-0701` twice with different provenance
- **THEN** the graph holds the first occurrence, and a duplicate finding names the ID and
  both provenances

#### Scenario: Duplicate across kinds
- **WHEN** a document declares `X-1` as kind `req` and again as kind `test`
- **THEN** the first is ingested and the second becomes a duplicate finding

### Requirement: Text that is not UTF-8 is reported where it is, as written
An adapter that reads register files as UTF-8 text SHALL report each line holding bytes
that are not UTF-8 as its own `PARSE_ERROR` at that line, naming the one-based column of
the first such byte and showing the line as written, each such byte as `\xNN` (uppercase
hex). It SHALL NOT read the line as content: a replacement character is not what the
register wrote. How much the line withholds follows the format:

- In a Markdown-shaped register (the md, prose-xref and OpenSpec spec readers), the line
  withholds only the entry that holds it — a table row, a heading or reference, a
  requirement's section — and every other line is read. A line that defines how others
  are read (a table's header or separator, a multi-table heading, a requirement heading)
  withholds what it defines, and a second `PARSE_ERROR` at that entry says so.
- In a format defined over UTF-8 (TOML, and the test source files the OpenSpec adapter
  reads), the file is not valid input, as for a syntax error: it is read as nothing, and
  each such line is reported as above.

The Rust adapters find the lines through `latt_adapter::decode_lines`; the Python
adapters through `adapters/_read.py`.

Verified by: `.venv/bin/python -m pytest tests -k "non_utf8 or not_utf8 or invalid_utf8 or undecodable"`,
and `cargo test -p latt-adapter --doc decode_lines`

#### Scenario: One row holds a byte that is not UTF-8
- **WHEN** a Markdown register's row reads `| REQ-2 | Pr\xE9serve source |`
- **THEN** that row is a `PARSE_ERROR` at its line showing it as written, and every other
  row of the file is read

#### Scenario: A TOML file holds a byte that is not UTF-8
- **WHEN** a TOML register's line 6 reads `status = "dr\xE4ft-3"`
- **THEN** one `PARSE_ERROR` at line 6 shows the line as written and says the file was
  not read, and the file contributes no node

### Requirement: A leading byte order mark is encoding, not content
An adapter that reads register files as UTF-8 text SHALL discard one leading UTF-8 byte
order mark (`EF BB BF`) before parsing, and SHALL produce for a file so prefixed the same
document it produces without the mark. Left in place, the mark joins the first heading or
table row, which then fails to match and is dropped with no finding. The Rust adapters
decode through `latt_adapter::decode_utf8`; the Python adapters read through
`adapters/_read.py`.

Verified by: `.venv/bin/python -m pytest tests -k byte_order`, and
`cargo test -p latt-adapter --doc decode_utf8`

#### Scenario: Requirement heading on the first line
- **WHEN** an OpenSpec `spec.md` starts with a byte order mark and then
  `### Requirement: First`
- **THEN** the adapter emits requirement `First` as it would without the mark

#### Scenario: Table on the first line
- **WHEN** a Markdown register file starts with a byte order mark and then a table header
- **THEN** the adapter reads the table's rows as it would without the mark

### Requirement: Adapter failure is a broken setup, not a finding
The core SHALL exit 2 when the adapter program cannot be run, exits non-zero, emits
stdout that is not UTF-8, emits output that cannot be parsed, or emits output that does
not satisfy the interface schema. Invalid UTF-8 SHALL NOT be replaced and ingested.
A native adapter that fails to produce a document SHALL also cause exit 2.
The core SHALL NOT ingest a partial document, and SHALL NOT report such a failure as a
register finding.

Verified by: `cargo test --test cli exit`, `cargo test --test dispatch`, and
`cargo test --test native native_configuration_errors`.

#### Scenario: Adapter program not executable
- **WHEN** `--adapter` names a path that cannot be executed
- **THEN** the core exits 2 with a message naming the program

#### Scenario: Adapter exits non-zero
- **WHEN** the adapter program exits 1 after writing part of a document
- **THEN** the core exits 2 and ingests nothing

#### Scenario: Adapter output is unparseable
- **WHEN** the adapter program exits 0 but writes output that is not a valid document
- **THEN** the core exits 2 naming the parse failure

#### Scenario: Adapter output is not UTF-8
- **WHEN** the adapter program exits 0 but writes a document holding the byte `0xE9`
  alone, as in `REQ-\xE9`
- **THEN** the core exits 2 with a message naming the adapter and saying its output is
  not UTF-8

#### Scenario: Adapter output fails the schema
- **WHEN** the adapter emits a well-formed document whose node entries lack provenance
- **THEN** the core exits 2 rather than ingesting nodes without provenance

#### Scenario: Native adapter internal failure
- **WHEN** a native adapter cannot read its profile or encounters an internal error
- **THEN** the core exits 2 with a diagnostic message

### Requirement: Schema failure messages use the native type vocabulary
When the core rejects an interface document as structurally invalid, the failure
message SHALL name the offending value's type in the native vocabulary
(`string`, `int`, `float`, `bool`, `list`, `object`, `null`), not in Python's
(`str`, `dict`, `NoneType`).

Verified by: `cargo test --test native_messages`

#### Scenario: Explicit null array is reported with a native type name
- **WHEN** an interface document carries `"nodes": null`
- **THEN** the schema failure message reads `'nodes' must be a list, got null`

### Requirement: Interface versions remain ingest-compatible
The issue severity vocabulary SHALL be `error`, `warning`, `info`, `hint`. The
vocabulary extension is interface version `1.1`. Interface version `1.2` renames the
top-level `issues` array to `findings` and adds optional edge attrs. Interface version
`1.3` adds the optional edge `declared_by`, and `1.4` the optional node
`attr_provenance`: emit helpers SHALL declare `1.4`, while the core SHALL accept
documents declaring `1.0`, `1.1`, `1.2`, `1.3` or `1.4`.
For compatibility, the core SHALL still read `issues`, as "Document keys are closed"
states. An
unknown severity string remains a schema failure (exit 2), never a fallback.

Verified by: `cargo test --test contract hint` and `.venv/bin/python -m pytest tests -k interface_version`

#### Scenario: Hint severity survives ingest
- **WHEN** a document declaring interface version `1.1` carries an issue at severity `hint`
- **THEN** the core ingests it and reports the issue at `hint`

#### Scenario: 1.0 documents still ingest
- **WHEN** a document declares interface version `1.0`
- **THEN** the core ingests it unchanged

#### Scenario: 1.2 findings and edge attrs ingest
- **WHEN** a document declares interface version `1.2` with `findings` and edge `attrs`
- **THEN** the core ingests both shapes without discarding their content

#### Scenario: Unknown severity is still a schema failure
- **WHEN** a document carries an issue at severity `suggestion`
- **THEN** the core exits 2 naming the unknown severity

### Requirement: Resolved profile scratch files are private and transient
The core SHALL exclusively create resolved-profile scratch files, retrying name
collisions without overwriting existing entries. On Unix it SHALL request mode
0600 at creation; umask MAY further restrict access. Native and external paths
SHALL use the same scratch-profile writer and close the writer before invocation.

The core SHALL attempt removal on normal return, error return, and unwinding,
including write failure. Cleanup is not guaranteed after abort, process kill,
or filesystem removal failure. No process-wide umask change is required.

Verified by: `cargo test --lib document::tests`, `cargo test --test dispatch external_scratch`,
and `cargo test --test native cleanup_scratch`.

#### Scenario: Private creation
- **WHEN** the core creates a resolved profile scratch file on Unix
- **THEN** its initial permissions grant no group or other access

#### Scenario: Existing scratch name
- **WHEN** exclusive creation encounters an existing scratch path
- **THEN** the core retries another name and preserves the existing contents

#### Scenario: Write or execution failure cleanup
- **WHEN** scratch writing or native/external execution returns an error
- **THEN** the scratch owner attempts removal as its scope ends

#### Scenario: Unwinding cleanup
- **WHEN** adapter work unwinds through a live scratch owner
- **THEN** the owner attempts removal

### Requirement: The adapter program's run is bounded
The core SHALL wait for an external adapter program no longer than the bound the
profile declares as `adapter.timeout_seconds`, defaulting to 300 seconds when the key
is absent. When the bound expires the core SHALL terminate the adapter process, SHALL
ingest nothing, and SHALL exit 2 with a message naming the program and the bound in
seconds. Expiry is an adapter failure in the sense of "Adapter failure is a broken
setup, not a finding": a program that has not answered produced no view of the
register.

Verified by: `cargo test --test cli adapter_exceeding_its_bound_exits_two` and
`cargo test -p latt-adapter process`.

#### Scenario: Adapter exceeds the declared bound
- **WHEN** the profile declares `adapter: { timeout_seconds: 1 }` and the adapter
  program sleeps longer than one second before writing anything
- **THEN** the core exits 2 with a message naming the program and `1` second, ingests
  nothing, and the adapter process no longer exists

#### Scenario: Adapter finishes inside the bound
- **WHEN** the profile declares `adapter: { timeout_seconds: 60 }` and the adapter
  program writes a valid document and exits 0 within that time
- **THEN** the core ingests the document exactly as it does with no bound declared

#### Scenario: Default bound applies when the key is absent
- **WHEN** the profile's `adapter:` block has no `timeout_seconds`
- **THEN** the core waits at most 300 seconds for the adapter program

### Requirement: Adapter-core document types admit additions
The interface-document types the `latt-adapter` crate exports (the document, nodes, edges,
pathways and findings) SHALL be non-exhaustive to code outside the crate, and SHALL be
built through constructors taking the fields the wire requires and builders for the
optional ones. Their fields SHALL stay readable. Node and edge kinds SHALL be owned
strings, so the document carries no lifetime. A finding's severity SHALL be an enum
that serializes to exactly the wire spellings `error`, `warning`, `info` and `hint`,
so an adapter cannot compile a severity ingest would refuse. The bounded-run error
SHALL be non-exhaustive. A provenance SHALL be non-exhaustive too, built through its
constructors and read through `file()` and `line()`; `line()` SHALL be `None` for the
wire's line 0, so no reader can take a whole-file location for line 0. These changes
SHALL NOT change the serialized document.

Verified by: `cargo test -p latt-adapter --locked`, the adapter gates
(`.venv/bin/python -m pytest tests -k gate`), and `./tools/verify-rust-tooling.sh`.
The semver-checks lints are listed under Breaking in the release that shipped them and
reconciled by
`tools/check-breaking.sh`.

#### Scenario: Built through constructors, serialized unchanged
- **WHEN** an adapter builds a node with attrs, an edge, and a parse error attributed to a node through the constructors and builders
- **THEN** the serialized document has the same keys and values as the struct literals produced before, including `interface_version` `1.4`, and an edge given no `declared_by` carries no such key

#### Scenario: Severity spellings
- **WHEN** each `Severity` variant is serialized
- **THEN** the values are `"error"`, `"warning"`, `"info"` and `"hint"`

#### Scenario: Adapter output unchanged
- **WHEN** the four Rust adapters (markdown, TOML, prose-xref and entomologist) run their gates after the migration
- **THEN** every gate passes against its existing fixture expectations

#### Scenario: Provenance is built through a constructor
- **WHEN** an adapter outside the crate builds a provenance for `REQUIREMENTS.md` line 47
- **THEN** it calls `Provenance::new`, a struct literal does not compile, and the serialized provenance is `{"file": "REQUIREMENTS.md", "line": 47}` as before

### Requirement: Core and adapter crate share one provenance line type
The core and the adapter crate SHALL hold a provenance line as the same unsigned 32-bit
type, so a document a Rust adapter can build is one ingest accepts. A built-in producer
(the markdown, TOML and prose-xref adapters, and `latt-adapter`'s source scanners) SHALL
check a file's line count before reading it: a file with more than 4294967295 lines
SHALL NOT be read, and SHALL be reported as one `PARSE_ERROR` at line 0 of that file
naming its line count. The refused file is the unread item. Inside a file that is read
every counted line fits, so no item is placed at a wrapped or clamped line.
`docs/adapters.md` and `docs/json.md` SHALL state the bound.

No fixture can hold four billion lines, so the check's tests inject the line count; its
use is covered by the adapter gates and by reading each producer's file-read path.

Verified by: `cargo test -p latt-core --locked --test provenance_line`, the checks' unit
tests (`cargo test -p latt-core --locked --lib line_fits`,
`cargo test -p latt-adapter --locked line_fits`), and the adapter gates.

#### Scenario: A file with more lines than fit is refused
- **WHEN** a built-in producer meets a file of 4294967296 lines
- **THEN** it reads nothing from that file and reports one `PARSE_ERROR` at line 0 of it naming 4294967296

#### Scenario: A file of the largest line count is read
- **WHEN** a built-in producer meets a file of 4294967295 lines
- **THEN** it reads the file, and no finding is added for its length

