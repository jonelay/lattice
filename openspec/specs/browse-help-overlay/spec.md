# browse-help-overlay Specification

## Purpose

Full-screen keybinding help overlay toggled by `?`, dynamically populated
per view, replacing the prior edge-kind legend toggle.

## Requirements

### Requirement: Help overlay toggle
Pressing `?` in Normal input mode SHALL toggle a full-screen help overlay.
The overlay SHALL obscure the current view content but not destroy view
state. The navigation keys (`j`/`k`, `g`/`G`, PageDown/PageUp and their
equals) SHALL scroll the overlay within its content, `Q` SHALL quit browse,
and `?` or any other key SHALL dismiss the overlay and return to the
previous view. The overlay SHALL open scrolled to the top.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for overlay toggle, `help_overlay_scrolls_within_its_content`) and demonstration.

#### Scenario: Open help overlay
- **WHEN** the user presses `?` in Normal mode on any view
- **THEN** a full-screen overlay appears listing keybindings

#### Scenario: Dismiss with question mark
- **WHEN** the help overlay is visible and the user presses `?`
- **THEN** the overlay closes and the previous view is restored unchanged

#### Scenario: Dismiss with any key
- **WHEN** the help overlay is visible and the user presses a key that is
  not `?`, a navigation key or `Q`
- **THEN** the overlay closes; the keypress is consumed (not forwarded
  to the underlying view)

#### Scenario: Scroll a help overlay taller than the terminal
- **WHEN** the help overlay has more lines than the terminal shows and the
  user presses `G`
- **THEN** the overlay scrolls to its last line and stays open; `j` past
  the end does nothing and `g` returns to the top

### Requirement: Per-view binding sections
The help overlay SHALL display keybindings grouped by section. Each
top-level view SHALL register its own binding set. The overlay SHALL
show:
1. Global bindings (quit, view switching, search, help toggle) — present
   in every view.
2. View-specific bindings — only those applicable to the current
   top-level view.

Bindings SHALL be displayed as `key — description` rows, left-aligned,
grouped under section headers.

Verified by: `cargo test -p latt --locked` (unit tests verifying
each view produces a non-empty binding list) and demonstration.

#### Scenario: Graph view bindings shown
- **WHEN** the user opens the help overlay from the Graph view
- **THEN** the overlay shows global bindings and map-specific bindings
  (Enter, Tab, +/-, F, C, Backspace)

#### Scenario: Node list bindings shown
- **WHEN** the user opens the help overlay from the node list view
- **THEN** the overlay shows global bindings and node-list-specific
  bindings (f, s, g, Enter)

#### Scenario: View switch updates overlay
- **WHEN** the user switches from view 1 to view 5 and opens the overlay
- **THEN** the overlay shows map-specific bindings, not node-list bindings

### Requirement: Edge-kind legend in overlay
The edge-kind color legend previously shown by `?` SHALL be included as
a section within the help overlay, labeled "Edge kinds". Each edge kind
SHALL show its assigned color swatch and name.

Verified by: demonstration (open help overlay on a register with multiple
edge kinds and confirm the legend section is present).

#### Scenario: Legend section present
- **WHEN** the help overlay is open on a register with edge kinds
  "verifies", "contains", and "derives"
- **THEN** a section labeled "Edge kinds" lists each kind with its color
