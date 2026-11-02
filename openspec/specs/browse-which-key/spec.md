# browse-which-key Specification

## Purpose

Transient idle hint bar that surfaces modifier keys for the current view
without requiring the user to open the full help overlay.

## Requirements

### Requirement: Idle hint bar
When the user has been idle for approximately 1 second in Normal input
mode, a one-line hint bar SHALL appear above the status line. The hint
bar SHALL list the modifier keys available in the current view (e.g.,
`f filter | d direction | +/- radius | F edge filter | C columns | ? help`). The hint bar SHALL
disappear on any keypress.

The hint bar SHALL NOT appear when the user is in an input mode (Search,
KindFilter, EdgeKindFilter) or when the help overlay is visible.

Verified by: demonstration (wait idle on the Graph view and confirm the
hint bar appears; press a key and confirm it disappears).

#### Scenario: Hint appears after idle
- **WHEN** the user is in Normal mode on the Graph view and no key is
  pressed for ~1 second
- **THEN** a hint bar appears above the status line listing map-specific
  modifier keys

#### Scenario: Hint disappears on keypress
- **WHEN** the hint bar is visible and the user presses any key
- **THEN** the hint bar disappears immediately

#### Scenario: No hint during input mode
- **WHEN** the user is in Search input mode and idle for >1 second
- **THEN** no hint bar appears

#### Scenario: No hint when help overlay is open
- **WHEN** the help overlay is visible and the user is idle
- **THEN** no hint bar appears

### Requirement: Per-view hint content
Each view SHALL provide its own hint content, generated from the key table. The hint bar SHALL show
only keys relevant to the current view. Views with no modifier keys
beyond the global set MAY show an abbreviated hint or no hint.

Verified by: `cargo test -p latt --locked` (unit test that each
view's hint content is non-empty or explicitly empty).

#### Scenario: Graph view hints
- **WHEN** the hint bar appears on the Graph view
- **THEN** it includes `f filter`, `+/- radius`, `F edge filter`, `C columns`, `? help`

#### Scenario: Node list hints
- **WHEN** the hint bar appears on the node list view
- **THEN** it includes `f filter`, `o sort`, `S schema`, `? help`
