# trace-report Specification

## Purpose
The trace-report payload — one entry per node carrying its kind, attrs, edges, provenance,
and attached findings — that feeds RTM deliverables, coverage dashboards, and CI reporting.
## Requirements
### Requirement: Trace entry structure
A trace entry SHALL carry: node ID, kind, profile-declared attrs, provenance (file + line),
outgoing edge targets grouped by edge kind, and a list of findings attached to this node
(matched by `node_id`). Edge targets are outgoing only.

#### Scenario: Entry for a req node with edges and findings
- **WHEN** the graph contains node `REQ-0101` (kind `req`) with a `derives` edge to `UN-1`
  and a `COVERAGE` finding attached
- **THEN** the trace entry for `REQ-0101` includes kind `req`, its attrs, its provenance,
  `derives: ["UN-1"]` in edges, and the `COVERAGE` finding in its findings list

#### Scenario: Entry for a node with no findings
- **WHEN** node `UN-1` has no validation findings
- **THEN** the trace entry for `UN-1` has an empty findings list

#### Scenario: Entry for a node with no outgoing edges
- **WHEN** node `BN-1` has no outgoing edges
- **THEN** the trace entry for `BN-1` has an empty edges map

### Requirement: Trace report structure
The trace report payload SHALL contain: a header with profile name, `profile_version`,
and lattice version; an ordered list of trace entries (one per node); and a list of
unattachable findings (issues whose `node_id` is null or names no node in the
graph); and an array of ordering pathways from the graph. Every validation issue SHALL
appear in exactly one of the two places — a finding that affects the exit code but
appears nowhere in the report would be silently dropped output.

#### Scenario: Header fields present in JSON
- **WHEN** `latt trace --format=json` runs
- **THEN** the JSON output includes `profile`, `profile_version`, `trace_version`, and
  `lattice_version` in its header

#### Scenario: Unattachable findings in report
- **WHEN** validation produces an `UNKNOWN_KIND` issue for an edge, which carries
  `node_id=None`
- **THEN** the issue appears in the report's unattachable findings list, not in any
  trace entry

#### Scenario: Finding attributed to a ghost node
- **WHEN** validation produces a `VACANCY` issue whose `node_id` is an edge
  source that was never added as a node
- **THEN** the issue appears in the unattachable findings list, and no trace entry
  exists for that ID

#### Scenario: Pathways are preserved in JSON
- **WHEN** the graph contains pathway `stage` with an order and current position
- **THEN** trace JSON includes `{"name":"stage","order":[...],"current":"..."}` in
  its `pathways` array

#### Scenario: No pathways are present
- **WHEN** the graph contains no pathways
- **THEN** trace JSON includes `"pathways": []`

### Requirement: Trace entry ordering
Trace entries SHALL be ordered by kind (in profile declaration order), then by node ID
(lexicographic). Within an entry, edge kinds SHALL be ordered lexicographically, and the
targets within each edge kind SHALL be ordered lexicographically by target ID, then by the
referencing edge's provenance file and line. Repeated targets are preserved, not
deduplicated. This order is deterministic for identical inputs and SHALL NOT depend on the
order in which the graph was built or on any graph library's iteration order.

Edge-kind order is carried by the same mechanism as every other payload mapping: JSON
object keys are emitted in sorted order, and a reimplementation SHALL do the same. Plain
and rich render an edge count rather than the kinds, so the ordering is observable only in
JSON.

Verified by: `cargo test --test output order`

#### Scenario: Stable ordering across runs
- **WHEN** `latt trace --format=json` runs twice on the same input
- **THEN** both outputs are byte-identical

#### Scenario: Kind groups then ID within kind
- **WHEN** the graph has nodes `REQ-0201` (kind `req`) and `BN-1` (kind `need`) and the
  profile declares `need` before `req`
- **THEN** `BN-1` appears before `REQ-0201` in the trace

#### Scenario: Edge targets ordered within a kind
- **WHEN** a node has `derives` edges to `UN-3`, `UN-1` and `UN-2`
- **THEN** the entry lists them as `UN-1`, `UN-2`, `UN-3`

#### Scenario: Edge kinds ordered lexicographically, not by declaration
- **WHEN** a node carries both `verifies` and `derives` edges and the profile declares
  `verifies` before `derives`
- **THEN** `derives` still precedes `verifies` in the entry's edge map

#### Scenario: Ordering is independent of build order
- **WHEN** two registers declare the same nodes and edges in different source order
- **THEN** both produce byte-identical trace output

### Requirement: Exit codes for trace
`latt trace` SHALL use the same 0/1/2 exit-code contract as `validate`: 0 when the
register has no error-severity findings, 1 when error-severity findings exist, 2 when
lattice could not run. `--strict` promotes warnings to errors before the exit-code
decision.

#### Scenario: Trace with errors exits 1
- **WHEN** trace runs and the graph has a `VACANCY` error
- **THEN** exit code is 1

#### Scenario: Trace clean exits 0
- **WHEN** trace runs and the graph has no error-severity findings
- **THEN** exit code is 0

#### Scenario: Strict promotes warnings
- **WHEN** trace runs with `--strict` and only warning-severity findings exist
- **THEN** exit code is 1

### Requirement: Trace JSON stability
The trace-JSON payload has two version axes in its header, each governing a
distinct surface, and one provenance field:

- `profile_version` governs the register schema: node kinds, edge kinds, ID
  patterns, and which attrs each kind carries. A consumer shim refusing a
  register it does not understand SHALL key on `profile_version`.
- `trace_version` governs the payload envelope: the set of top-level keys,
  the shape of a trace entry, and the shape of a finding object. It is an
  integer string (current value: the versioning capability's wire
  version axes). A downstream tool that parses the JSON structure
  SHALL key on `trace_version`; an optional key added to an entry or finding does
  not change it.
- `lattice_version` is provenance only: the version of the core that wrote the
  payload. No contract keys on it, and it differs between builds whose payloads
  are otherwise identical.

The following fields are public surfaces — changing their names or types is a
versioned change carrying a changelog line:
- Header: `profile`, `profile_version`, `trace_version`, `lattice_version`.
- Entry: `id`, `kind`, `attrs`, `edges`, `provenance`, `findings`.
- Pathway: `name`, `order`, `current`.
- Top-level: `header`, `entries`, `unattachable_findings`, `pathways`.

The ordering contract is a public surface, and covers both entry order (kind-order then
lexicographic ID) and within-entry edge order (lexicographic kind, then lexicographic
target). Consumers MAY depend on byte-identical output for identical inputs, across
implementations of the core and not merely across runs of one.

Verified by: `cargo test --test output byte_stable` for byte-identity across runs and
`cargo test --test output ordering_is_independent_of_build_order` for byte-identity
across build order. The enumerated public fields are a documentation contract read off
this spec, verified by inspection.

#### Scenario: Header documents both version axes
- **WHEN** the trace-report spec is read
- **THEN** it states that `profile_version` governs register schema,
  `trace_version` governs payload envelope, and `lattice_version` is provenance

#### Scenario: Public fields enumerated
- **WHEN** the trace-report spec is read
- **THEN** it names the entry-level fields and header fields that are public
  surfaces

#### Scenario: Ordering holds across implementations
- **WHEN** two independent implementations of the core read the same register with the
  same profile
- **THEN** their trace output is byte-identical apart from the `lattice_version` header
  field

### Requirement: Trace findings carry their note
A finding in a trace entry or in `unattachable_findings` that carries a note SHALL
expose it as the `note` object the output capability defines for findings JSON, and
SHALL omit the key otherwise. `trace_version` SHALL NOT change: the key is optional,
and a trace from a profile without notes is byte-identical to before it existed.

Verified by: `cargo test -p latt-core --locked --test output trace_note`, and
`cargo test -p latt-core --locked --test trace_baseline` with the stored baseline
unchanged.

#### Scenario: Noted finding in a trace entry
- **WHEN** `trace --format json` runs and `UN-3`'s `VACANCY` finding carries a note
- **THEN** that finding in `UN-3`'s entry carries the `note` object

#### Scenario: Trace without notes unchanged
- **WHEN** `trace --format json` runs over the synthetic fixture, whose profile declares
  no `annotations`
- **THEN** the output is byte-identical to the stored baseline
