# browse-map-columns Specification

## Purpose

Edge-kind labels and toggleable column display in the Graph view rows so users
can see relationship types at a glance and control information density.

## Requirements

### Requirement: Edge-kind label in map rows
Each row in the Graph view SHALL display the edge kind between the
direction arrow and the target node ID. The format SHALL be
`→ [verifies] REQ-001 Summary text` for outgoing edges and
`← [verifies] REQ-001 Summary text` for incoming edges. The edge-kind
label SHALL use the same color assigned to that edge kind elsewhere in
the TUI.

Verified by: `cargo test -p latt --locked` (unit test for map
row formatting with edge-kind label) and demonstration.

#### Scenario: Outgoing edge with kind
- **WHEN** the map shows an outgoing edge of kind "verifies" to node
  REQ-001
- **THEN** the row renders as `→ [verifies] REQ-001 …`

#### Scenario: Incoming edge with kind
- **WHEN** the map shows an incoming edge of kind "contains" from node
  CAP-002
- **THEN** the row renders as `← [contains] CAP-002 …`

### Requirement: Column toggle
Pressing `C` in the Graph view SHALL cycle through column display modes:
1. **Default**: edge kind visible, node kind hidden.
2. **Full**: edge kind and node kind both visible.
3. **Compact**: both hidden (direction arrow + node ID + summary only).

The current column mode SHALL be indicated in the status bar.

Verified by: `cargo test -p latt --locked` (unit test for column
mode cycling) and demonstration.

#### Scenario: Default column mode
- **WHEN** the Graph view opens
- **THEN** rows show edge kind labels; node kind is not shown

#### Scenario: Cycle to full
- **WHEN** the user presses `C` once from default mode
- **THEN** rows show both edge kind and node kind; status bar indicates
  "columns: full"

#### Scenario: Cycle to compact
- **WHEN** the user presses `C` twice from default mode
- **THEN** rows show only direction + node ID + summary; status bar
  indicates "columns: compact"

#### Scenario: Cycle wraps
- **WHEN** the user presses `C` three times from default mode
- **THEN** the display returns to default mode
