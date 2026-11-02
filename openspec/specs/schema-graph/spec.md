# schema-graph Specification

## Purpose
Snapshot of a profile's type structure exposing node kinds, and
allowed edge-kind triples as edges, joinable with instance counts from a
loaded graph. These items are `#[doc(hidden)]`: internal to the `latt` binary, not supported
embedding API.
## Requirements
### Requirement: Schema graph extracts the profile's type structure
The library SHALL expose a `SchemaGraph` extracted from a `Profile` that
presents:
- Node kinds as the graph's nodes (every kind declared in the profile)
- Allowed triples `(edge_kind, source_kind, target_kind)` as edges (from
  each `EdgeKind.allowed` list)

#### Scenario: Profile with two node kinds and one edge kind
- **WHEN** the profile declares node kinds "requirement" and "test", and edge
  kind "verifies" allowing (test, requirement)
- **THEN** the schema graph has 2 kinds and 1 allowed triple

#### Scenario: All profile kinds appear even at zero instances
- **WHEN** the profile declares kind "risk" but the graph has no risk nodes
- **THEN** the schema graph still includes "risk" as a kind

### Requirement: Instance counts joinable from adjacency index
The schema graph SHALL provide a method to join instance counts from a
loaded graph, reporting per node kind: how many nodes of that kind exist
in the instance. Per allowed triple: how many edges of that kind connect
the specified source and target kinds.

#### Scenario: Instance counts for a populated graph
- **WHEN** the graph has 5 requirement nodes, 8 test nodes, and 12 "verifies" edges
  from test to requirement
- **THEN** the joined schema graph reports kind counts (requirement: 5, test: 8)
  and triple count (verifies, test, requirement): 12

#### Scenario: Zero-count triples are reported
- **WHEN** the profile allows (test, requirement) via "verifies" but no such edges exist
- **THEN** the joined count for that triple is 0

### Requirement: Deterministic ordering
Kinds SHALL appear in profile declaration order (the `declared_index`
on `NodeKind`). Allowed triples SHALL be sorted by (edge kind, source kind,
target kind).

#### Scenario: Kind order matches profile
- **WHEN** the profile declares kinds in order: requirement, spec-goal, test
- **THEN** the schema graph's kinds appear in that order

Verified by: `cargo test -p latt-core --locked` (unit tests on schema
extraction, instance-count joining, zero-count cases, ordering).

