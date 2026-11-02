# browse-tree Specification

## Purpose

Rooted reachability tree view within `latt browse`, visualizing depth-first
traversal with expand/collapse, severity markers, taint, and SCC collapse.

## Requirements
### Requirement: Tree view activation and root selection
Pressing `2` SHALL switch to the reachability tree view. The root node
SHALL be determined by context:
- From the node list: the node under the cursor.
- From the node detail: the open detail node.
- From the findings view: the finding's attached node, if any; otherwise
  `2` SHALL do nothing.
- From within the tree view: pressing `2` SHALL re-root the tree at the
  node under the cursor.

If the selected node has no outgoing edges (forward) or no incoming edges
(reverse), the tree SHALL display the root as a single leaf row.

Verified by: `cargo test -p latt --locked` (app-state
unit tests for root selection from each source view; a `TestBackend`
rendering test of the tree against the `mini-toml` fixture).

#### Scenario: Root from node list
- **WHEN** the cursor is on node REQ-001 in the node list and the user presses `2`
- **THEN** the tree view opens rooted at REQ-001

#### Scenario: Re-root within tree
- **WHEN** the tree is showing and the cursor is on node TEST-003 and the user presses `2`
- **THEN** the tree rebuilds rooted at TEST-003

#### Scenario: Root from finding without node
- **WHEN** the cursor is on an unattached finding and the user presses `2`
- **THEN** nothing happens (the view does not switch)

#### Scenario: Leaf root
- **WHEN** the selected node has no outgoing edges
- **THEN** the tree shows the root as a single row with no children

#### Scenario: Rendered tree
- **WHEN** browse renders the tree rooted at a `mini-toml` node with forward
  edges at 80×24 and the root expanded
- **THEN** the first content row is the root and the next row is an indented
  child row carrying the edge kind label and the child's ID

### Requirement: Lazy expand and collapse
Tree rows SHALL start collapsed (root visible, children hidden). Space
SHALL expand the node under the cursor by one level, revealing its
immediate children. `-` SHALL collapse the node, hiding all descendants.
Enter SHALL push the node's detail onto the view stack (consistent with
other views).

Expansion state SHALL reset when the tree rebuilds (root change,
direction toggle, edge-kind filter change).

Verified by: `cargo test -p latt --locked` (app-state unit tests
for expand, collapse, and reset on rebuild).

#### Scenario: Expand root
- **WHEN** the tree is rooted at A (collapsed) and the user presses Space
- **THEN** A's immediate children become visible

#### Scenario: Collapse hides descendants
- **WHEN** A is expanded showing B, and B is expanded showing C,
  and the user collapses A
- **THEN** B and C are hidden

#### Scenario: Enter pushes detail
- **WHEN** the cursor is on a tree row for node REQ-001 and the user presses Enter
- **THEN** the node detail view is pushed onto the view stack

#### Scenario: Expansion resets on direction change
- **WHEN** several nodes are expanded and the user toggles direction
- **THEN** the tree rebuilds and all nodes start collapsed

### Requirement: Inline severity markers
Each tree row for a declared node SHALL show the worst unsuppressed
finding severity and the count of unsuppressed findings, using the same
severity coloring as the node list view. Nodes with no findings SHALL
show no marker.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for severity marker content).

#### Scenario: Node with errors
- **WHEN** node REQ-001 has 2 error-severity and 1 warning-severity unsuppressed findings
- **THEN** its tree row shows the error severity marker with count 3

#### Scenario: Node with no findings
- **WHEN** node TEST-005 has no unsuppressed findings
- **THEN** its tree row shows no severity marker

### Requirement: Edge-kind coloring
The edge leading to each tree row SHALL be colored by the edge's kind,
consistent with the `?` legend. The root row has no edge and SHALL show
no edge-kind color.

Verified by: demonstration.

#### Scenario: Edge color matches kind
- **WHEN** a tree row was reached via a "verifies" edge
- **THEN** the edge indicator is colored with the "verifies" kind color

### Requirement: Taint and endpoint annotations
A tree row for a vacancy endpoint (undeclared node) SHALL be visually
distinct (dimmed or marked with a vacancy indicator). A tree row for a
tainted node (every path from root crosses a vacancy) SHALL carry a
taint annotation. Taint and vacancy are orthogonal and both may apply.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for taint and vacancy flags on visible rows).

#### Scenario: Vacancy endpoint
- **WHEN** a tree row's node is undeclared
- **THEN** the row carries a vacancy indicator

#### Scenario: Tainted node
- **WHEN** every path from the root to a node crosses a vacancy
- **THEN** the row carries a taint annotation

#### Scenario: Both vacancy and taint
- **WHEN** a node is both undeclared and tainted
- **THEN** the row carries both annotations

### Requirement: SCC cluster collapse
When the traversal covers a strongly connected component with more than
one declared member, the tree SHALL optionally collapse the cluster into
a single summary row showing member count, member IDs, and the edge
kinds within the cluster. The summary row SHALL be expandable (Space) to
show the individual member rows.

SCC collapse SHALL be a toggle (`z`), default off. When off, the tree
shows the raw traversal with cycle markers. When on, multi-member
clusters collapse. Single-member and non-cyclic components are unaffected.
The toggle SHALL be listed in the Tree view's help section.

The admitted set for SCC computation SHALL be the declared node IDs
present in the traversal, scoped to the same filtered edge-kind index.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for cluster collapse toggle and visible row content).

#### Scenario: Cluster collapse on
- **WHEN** the traversal contains a 3-node cycle {A, B, C} and SCC collapse is on
- **THEN** the tree shows one cluster row "[3 nodes: A, B, C]" at the
  first-encounter position, with in-cluster cycle steps hidden

#### Scenario: Cluster collapse off
- **WHEN** the same 3-node cycle exists and SCC collapse is off
- **THEN** the tree shows the raw DFS order with cycle visit markers

#### Scenario: Expand cluster
- **WHEN** SCC collapse is on, the cursor is on a cluster row, and the user presses Space
- **THEN** the cluster expands to show its member rows

### Requirement: Direction toggle
Pressing `d` SHALL toggle the traversal direction between forward
(outgoing edges) and reverse (incoming edges). The tree SHALL rebuild
from the same root node in the new direction. Expansion state SHALL
reset on toggle.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for direction toggle and rebuild).

#### Scenario: Toggle to reverse
- **WHEN** the tree is in forward direction and the user presses `d`
- **THEN** the tree rebuilds showing incoming-edge reachability from the same root

#### Scenario: Direction persists across re-root
- **WHEN** the user sets reverse direction and then re-roots at a different node
- **THEN** the new tree uses reverse direction

### Requirement: Edge-kind filter
Pressing `F` SHALL open a picker listing all edge kinds in the profile,
each with a checkbox. `j`/`k` navigate, Space toggles, Enter or Esc
closes. The tree SHALL rebuild using only the checked edge kinds,
filtering the adjacency index before traversal. All kinds SHALL be
checked by default.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for edge-kind filter state and rebuild trigger).

#### Scenario: Filter to one edge kind
- **WHEN** the user unchecks all edge kinds except "verifies"
- **THEN** the tree shows only reachability through "verifies" edges

#### Scenario: All kinds checked by default
- **WHEN** the tree view is first entered
- **THEN** all edge kinds are checked

### Requirement: Search in tree view
`/` SHALL open a regex search prompt in the tree view. Matches SHALL be
highlighted among the visible (expanded) rows. `n` and `N` SHALL cycle
through matches. Search SHALL highlight, not reduce the visible set.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for search match cycling in tree context).

#### Scenario: Search highlights visible rows
- **WHEN** the user searches for "REQ" in the tree view
- **THEN** visible rows matching "REQ" are highlighted

#### Scenario: Search skips collapsed rows
- **WHEN** a matching node is under a collapsed parent
- **THEN** it is not included in the match cycle

### Requirement: Status line in tree view
The status line SHALL show "Tree" as the view name, the root node ID,
the current direction (forward/reverse), the count of reached nodes,
and the active edge-kind filter (if not all kinds are checked).

Verified by: demonstration.

#### Scenario: Status line content
- **WHEN** the tree view is active rooted at REQ-001 in forward direction
  reaching 15 nodes with all edge kinds checked
- **THEN** the status line shows "Tree REQ-001 forward 15 reached"

#### Scenario: Status line with filter
- **WHEN** the edge-kind filter excludes some kinds
- **THEN** the status line shows which kinds are active

### Requirement: Jump to source from tree
Pressing `e` on a tree row SHALL use the node's provenance to open
`$EDITOR` at the file:line, consistent with the node list and detail
views. If the node is a vacancy (undeclared), `e` SHALL do nothing.

Verified by: demonstration.

#### Scenario: Editor on tree row
- **WHEN** the user presses `e` on a tree row for a declared node
- **THEN** the TUI suspends and `$EDITOR` opens at the node's provenance

#### Scenario: Editor on vacancy
- **WHEN** the user presses `e` on a vacancy row
- **THEN** nothing happens
