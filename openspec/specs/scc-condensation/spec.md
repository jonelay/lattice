# scc-condensation Specification

## Purpose
Edge-kind-scoped strongly connected component analysis, extracted
from the COVERAGE_DEEP validator as a shared primitive. These items, with the generic
`algo::scc` and its `Neighbors` trait, are `#[doc(hidden)]`: shared by the core's
validators and the `latt` binary, not supported embedding API.
## Requirements
### Requirement: Admitted-node SCC decomposition
The library SHALL expose a `components()` function that computes
strongly connected components over an admitted node set. It SHALL accept
a pre-filtered `AdjacencyIndex` (controlling edge-kind scope) and an
`admit` set of node IDs (controlling which nodes participate). The admitted set
SHALL be borrowed only for the call: the returned condensation SHALL borrow the
index alone.

#### Scenario: Simple cycle detected
- **WHEN** the graph has A → B → C → A via edge kind "derives"
- **AND** `components()` is called with a "derives"-filtered index and all three admitted
- **THEN** one component is returned containing {A, B, C} with cyclic flag true

#### Scenario: Acyclic graph yields singleton components
- **WHEN** the graph has A → B → C (no cycles) via edge kind "derives"
- **THEN** three singleton components are returned, each with cyclic flag false

#### Scenario: Admitted set outlived by the condensation
- **WHEN** `components()` is called with an admitted set built from strings that are dropped before the condensation is read
- **THEN** the code compiles, and the condensation's components and inter-component edges are those the same set gave before

### Requirement: Component ordering and member identity
Components SHALL appear in Kosaraju topological order. Member IDs within
each component SHALL be sorted lexicographically.

#### Scenario: Component member ordering
- **WHEN** a component contains nodes C, A, B
- **THEN** the members are reported as [A, B, C]

### Requirement: Self-loop detection
A node with an edge to itself (via a scoped edge kind) SHALL be reported
as a single-member cyclic component with a self-loop flag.

#### Scenario: Self-loop node
- **WHEN** node A has an edge to itself via kind "derives"
- **THEN** A's component has one member, cyclic true, self_loop true

### Requirement: Inter-component edges
The result SHALL include inter-component edge references: for each edge
that crosses from one component to another (within the scoped edge kinds),
the source component, target component, and the `&Edge` reference.

#### Scenario: Edge between two components
- **WHEN** components {A, B} and {C} exist, with edge B → C
- **THEN** the inter-component edges include (component-of-AB, component-of-C, &edge)

### Requirement: Generic SCC over a neighbor-enumerating graph
The library SHALL expose `algo::scc()`, generic over the `algo::Neighbors`
trait, returning components as sorted member lists in Kosaraju topological
order. `AdjacencyIndex` SHALL implement `Neighbors`, and `components()`
SHALL obtain its decomposition from `scc()` and add only the condensation
extras (self-loop flags, inter-component edges). `scc()`, `Neighbors` and the other
generic algorithms SHALL be unstable (`#[doc(hidden)]`): shared by the core's
validators and the `latt` binary, and not offered to hosts with their own graphs.

Verified by: `cargo test -p latt-core --locked --lib algo`, and the rustdoc inspection the
graph-service "service-first" requirement names.

#### Scenario: Host graph without a LatticeGraph
- **WHEN** a test type implements `Neighbors` over its own adjacency lists
- **AND** `scc()` is called with every node admitted
- **THEN** the components match those `components()` reports for the same edges

#### Scenario: Unadmitted node breaks a cycle
- **WHEN** A → X → B → A and only A and B are admitted
- **THEN** A and B are separate singleton components

### Requirement: COVERAGE_DEEP consumes shared primitive
The `COVERAGE_DEEP` validator SHALL use the extracted `components()`
function instead of inline SCC logic. Its observable behavior — findings,
severities, fixed-point semantics — SHALL be unchanged.

#### Scenario: COVERAGE_DEEP behavior preserved
- **WHEN** `validate()` runs on a graph with cyclic via-edges and COVERAGE_DEEP rules
- **THEN** the findings are identical to the pre-extraction baseline

Verified by: `cargo test -p latt-core --locked` (unit tests on SCC
detection, member ordering, self-loops, inter-component edges, the
`algo::tests` standalone `Neighbors` implementation, and COVERAGE_DEEP
baseline comparison). The synthetic fixture baseline
at `crates/latt-core/tests/fixtures/synthetic.*` covers COVERAGE_DEEP.

