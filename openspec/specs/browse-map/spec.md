# browse-map Specification

## Purpose

Ego-centric graph view for `latt browse` — a Sugiyama layered DAG
display centered on a selected node, with walk-by-re-center navigation,
crossing minimization, and kind/edge-kind filtering.
## Requirements
### Requirement: Graph view activation
Pressing `5` SHALL activate the Graph view. The tab bar SHALL label this
view `Graph`. The view SHALL use a Sugiyama layered DAG layout with
BFS subgraph collection, SCC condensation, longest-path layering,
crossing minimization, and grid rendering. Walk-by-re-center,
kind/edge-kind filters, centrality, vacancy rendering, distance
coloring, and cycle indicators SHALL be preserved.

Verified by: `cargo test -p latt --locked` (app-state
unit tests for activation from each source view; a `TestBackend` rendering
test of the graph against the `mini-toml` fixture).

#### Scenario: Activate from node list
- **WHEN** the user is in the node list with cursor on SPEC-001 and
  presses `5`
- **THEN** the Graph view opens centered on SPEC-001, with the cursor on it

#### Scenario: Activate from detail
- **WHEN** the user is viewing detail for REQ-001 and presses `5`
- **THEN** the Graph view opens centered on REQ-001

#### Scenario: No valid node
- **WHEN** the user is in findings with cursor on an unattached finding
  and presses `5`
- **THEN** nothing happens

#### Scenario: Rendered graph
- **WHEN** browse renders the graph centered on a `mini-toml` node with a
  forward neighbor at 80×24
- **THEN** the center node's ID appears marked as the center and its forward
  neighbor's ID appears in a later layer row

### Requirement: Graph labels fit the view
The graph SHALL label a neighbour with its ID when the ID is at most 32
characters, and otherwise with `…` followed by the ID's last 31 characters.
The center node SHALL be labelled with its whole ID. Activating the graph,
re-centering, and popping the center history SHALL put the cursor on the
center node. When the laid-out graph is wider than the view, the view SHALL
scroll horizontally so the node under the cursor is in sight.

Verified by: `cargo test -p latt --locked` (a `TestBackend` rendering test
over a register with twelve path-shaped test IDs around one requirement).

#### Scenario: Wide neighbourhood
- **WHEN** browse renders the graph at 120×32 centered on a requirement that
  twelve tests with 64-character IDs verify
- **THEN** the center appears whole and marked as the center, and each test
  appears as `…` and the tail of its ID

### Requirement: Sugiyama layered DAG layout
The graph SHALL display nodes in Sugiyama layers ordered by longest-path
assignment from the condensed DAG. Layers above the center layer are
"incoming"; layers below are "outgoing". Within each layer, nodes are
positioned by crossing-minimization column order. The layout applies
SCC condensation, longest-path layering, barycenter crossing
minimization with Phase 2 equal-group reversal, and PR horizontal
positioning.

#### Scenario: Layer structure
- **WHEN** the graph is centered on a node with both incoming and
  outgoing edges
- **THEN** incoming neighbors appear above the center grouped by layer,
  outgoing neighbors appear below grouped by layer

### Requirement: Graph direction toggle
Pressing `d` in Graph mode SHALL toggle the Sugiyama layout flow
direction between forward and reverse and rebuild the graph layout.
Forward orders layers root-to-leaf (natural edge direction); reverse
flips the DAG so leaves appear at the top. Both directions show the
same reachable node set; only the layer ordering changes. The current
direction SHALL be shown in the status line.

#### Scenario: Toggle direction
- **WHEN** the user presses `d` in the Graph view
- **THEN** the layout direction toggles, the graph rebuilds, and the
  layer ordering reverses while the node set remains the same

### Requirement: Depth radius control
The graph SHALL display a configurable depth radius (default 2). Pressing
`+` SHALL increase the radius by 1. Pressing `-` SHALL decrease the
radius by 1 (minimum 1). The status line SHALL show the current radius.

#### Scenario: Increase depth
- **WHEN** the graph has radius 2 and the user presses `+`
- **THEN** the graph rebuilds showing layers up to 3 hops

#### Scenario: Minimum depth
- **WHEN** the graph has radius 1 and the user presses `-`
- **THEN** the radius stays at 1

### Requirement: Walk by re-center
Pressing Enter on a node in the graph SHALL re-center the graph on that
node. The previous center SHALL be pushed onto the center history stack.
`Backspace` or `Esc` SHALL pop the most recent entry from center history
and re-center on it; if the history is empty, they SHALL do nothing.
`q` SHALL quit or pop the view stack — it SHALL NOT pop center history.
`Q` SHALL quit entirely.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for re-center, Backspace pop, and q-always-quits) and demonstration.

#### Scenario: Re-center on neighbor
- **WHEN** the user presses Enter on a layer-1 node B
- **THEN** the graph re-centers on B, with the previous center appearing
  in B's neighborhood

#### Scenario: Pop back via Backspace
- **WHEN** the user has re-centered from A to B and presses Backspace
- **THEN** the graph re-centers back to A

#### Scenario: Pop back via Esc
- **WHEN** the user has re-centered from A to B and presses Esc
- **THEN** the graph re-centers back to A

#### Scenario: q always quits
- **WHEN** the user has re-centered from A to B and presses q
- **THEN** browse quits or pops the view stack; it does NOT re-center on A

#### Scenario: Backspace on empty history
- **WHEN** the center history is empty and the user presses Backspace
- **THEN** nothing happens

### Requirement: Detail from graph
Pressing Tab in the graph SHALL open the node detail pane for the node
under the cursor (using the shared detail pane). Tab again SHALL close
it. This is distinct from Enter (which re-centers).

The status line SHALL indicate `Enter: re-center | Tab: detail`.

#### Scenario: Open detail from graph
- **WHEN** the user presses Tab on a node in the graph
- **THEN** the detail pane opens showing that node's attributes, edges,
  and findings

### Requirement: Kind layer filtering
Pressing `f` in the graph SHALL open the kind filter picker (shared with
the node list). Toggling a kind off SHALL hide all nodes of that kind
from all layers. Connectors and virtual routing glyphs for edges whose
source or target node is filtered out SHALL also be hidden. An empty
layer (all nodes filtered out) renders as a blank line (not hidden).
The edge-kind filter (`F`) SHALL control which connections the BFS
follows; the kind filter (`f`) SHALL control which nodes are displayed
in the result.

#### Scenario: Hide a kind
- **WHEN** the user toggles off kind `test` in the graph
- **THEN** all `test` nodes and their connectors disappear from all layers

#### Scenario: Empty layer shown as blank
- **WHEN** filtering hides all nodes in layer 2
- **THEN** layer 2 renders as a blank line

### Requirement: Vacancy rendering
Undeclared edge targets (vacancies) reachable within the depth radius
SHALL appear in their Sugiyama-assigned layer position with a `◌` marker,
in the terminal's default foreground and without the dim modifier. The
graph SHALL NOT silently drop vacancies.

Verified by: `cargo test -p latt --locked --bin latt no_text_is_dim`
(the Graph render includes a vacancy).

#### Scenario: Vacancy in neighborhood
- **WHEN** the center node has an edge to an undeclared target GHOST
- **THEN** GHOST appears in its assigned layer with `◌`, not dimmed

### Requirement: Centrality weighting
The graph SHALL compute betweenness centrality over the current edge-kind
filter set and visually distinguish high-centrality nodes (bold or
brighter style). Centrality SHALL be recomputed when the edge-kind
filter changes.

#### Scenario: Hub node stands out
- **WHEN** a node has high betweenness centrality in the filtered graph
- **THEN** it renders with a visually distinct style (bold)

### Requirement: SCC condensation
Multi-member strongly connected components SHALL be collapsed into a
single grid position and decorated with a cluster marker (`◆`).

#### Scenario: Center in SCC
- **WHEN** the center node is in an SCC with nodes B and C
- **THEN** the SCC renders as a single position with a cluster marker

### Requirement: Graph state scoping
The graph view's state (center node, depth radius, cursor position, kind
filter, edge-kind filter, search query, center history, direction) SHALL
be independent of other views' state. Switching away from the graph and
back SHALL preserve all graph state.

#### Scenario: State preserved across view switches
- **WHEN** the user sets a radius and kind filter in the graph, switches
  to node list (`1`), and returns to the graph (`5`)
- **THEN** the graph's radius, filter, center, and cursor are preserved

