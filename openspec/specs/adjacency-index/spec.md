# adjacency-index Specification

## Purpose
Borrowed adjacency index over a loaded graph, providing per-node
edge lookup with deterministic order and edge-kind filtering. Its edge readers
(`edges_from`, `edges_to`, `has_incoming`, `has_outgoing`, `connected`) are
`#[doc(hidden)]`: internal to the `latt` binary, not supported embedding API.
## Requirements
### Requirement: Borrowed adjacency index built from graph and direction
The library SHALL expose an `AdjacencyIndex` that borrows a
`LatticeGraph` and indexes edges in both directions (outgoing and incoming).
The index SHALL be constructed outside `LatticeGraph` — the graph has no
finalize step, and the index is scoped to one session, not cached. The index and all
its methods SHALL be unstable (`#[doc(hidden)]`): shared by the core and the `latt`
binary, not supported embedding API. A host reaches the same answers through the
service's reach, path and orphans requests.

Verified by: `cargo test -p latt-core --locked --lib query::index`, and the signature walk the
graph-service "service-first" requirement names.

#### Scenario: Build an adjacency index from a loaded graph
- **WHEN** a `LatticeGraph` with nodes and edges is loaded
- **THEN** `AdjacencyIndex::build(&graph)` succeeds and indexes all edges in both directions

#### Scenario: The index is absent from the rustdoc
- **WHEN** the `latt-core` rustdoc is generated
- **THEN** `AdjacencyIndex` has no page of its own in it, and the `latt` binary still builds and uses it

### Requirement: Per-node edge lookup returns edge references in deterministic order
`edges_from(id)` SHALL return all edges whose `src` matches `id`, as `&[&Edge]`.
`edges_to(id)` SHALL return all edges whose `tgt` matches `id`, as `&[&Edge]`.
Both SHALL return edges sorted by (far endpoint, edge kind), with ties broken
by ingestion order. An unknown ID SHALL return an empty slice.

#### Scenario: Forward edges from a node
- **WHEN** node A has outgoing edges to B (kind "verifies") and C (kind "derives")
- **THEN** `edges_from("A")` returns both edges sorted by (far endpoint, edge kind)

#### Scenario: Incoming edges to a node
- **WHEN** node B has incoming edges from A (kind "verifies") and D (kind "fulfills")
- **THEN** `edges_to("B")` returns both edges sorted by (far endpoint, edge kind)

#### Scenario: Unknown node ID returns empty
- **WHEN** `edges_from("nonexistent")` is called
- **THEN** the result is an empty slice

### Requirement: Index covers both declared and undeclared endpoints
The index SHALL include edges whose `src` or `tgt` names an ID not present
as a declared node. An undeclared endpoint is a valid index key — it may
have edges but no `Node` in the graph.

#### Scenario: Edges to an undeclared endpoint are indexed
- **WHEN** edge (A → X, kind "verifies") exists and X is not a declared node
- **THEN** `edges_from("A")` includes the edge, and `edges_to("X")` includes the edge

### Requirement: Edge-kind filtering
The index SHALL support constructing a filtered view that includes only edges
of specified kinds. The filtered view SHALL expose the same `edges_from`/`edges_to`
API. Filtering SHALL not change the relative order of retained edges.

#### Scenario: Filter to a single edge kind
- **WHEN** node A has edges of kinds "verifies" and "derives"
- **AND** a filtered view is built for kind "verifies" only
- **THEN** `edges_from("A")` on the filtered view returns only "verifies" edges

### Requirement: Parallel edges are preserved without deduplication
When multiple edges share the same `(src, tgt, kind)`, all occurrences
SHALL appear in the index. Each carries distinct attrs and provenance.

#### Scenario: Two parallel edges between same endpoints
- **WHEN** two edges (A → B, kind "verifies") exist with different provenance
- **THEN** `edges_from("A")` returns both edges in ingestion order

### Requirement: Subsumes EdgeIndex
`EdgeIndex` SHALL be removed from the public API. Its boolean queries
(`has_incoming`, `has_outgoing`) SHALL be derivable from the adjacency
index (non-empty `edges_to`/`edges_from` respectively).

#### Scenario: EdgeIndex no longer exists
- **WHEN** a consumer references `graph::EdgeIndex`
- **THEN** compilation fails — the type is removed

Verified by: `cargo test -p latt-core --locked` (unit tests on index
construction, ordering, filtering, parallel edges, undeclared endpoints).

