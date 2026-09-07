# adapter-contract Specification

## Purpose
The interchange between a per-repo adapter and the core: an adapter program emits its
register as a serialized graph plus issues, and the core ingests it. This keeps ingest in
the host repo's own language while the core is free to be written in another.
## Requirements
### Requirement: Adapter is a program emitting a serialized graph
An adapter SHALL be an executable program. Given the profile and the target path, it
SHALL write to stdout a single document containing the nodes, edges, any ordering axis,
and the issues it collected while reading the register. The core SHALL ingest that
document and SHALL NOT import, link, or otherwise execute adapter code in its own
process.

The adapter's own reading errors travel as issues inside the document, preserving the
standing contract that unreadable input is reported and never dropped. Concretely: a
recognized candidate the adapter cannot read or represent SHALL produce an issue — a
failed target-file read (undecodable bytes, missing permission, a directory where a
file was expected) is register input failing, not the adapter failing, and SHALL NOT
become a non-zero exit. When the candidate's identity remains readable, the adapter
SHOULD preserve the node and omit only the unreadable contribution.

Verified by: `cargo test --test contract` and
`.venv/bin/python -m pytest -q tests/test_adapter_openspec.py -k Unreadable`

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

### Requirement: Document schema
The document SHALL be a JSON object carrying `contract_version`, `nodes`, `edges`, `axes`
and `issues`. A node SHALL carry `id`, `kind`, `attrs` and `provenance`; an edge `src`,
`tgt`, `kind` and `provenance`; an axis `name`, `order` and `current`; an issue
`severity`, `code`, `message`, `provenance` and `node_id`. A provenance SHALL carry
`file` and `line`.

A provenance's `file` SHALL name a path relative to the target root whenever the file
lies inside the target, and SHALL be left as the adapter wrote it otherwise, so that a
path outside the target and a placeholder such as `<profile>` still say what they mean.
An absolute path records where a checkout happens to sit, and two runs of one commit
under different paths would then disagree byte for byte — which defeats any comparison
of two runs, the trace baselines included.

A node's `attrs` SHALL be an arbitrary JSON object, nesting objects and arrays to any
depth. An adapter whose source format has types JSON does not carry SHALL encode them
before emitting; a value that is not JSON is a schema failure, not a finding.

Each issue SHALL carry the severity its adapter chose. The core SHALL NOT recompute an
adapter issue's severity from its own defaults, because an adapter may emit a code the
core has never heard of and recomputation would silently reduce it to a fallback. The
profile's severity overrides still apply after ingest.

Verified by: `cargo test --test contract refused`

#### Scenario: Nested attribute values survive ingest
- **WHEN** a document declares a node whose `attrs` contain a nested object and an array
  of objects
- **THEN** the graph's node carries those values unchanged

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

Order is part of the contract because both consequences are observable in output, and a
serializer free to reorder would make ingest non-deterministic.

Verified by: `cargo test --test contract order`

#### Scenario: Node order decides the surviving duplicate
- **WHEN** a document declares `REQ-1` with attrs A before declaring it with attrs B
- **THEN** the graph's `REQ-1` carries attrs A

### Requirement: Core invokes the adapter with profile and target paths
The core SHALL run the adapter program with a profile path and the target path, and
SHALL read the document from the program's stdout. The profile path SHALL name the
core's **resolved profile document** — not the user's profile file. The adapter SHALL
exit 0 when it has emitted a document, including when that document's only content is
issues.

The profile still travels as a path so that the interface stays a file the adapter
reads with its own library, in whatever language it is written. The content behind the
path changes: it is core-resolved canonical JSON, which is a YAML subset, so an adapter
reading it with a YAML library continues to work unchanged during migration.

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
every edge kind with its allowed endpoint pairs; validations; axes; `name`;
`profile_version`; and every section the profile declared that the core does not
itself consume, including the `adapter:` namespace, reproduced unchanged. The core
SHALL refuse to run the adapter if the resolved document cannot be produced, exiting 2.

Round-tripping sections the core does not model is normative because adapters own
their `adapter:` configuration; a resolved document that dropped it would force
adapters back to reading the user's file, which is the duplication this handoff
removes.

Verified by: `cargo test --test contract resolved` and
`.venv/bin/python -m pytest tests -k resolved`

#### Scenario: Declared fields round-trip
- **WHEN** the core resolves a profile declaring node kinds, edge kinds, validations,
  and an `adapter:` section with `paths` and `cited_path_prefixes`
- **THEN** the resolved document carries every one of those fields with the values the
  profile declared, and `id_pattern` values are the raw pattern sources

#### Scenario: Resolved document declares its schema version
- **WHEN** the core produces a resolved profile document
- **THEN** the document carries a `resolved_schema` field identifying the schema
  version, distinct from `profile_version` and from the contract version

#### Scenario: Unresolvable profile is a broken setup
- **WHEN** the profile fails validation at load
- **THEN** the core exits 2 without running the adapter, as today

#### Scenario: Adapter rejects an unparseable resolved document
- **WHEN** an adapter's profile reader is handed a resolved document that is not valid JSON
- **THEN** the reader raises a profile error naming the read failure rather than guessing at the profile

### Requirement: Axis validity is the adapter's responsibility
An adapter SHALL validate an ordering axis before emitting it: positions unique, and
`current` among them. An adapter that reads an invalid axis SHALL emit an `AXIS_INVALID`
issue and omit the axis, rather than emitting the axis as read. An axis in the document
that is structurally invalid SHALL be a schema failure.

Axis validity stays with the adapter, where duplicate detection moved to the core,
because the adapter is what read the target's declaration and can name the file it came
from. An invalid axis places every position both before and after `current`, so ingesting
one would silently corrupt every finding bound to it.

Verified by: `cargo test --test contract axis`

#### Scenario: Invalid axis becomes a finding, not an axis
- **WHEN** the target declares a `current` position that is not in the declared order
- **THEN** the adapter emits `AXIS_INVALID` naming the file and emits no axis

#### Scenario: Invalid axis in the document is refused
- **WHEN** a document carries an axis whose `current` is not in its `order`
- **THEN** the core exits 2 rather than ingesting it

### Requirement: Contract document declares its version
The document SHALL carry a contract version identifying the interchange format. The core
SHALL refuse a document whose contract version it does not support, rather than ingesting
it partially.

The contract version is distinct from `profile_version`, which governs the register
schema, and from the lattice version, which governs the trace payload envelope.

Verified by: `cargo test --test contract version`

#### Scenario: Unsupported contract version is refused
- **WHEN** an adapter emits a document declaring a contract version the core does not
  support
- **THEN** the core exits 2 naming the version it received and the versions it supports

### Requirement: Duplicate node IDs are resolved at ingest
The core SHALL detect duplicate node IDs when ingesting the document. The first
occurrence SHALL become the node; each subsequent occurrence of that ID SHALL become a
finding carrying both provenances, and SHALL NOT replace the node already ingested.

Detection belongs to the core so that every adapter gets it without implementing it, and
so no adapter can drop a duplicate in silence. Ordering is specified because which
occurrence wins determines the resulting node's attrs and provenance, and node IDs are a
public surface.

Verified by: `cargo test --test contract duplicate` and `cargo test --test contract occurrence`

#### Scenario: Second occurrence becomes a finding
- **WHEN** a document declares node `REQ-0701` twice with different provenance
- **THEN** the graph holds the first occurrence, and a duplicate finding names the ID and
  both provenances

#### Scenario: Duplicate across kinds
- **WHEN** a document declares `X-1` as kind `req` and again as kind `test`
- **THEN** the first is ingested and the second becomes a duplicate finding

### Requirement: Adapter failure is a broken setup, not a finding
The core SHALL exit 2 when the adapter program cannot be run, exits non-zero, emits
output that cannot be parsed, or emits output that does not satisfy the contract schema.
It SHALL NOT ingest a partial document, and SHALL NOT report such a failure as a
register finding.

An adapter that failed produced no trustworthy view of the register, so reporting its
absence as "no findings" would be wrong rather than merely incomplete.

Verified by: `cargo test --test cli exit`

#### Scenario: Adapter program not executable
- **WHEN** `--adapter` names a path that cannot be executed
- **THEN** the core exits 2 with a message naming the program

#### Scenario: Adapter exits non-zero
- **WHEN** the adapter program exits 1 after writing part of a document
- **THEN** the core exits 2 and ingests nothing

#### Scenario: Adapter output is unparseable
- **WHEN** the adapter program exits 0 but writes output that is not a valid document
- **THEN** the core exits 2 naming the parse failure

#### Scenario: Adapter output fails the schema
- **WHEN** the adapter emits a well-formed document whose node entries lack provenance
- **THEN** the core exits 2 rather than ingesting nodes without provenance

### Requirement: Schema failure messages use the native type vocabulary
When the core rejects an adapter document as structurally invalid, the failure
message SHALL name the offending value's type in the native vocabulary
(`string`, `int`, `float`, `bool`, `list`, `object`, `null`), not in Python's
(`str`, `dict`, `NoneType`).

Verified by: `cargo test --test native_messages`

#### Scenario: Explicit null array is reported with a native type name
- **WHEN** an adapter document carries `"nodes": null`
- **THEN** the schema failure message reads `'nodes' must be a list, got null`

### Requirement: Contract 1.1 extends the severity vocabulary with hint
The issue severity vocabulary SHALL be `error`, `warning`, `info`, `hint`. The
vocabulary extension is contract version `1.1`: the emit helper SHALL declare `1.1`,
and the core SHALL accept documents declaring `1.0` or `1.1`. A `1.0` document by
construction never carries `hint`; the core applies one parser to both versions, since
`1.1` is a strict superset. An unknown severity string remains a schema failure (exit
2), never a fallback.

Verified by: `cargo test --test contract hint` and `.venv/bin/python -m pytest tests -k contract_version`

#### Scenario: Hint severity survives ingest
- **WHEN** a document declaring contract version `1.1` carries an issue at severity `hint`
- **THEN** the core ingests it and reports the issue at `hint`

#### Scenario: 1.0 documents still ingest
- **WHEN** a document declares contract version `1.0`
- **THEN** the core ingests it unchanged

#### Scenario: Unknown severity is still a schema failure
- **WHEN** a document carries an issue at severity `suggestion`
- **THEN** the core exits 2 naming the unknown severity
