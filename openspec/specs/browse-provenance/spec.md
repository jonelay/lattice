# browse-provenance Specification

## Purpose
Provenance view for `latt browse` — a lane display showing the
register DAG via `assign_lanes()`, with a git commit history panel
for the selected node's source line via `git log -L`.
## Requirements
### Requirement: Provenance view activation
Pressing `4` SHALL activate the Provenance view. The tab bar SHALL label
this view `Provenance`. The view SHALL render the existing lane layout
(`assign_lanes()` over the register DAG) with a git commit history panel
below the lane display when a target directory is set.

Verified by: demonstration (press `4` and confirm tab label reads
"Provenance").

#### Scenario: Activate provenance view
- **WHEN** the user presses `4`
- **THEN** the Provenance view activates and the tab bar highlights
  `4 Provenance`

### Requirement: Kind filter suppressed in Provenance
Pressing `f` in Provenance mode SHALL open no picker and SHALL say in the
status line that the kind filter does not apply here. The kind filter is
irrelevant to provenance context (commit history is per-node, not
per-kind). The edge-kind filter (`F`) and direction toggle (`d`) SHALL
remain active.

Verified by: `cargo test -p latt --locked`
(unit tests `f_in_provenance_opens_nothing_and_says_why`,
`f_edge_kind_filter_in_provenance`).

#### Scenario: f opens nothing in Provenance
- **WHEN** the user presses `f` while in the Provenance view
- **THEN** the kind filter picker does not open, and the status line says
  why

#### Scenario: F still works in Provenance
- **WHEN** the user presses `F` while in the Provenance view
- **THEN** the edge-kind filter picker opens, and closing it after
  toggling edge kinds rebuilds the lane layout with filtered edges

### Requirement: Git log parser
The `tui::git` module SHALL provide a `parse_git_log` function that
executes `git log -L<start>,<end>:<file>` via `std::process::Command`
and returns a `Vec<GitCommit>`. Each `GitCommit` SHALL contain at
minimum: commit hash, parent hashes, author, date, and subject line. The parser SHALL
return an empty vec when the command fails or the target directory is
not a git repository.

Verified by: `cargo test -p latt --locked` (unit tests parsing
sample `git log -L` output).

#### Scenario: Parse valid git log output
- **WHEN** `parse_git_log` receives well-formed `git log -L` output
- **THEN** it returns a vec of `GitCommit` with hash, parent hashes,
  author, date, and subject populated

#### Scenario: Non-git directory
- **WHEN** `parse_git_log` is called with a target directory that is not
  a git repo
- **THEN** it returns an empty vec

### Requirement: Git commit display in Provenance view
When a node is selected in the Provenance view and a target directory is
set, the view SHALL display git commits touching the node's source line
in a panel below the lane layout. Commits are fetched via
`parse_git_log` (with `--parents` to rewrite parent hashes to the
filtered set) using the node's provenance file and line. Each commit
SHALL show lane-style DAG graph characters (showing parent/child
relationships) followed by the abbreviated hash, date, author, and
subject. Lane assignment is computed by `commit_lanes()` from the
parent relationships. The commit list SHALL be cached per node ID and
refreshed only when the cursor moves to a different node. When no target
directory is set or the node has no exact source line (line 0), the
commit panel SHALL not appear.

The fetch SHALL run on a background thread so cursor movement never
waits on `git`. While a fetch is pending the panel shows a
`Git (fetching…)` title and no rows; the event loop polls for the result
and shortens its tick while a fetch is pending. At most one fetch runs
at a time. A result for a node the cursor has since left SHALL be
discarded, and the fetch for the current node starts in its place.

Verified by: `cargo test -p latt --locked`
(unit tests `provenance_git_commits_empty_without_target`,
`provenance_git_commits_populated_with_target`,
`stale_git_fetch_never_lands_on_a_newer_node`,
`parse_git_log_real_repo`, `lanes_linear_chain_stays_in_lane_zero`,
`lanes_merge_opens_and_closes_side_lane`,
`lanes_unrelated_roots_get_fresh_lanes`).

#### Scenario: Commits shown for selected node
- **WHEN** the Provenance view is active with a target directory set
  and the cursor is on a node with provenance line > 0
- **THEN** a git commit panel appears below the lane layout showing
  commits that touched the node's source line

#### Scenario: No target directory
- **WHEN** the Provenance view is active without a target directory
- **THEN** the commit panel does not appear

#### Scenario: Cursor leaves the node before git answers
- **WHEN** a fetch for node A is in flight and the cursor moves to node B
- **THEN** A's commits are never shown, and B's fetch starts when A's result arrives

### Requirement: The provenance fetch is bounded
The background `git log` fetch for the selected node SHALL wait no longer than 10
seconds. When the bound expires the process SHALL be terminated, the commit panel for
that node SHALL show a `Git (timed out)` title and no rows, and no commits SHALL be
cached for that node, so moving the cursor away and back starts a fresh fetch. A
timed-out fetch for a node the cursor has since left SHALL be discarded like any other
stale result. The `Git (fetching…)` state SHALL never persist longer than the bound.

Verified by: `cargo test -p latt --locked
provenance_git_fetch_times_out` and demonstration with a `git` shim that sleeps.

#### Scenario: git log does not answer
- **WHEN** the Provenance view is active, the cursor is on a node with a source line,
  and `git log` does not exit within the bound
- **THEN** the panel changes from `Git (fetching…)` to `Git (timed out)` with no rows,
  and the git process no longer exists

#### Scenario: Cursor returns after a timeout
- **WHEN** a node's fetch timed out and the cursor leaves and returns to that node
- **THEN** a new fetch starts and the panel shows `Git (fetching…)` again
