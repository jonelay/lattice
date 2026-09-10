# graph-model Specification

## Purpose
The in-memory graph that adapters build and core validates — nodes with typed attrs,
directed edges with kind constraints, and source provenance on every element.
## Requirements
### Requirement: Add node with provenance
`graph.add_node(id, kind, attrs, provenance)` SHALL add a node to the graph. `provenance`
is a required argument containing at minimum `file` (string path) and `line` (int).

#### Scenario: Add a valid node
- **WHEN** an adapter calls `add_node("REQ-0701", "req", {"text": "..."}, Provenance("REQUIREMENTS.md", 47))`
- **THEN** the graph contains a node with id "REQ-0701", kind "req", the given attrs, and provenance

#### Scenario: Provenance is required
- **WHEN** an adapter calls `add_node` without provenance
- **THEN** a TypeError is raised

### Requirement: Add edge with attrs and provenance
`graph.add_edge(src, tgt, kind, attrs, provenance)` SHALL add a directed edge. `attrs`
is an arbitrary JSON object and defaults empty when the contract omits it. `provenance` is
required.

#### Scenario: Add a valid edge
- **WHEN** an adapter calls `add_edge("test_fem", "REQ-0704", "verifies", {"confidence": 1}, Provenance("tests/test_fem.py", 9))`
- **THEN** the graph contains a directed edge from "test_fem" to "REQ-0704" of kind "verifies" with that attr

### Requirement: Duplicate node semantics
Each node ID SHALL be unique across all kinds. When a register declares an ID more than
once, the first declaration SHALL become the node and each subsequent declaration SHALL
become a finding carrying the ID and both provenances.

Duplicate resolution is the core's responsibility at ingest, so every adapter gets the
behaviour without implementing it and none can discard a duplicate in silence. See the
`adapter-contract` capability for the ingest-side contract.

`LatticeGraph::add_node` SHALL still fail on a repeated ID, returning a `DuplicateNode`
carrying the ID and both provenances. That refusal is the mechanism ingest detects
duplicates with, not a contract on adapters — adapters build into a duplicate-tolerant
builder and never call it, because a builder that rejected the second occurrence would
hide it from the core that resolves it.

Verified by: `cargo test --test graph_model duplicate` for the refusal, and
`cargo test --test contract duplicate` for the register semantics

#### Scenario: Duplicate node ID same kind
- **WHEN** a register declares `REQ-0701` as kind `req` twice
- **THEN** the graph holds the first declaration and a duplicate finding names the ID and
  both provenances

#### Scenario: Duplicate node ID different kind
- **WHEN** a register declares `X-1` as kind `req` and again as kind `test`
- **THEN** the graph holds the `req` node and a duplicate finding names the ID and both
  provenances

#### Scenario: The surviving node is the first declared
- **WHEN** a register declares `REQ-0701` with attrs A, then again with attrs B
- **THEN** the graph's `REQ-0701` carries attrs A and the provenance of the first
  declaration

### Requirement: Dangling edge targets
The graph SHALL accept edges referencing node IDs that have not yet been added.
Validation (not the graph builder) SHALL report dangling references. This allows adapters
to add edges before their targets exist, supporting any parse order.

#### Scenario: Edge added before target node
- **WHEN** `add_edge("test_x", "REQ-0701", "verifies", {}, ...)` is called before "REQ-0701" is added
- **THEN** the edge is stored; dangling-ref validation later reports it if the target is never added

### Requirement: Provenance on findings
Every validation finding that concerns a graph element SHALL carry that element's
provenance (file, line), so error messages reference source locations. A finding that
originates in the profile rather than the register — a configuration error — SHALL carry
a synthetic provenance identifying the profile, since no source line produced it.

#### Scenario: Orphan finding includes provenance
- **WHEN** validation finds an orphan node "REQ-9999" with provenance ("REQUIREMENTS.md", 102)
- **THEN** the finding's provenance is ("REQUIREMENTS.md", 102)

#### Scenario: Config finding is not attributed to a source line
- **WHEN** validation emits a `CONFIG_ERROR` for a malformed `COVERAGE` config
- **THEN** the finding's provenance names the profile rather than a register file and line

### Requirement: Reads return copies
`node_data`, `iter_nodes`, and `iter_edges` SHALL return copies of node and edge data,
with node attrs deep-copied. A caller mutating what it reads SHALL NOT change the graph.

Validators and queries receive graph data directly; returning live references would let a
read-only pass corrupt the register it is checking.

#### Scenario: Mutating a read does not affect the graph
- **WHEN** a caller reads a node via `node_data` and mutates the returned `attrs` map
- **THEN** a subsequent read of that node returns the original attrs

### Requirement: Parallel edges are distinct
The graph SHALL be a directed multigraph: two `add_edge` calls with the same source,
target, kind, and attrs SHALL produce two distinct edges, each retaining its own provenance.

Two rows can legitimately assert the same relationship from different source lines, and
collapsing them would discard one of the two provenances.

#### Scenario: Repeated edge keeps both provenances
- **WHEN** `add_edge("A", "B", "derives", {}, p1)` and `add_edge("A", "B", "derives", {}, p2)` are both called
- **THEN** iterating edges yields two `derives` edges from A to B

#### Scenario: One pair carries several edge kinds
- **WHEN** `add_edge("A", "B", "derives", {}, p1)` and `add_edge("A", "B", "verifies", {}, p2)` are both called
- **THEN** iterating edges yields both edges, neither replacing the other

### Requirement: Pathway storage on the graph
The graph SHALL accept an ordering pathway: a name, an ordered sequence of position values,
and the current position. It SHALL expose them for read, and SHALL report no pathway when
none was set.

The graph carries these because they are *ingested* target data, on the same footing as
nodes and edges — the register declares them and the adapter reads them. The graph SHALL
NOT compute, default, or infer a pathway: a target that declares none has none.

The graph SHALL reject a pathway whose positions are not unique, or whose current position
is not among them, by raising. Callers convert that to a finding under their own contract.
Repeated positions would make "before" and "after" depend on which occurrence was matched,
so there is no correct resolution to fall back on.

Verified by: `cargo test --test graph_model pathway` for setting and reading, and
`cargo test --test contract pathway` for the rejections

#### Scenario: Pathway set and read
- **WHEN** an adapter sets pathway `phase` with order `[R0, CB, M0, M1]` and current `M0`
- **THEN** reading pathway `phase` yields that order and that current position

#### Scenario: No pathway set
- **WHEN** no call sets a pathway
- **THEN** reading any pathway name yields nothing, and no error is raised

#### Scenario: Current position must be a member
- **WHEN** a pathway is set whose current position is not in its order
- **THEN** the graph raises

#### Scenario: Positions must be unique
- **WHEN** a pathway is set with order `[M0, M1, M0]`
- **THEN** the graph raises

### Requirement: A pathway read cannot mutate the graph
A caller reading a pathway SHALL NOT be able to change the graph through what it reads.

The severity-resolution pass reads the pathway while inspecting findings, and a read-only
pass must not be able to corrupt the register it is checking.

Verified by inspection: `LatticeGraph::pathway` returns `Option<&Pathway>`, a shared borrow, so
the mutation this forbids does not compile. The requirement was previously discharged by
a copy-on-read test, which the borrow makes unwritable rather than unnecessary — a future
implementation that returns an owned value SHALL restore the test.

#### Scenario: A read pathway is not a mutable handle
- **WHEN** a caller reads pathway `phase`
- **THEN** it receives a value it cannot use to change the graph's copy
