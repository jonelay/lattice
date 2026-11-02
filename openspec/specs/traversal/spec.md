# traversal Specification

## Purpose
Depth-first traversal report from a root node, yielding topology-preserving
steps with orthogonal endpoint, visit, parallel-edge, and taint fields. These items are `#[doc(hidden)]`: internal to the `latt` binary, not supported
embedding API.

## Requirements
### Requirement: Traversal report from a root node
The library SHALL expose a traversal function that walks the graph from a
given root node using the adjacency index, producing a `TraversalReport`
containing an ordered sequence of `TraversalStep` records.

#### Scenario: Traversal from a root with one edge
- **WHEN** a traversal is started from node A, which has one outgoing edge to B
- **THEN** the report contains two steps: A (root) and B

### Requirement: TraversalStep carries orthogonal fields
Each `TraversalStep` SHALL carry:
- `depth: usize` — distance from root (root is 0)
- `edge: Option<&Edge>` — the edge traversed to reach this step (None for root)
- `node_id: &str` — the endpoint node ID
- `endpoint: Endpoint` — `Declared` when the node exists in the graph, `Vacancy` otherwise
- `visit: Visit` — `First` on first encounter, `Cycle` when the node is an ancestor
  on the current DFS path, `Reconvergence` when visited via a different path
- `parallel_ordinal: usize` and `parallel_count: usize` — position among parallel
  edges between the same endpoints (0-indexed ordinal)
- `tainted: bool` — true when every path from root crosses a vacancy

#### Scenario: Root step
- **WHEN** traversal starts from node A
- **THEN** the first step has depth 0, edge None, node_id "A", visit First, and endpoint Declared

#### Scenario: Vacancy endpoint
- **WHEN** node A has an edge to X, and X is not a declared node
- **THEN** the step for X has endpoint Vacancy

#### Scenario: Cycle detection
- **WHEN** the graph has A → B → C → A
- **THEN** the revisit of A from C has visit Cycle

#### Scenario: Reconvergence detection
- **WHEN** A → B → D and A → C → D (diamond)
- **THEN** D's second encounter has visit Reconvergence

#### Scenario: Parallel edges
- **WHEN** two edges exist from A to B (different kinds)
- **THEN** both appear as steps with parallel_ordinal 0 and 1, parallel_count 2

### Requirement: Taint computed eagerly from a declared-only closure
Taint SHALL mean: every path from the root to this node crosses at least one
vacancy (undeclared endpoint). It SHALL be computed eagerly before the
traversal begins, using a BFS closure that admits only declared endpoints.
A node the DFS visits but the declared-only closure does not reach is
tainted. Taint SHALL NOT be derived lazily from visible rows.

#### Scenario: Node reachable only through a vacancy is tainted
- **WHEN** A → X → B, where X is undeclared
- **THEN** B's step has tainted true

#### Scenario: Node reachable via a declared path is not tainted
- **WHEN** A → B (both declared) and A → X → B (X undeclared)
- **THEN** B's step has tainted false (at least one clean path exists)

### Requirement: Direction support
The traversal SHALL accept a `Direction` parameter. Forward traversal
follows outgoing edges; reverse traversal follows incoming edges.

#### Scenario: Reverse traversal
- **WHEN** traversal is started in reverse from node B, which has an incoming edge from A
- **THEN** the report contains steps for B (root) and A

### Requirement: Edge-kind filtering via pre-filtered index
The traversal SHALL accept a pre-filtered `AdjacencyIndex`. The caller
controls edge-kind scope by filtering the index before passing it, rather
than passing a set of edge kinds to the traversal itself.

#### Scenario: Filtered traversal skips non-matching edges
- **WHEN** A has edges to B (kind "verifies") and C (kind "derives")
- **AND** the index is filtered to kind "verifies" before traversal
- **THEN** the report contains A and B but not C

### Requirement: Reconvergence stops expansion
A node visited via `Reconvergence` SHALL appear as a leaf in the traversal.
It SHALL NOT re-expand its children. Only `Cycle` and `Reconvergence` stop
expansion; `First` always expands.

#### Scenario: Reconverged node does not re-expand
- **WHEN** A → B → D and A → C → D → E
- **THEN** D appears twice (First under B, Reconvergence under C), and E appears only once under D's first expansion

Verified by: `cargo test -p latt-core --locked` (unit tests covering
root, vacancy, cycle vs reconvergence, parallel edges, taint, direction,
edge-kind filtering, reconvergence leaf behavior).
