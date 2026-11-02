# browse-map-breadcrumb Specification

## Purpose

Navigation breadcrumb bar for the Graph view that surfaces the center
history stack and makes `q` always quit instead of silently popping.

## Requirements

### Requirement: Breadcrumb bar
When the Graph view has a non-empty `center_history`, a breadcrumb line
SHALL render above the map row list showing the navigation path. The
format SHALL be `root-id > node-a > node-b > current-id`, with `>`
as the separator. Node IDs SHALL be truncated with `…` if the
breadcrumb exceeds the terminal width.

When `center_history` is empty (initial state), the breadcrumb line
SHALL NOT render — the map list occupies the full area.

Verified by: `cargo test -p latt --locked` (unit test for
breadcrumb rendering with 0, 1, and 3+ history entries) and
demonstration.

#### Scenario: No history, no breadcrumb
- **WHEN** the Graph view is first opened (empty center_history)
- **THEN** no breadcrumb line is displayed

#### Scenario: One re-center shows breadcrumb
- **WHEN** the user re-centers from node A to node B
- **THEN** a breadcrumb line appears: `A > B`

#### Scenario: Multiple re-centers
- **WHEN** the user navigates A → B → C → D
- **THEN** the breadcrumb shows `A > B > C > D`

#### Scenario: Truncation on narrow terminal
- **WHEN** the breadcrumb path exceeds terminal width
- **THEN** earlier entries are elided with `…` (e.g., `… > C > D`)

### Requirement: Backspace/Esc pops center history
Pressing `Backspace` or `Esc` in the Graph view SHALL pop the most recent
entry from `center_history` and re-center the map on it. If the history
is empty, `Backspace` and `Esc` SHALL do nothing.

Verified by: `cargo test -p latt --locked` (unit test for
history pop via Backspace) and demonstration.

#### Scenario: Backspace pops history
- **WHEN** the user has navigated A → B → C and presses Backspace
- **THEN** the map re-centers on B; the breadcrumb shows `A > B`

#### Scenario: Backspace on empty history
- **WHEN** the center_history is empty and the user presses Backspace
- **THEN** nothing happens

### Requirement: q always quits
In the Graph view, pressing `q` SHALL quit browse (or pop the view stack
if views are stacked). `q` SHALL NOT pop `center_history`. The
center-history-pop behavior previously on `q` is replaced by
`Backspace`/`Esc`.

Verified by: `cargo test -p latt --locked` (unit test confirming
`q` produces Action::Quit in the Graph view regardless of center_history
depth) and demonstration.

#### Scenario: q quits with history
- **WHEN** the user has navigated A → B → C in the map and presses `q`
- **THEN** browse quits (or pops the view stack); it does NOT re-center
  on B

#### Scenario: q quits with empty history
- **WHEN** the center_history is empty and the user presses `q`
- **THEN** browse quits (same behavior as before)
