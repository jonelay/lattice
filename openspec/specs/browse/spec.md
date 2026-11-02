# browse Specification

## Purpose

Read-only interactive TUI for navigating a loaded register — nodes, edges,
and findings — without leaving the terminal.
## Requirements
### Requirement: Browse subcommand
`latt browse` SHALL accept `--profile`, `--adapter`, `--target`, `--strict`,
`--filter`, and `--glyphs auto|unicode|ascii`. When `--glyphs` is absent the
glyph set SHALL resolve as `auto`. An explicit `--glyphs` SHALL take precedence
over any configuration file setting for the glyph set. It SHALL NOT accept
`--format` — browse owns its terminal directly and does not route output
through the batch output dispatcher.

Browse SHALL load the profile, run the adapter, ingest the document, and run
validation at startup. The loaded graph, profile, and validation findings are
then available for interactive navigation. The adapter and validation run again
only when the user reloads (see Reload); there is no watch mode.

Verified by: demonstration (launch `latt browse` with a fixture register
and confirm the TUI appears with navigable content), and
`cargo test -p latt --locked --bin latt glyph_flag_tests`
(the parsed `--glyphs` value for each of the three values and for absence, and a
clap invalid-value error for any other).

#### Scenario: Launch browse
- **WHEN** the user runs `latt browse --profile p.yaml --target /repo`
- **THEN** the TUI starts in alternate screen showing the node list view

#### Scenario: Browse rejects --format
- **WHEN** the user runs `latt browse --profile p.yaml --target /repo --format json`
- **THEN** clap reports an unrecognized argument error

#### Scenario: Browse rejects an unknown glyph set
- **WHEN** the user runs `latt browse --profile p.yaml --target /repo --glyphs wide`
- **THEN** clap reports an invalid value naming `auto`, `unicode` and `ascii`, and
  the exit code is 2

### Requirement: Feature gate
`browse` SHALL be available only when the `tui` feature of the `latt` package is
enabled. The feature SHALL be on by default, so `cargo install latt` builds `browse`.
The `latt-core` library SHALL have no `tui` feature, so a library consumer compiles
no TUI dependency whatever features it picks.
Without the feature, the `browse` subcommand SHALL be absent from the CLI —
`latt browse` SHALL print an unrecognized-subcommand error rather than a
missing-feature message.

Verified by: `cargo test -p latt --test cli --locked` and
`cargo test -p latt --no-default-features --test cli --locked`.

#### Scenario: Feature on
- **WHEN** lattice is built with default features
- **THEN** `latt browse --help` exits 0

#### Scenario: Feature off
- **WHEN** lattice is built with `--no-default-features`
- **THEN** `latt browse` fails with an unrecognized subcommand error and exit 2

#### Scenario: The library carries no TUI dependency
- **WHEN** `latt-core`'s manifest is read
- **THEN** no section but `[dev-dependencies]` declares `tui`, `clap`, `ratatui`,
  `tui-markdown`, `signal-hook` or `crossterm`

### Requirement: Exit contract
Browse SHALL exit 0 when the user quits and no error-severity findings exist
(after suppression and optional `--strict` promotion). Browse SHALL exit 1
when error-severity findings exist. Browse SHALL exit 2 when it cannot start
at all (bad profile, adapter failure, terminal init failure, `TERM=dumb`, or a
stdout that is not a terminal).
`TERM=dumb` SHALL be detected before the profile is loaded; browse SHALL then
write nothing to stdout, print to stderr that it needs a terminal with cursor
addressing and name `--format plain` on the other commands. A stdout that is not a
terminal SHALL be detected at the same point, after the `TERM=dumb` check, with a
stderr message that says browse needs a terminal and names `--format plain`, so the
refusal names its cause rather than an OS device error.

Verified by: demonstration (quit browse on a clean register → exit 0; quit
on a register with errors → exit 1; launch with a bad profile → exit 2), and
`cargo test -p latt --test cli --locked browse_dumb_terminal`
(run with `TERM=dumb` and a profile path that does not exist: the stderr names
cursor addressing and `--format plain`, not the missing profile; stdout is
empty; exit 2) and `browse_without_a_terminal` (the same with a captured stdout).

#### Scenario: Clean quit
- **WHEN** browse loads a register with no errors and the user presses `Q`
- **THEN** the exit code is 0

#### Scenario: Quit with errors
- **WHEN** browse loads a register with error-severity findings and the user quits
- **THEN** the exit code is 1

#### Scenario: Init failure
- **WHEN** browse cannot start (bad profile, adapter failure, or terminal init error)
- **THEN** the exit code is 2

#### Scenario: Dumb terminal
- **WHEN** the user runs `latt browse` with `TERM=dumb`
- **THEN** stderr says browse needs a terminal with cursor addressing and names
  `--format plain`, nothing is written to stdout, the profile is not read, and
  the exit code is 2

#### Scenario: No terminal
- **WHEN** the user runs `latt browse` with stdout redirected to a file or pipe
- **THEN** stderr says browse needs a terminal and names `--format plain`, nothing is
  written to stdout, the profile is not read, and the exit code is 2

### Requirement: Terminal lifecycle
Browse SHALL enter alternate screen and raw mode on startup and restore the
terminal on exit, whether by normal quit, error, signal, or panic. The
lifecycle SHALL be ratatui's `init()` and `restore()`: `init()` installs a
panic hook that restores the terminal before the previous hook prints, and
the terminal session calls `restore()` once, when it is dropped, whichever
way browse ends. Suspending for `$EDITOR` or for an interactive owner action
SHALL restore before the program starts and re-enter afterwards without
installing a second panic hook. A reload SHALL keep the terminal in the
alternate screen; it is restored once, when browse ends.

Key events whose kind is not `Press` SHALL be ignored, so a terminal that
reports key release or repeat events does not fire a binding twice.

SIGTERM, SIGHUP, SIGINT and SIGQUIT SHALL each end the event loop through
that restore, with the exit code of the last load that succeeded. SIGINT and
SIGQUIT received while `$EDITOR` or an interactive owner action runs SHALL be
left to that program, as `system()` does: the program shares browse's terminal,
so its Ctrl-C reaches browse too.

A terminal that hangs up SHALL end browse: input SHALL be read through a
backend that reports the failed read rather than retrying it, so the event
loop is not held inside one poll. With no terminal left to restore, browse
exits 2 on the I/O error.

Verified by: demonstration (quit browse and confirm the terminal is restored;
run browse under `timeout -s TERM` and confirm the terminal is restored;
trigger a panic and confirm the message prints on a restored terminal) and
`cargo test -p latt --locked --test cli`
(`browse_exits_when_its_terminal_hangs_up`,
`browse_restores_the_terminal_on_each_terminating_signal`,
`an_interrupt_during_the_editor_does_not_end_browse`,
`an_interrupt_during_an_interactive_action_does_not_end_browse`; Linux).

#### Scenario: Normal quit restores terminal
- **WHEN** the user quits browse normally
- **THEN** the original terminal state is fully restored

#### Scenario: Panic restores terminal
- **WHEN** browse panics
- **THEN** the terminal is restored before the panic message prints

#### Scenario: Signal restores terminal
- **WHEN** browse receives SIGTERM, SIGHUP, SIGINT or SIGQUIT
- **THEN** the event loop exits, the terminal is restored, and the exit code
  is the last successful load's

#### Scenario: An interrupt in the editor stays in the editor
- **WHEN** browse receives SIGINT or SIGQUIT while `$EDITOR` runs
- **THEN** browse resumes when the editor exits

#### Scenario: An interrupt in an interactive action stays in the action
- **WHEN** browse receives SIGINT while an interactive owner action runs
- **THEN** browse resumes when the action's program exits

#### Scenario: A terminal that hangs up ends browse
- **WHEN** the terminal browse runs in goes away, closing the pty
- **THEN** browse exits within seconds instead of running on, and the exit
  code is 2

#### Scenario: Release events are ignored
- **WHEN** the terminal reports a key release event for `j`
- **THEN** the cursor does not move

### Requirement: Accessibility baseline
Browse SHALL be usable without rebuilding lattice or configuring it. The
references behind this requirement are W3C WCAG2ICT 2.2 Appendix B (terminal
applications), WCAG 2.2 SC 1.4.1, 1.4.3, 1.4.11 and 2.1.1, no-color.org,
force-color.org, clig.dev, and Unicode UAX #11 §5.

**Color and modifiers.** Every meaning browse carries in color SHALL also be
carried by text or by a modifier (bold, underline, reverse): the cursor row is
reversed, the focused pane's border bold, a search match off the cursor
underlined, the tab and status bars reversed with the active tab not reversed.
Browse SHALL NOT use fixed white, gray or black foregrounds or backgrounds
except as an explicit pair. Edge-kind and severity colors are decoration
beside the kind or severity name.

**Dim.** No text SHALL carry the dim modifier: not node IDs or summaries,
vacancy and taint markers, key hints, dates, line numbers, the breadcrumb,
findings of any severity, schema rows, missing-edge rows, pathway positions,
pane titles, or Markdown rendered in the file preview. Dim SHALL appear only
on connector lane glyphs and on the border lines of an unfocused pane.

**NO_COLOR.** When `NO_COLOR` is set to a non-empty value, every cell of every
frame SHALL carry the terminal's default foreground and background, keeping
its modifiers.

**Terminal cursor.** On every frame browse SHALL show the terminal's cursor
and place it where the user's attention is:
- in a list view or the detail pane with focus, at the first cell inside the
  border on the selected row;
- in the Graph view, in every column mode, at the first cell of the selected
  node as drawn;
- with a filter picker open, on the picker's selected row;
- in the search prompt, after the typed text;
- with the help overlay or file preview open, at its first content cell.

When the focused pane has no rows, the cursor SHALL sit at its first inner
cell. The position SHALL always lie inside the frame. The reverse-video
highlight SHALL remain alongside the cursor.

**Keys.** Every action SHALL have a key that is not a Control chord: `Ctrl-D`
and `Ctrl-U` sit beside PageDown and PageUp, `Ctrl-C` beside `Q`, and `Ctrl-F`
beside `&`.

**Screen readers.** Browse redraws the whole screen, which a screen reader
cannot follow; the help overlay, `latt browse --help` and the README SHALL
name `--format plain` on the other commands for reading line by line and
`--format json` for structure.

**Glyphs.** Browse's glyphs SHALL be single-width. `--glyphs` selects the set:
`unicode` draws browse's own glyphs; `ascii` replaces every glyph browse and
its widgets draw (borders, scrollbars, arrows, markers) cell for cell by an
ASCII stand-in, so the layout is unchanged; `auto` chooses `unicode` when the
character-type locale is UTF-8 and `ascii` otherwise — the first of `LC_ALL`,
`LC_CTYPE`, `LANG` that is set and non-empty decides, and none set is the `C`
locale. The ASCII mapping applies to every cell, register text included;
characters outside the glyph table are left as they are.

Verified by: `cargo test -p latt --locked` —
`no_color_frame_keeps_every_meaning_in_modifiers`, `no_text_is_dim`,
`markdown_preview_is_never_dim`, `cursor_follows_the_selection`,
`cursor_stays_inside_a_tiny_frame`, `ascii_frame_keeps_layout_with_ascii_glyphs`,
`glyph_mode_tests`, `locale_tests`, `search_filter_has_a_key_without_a_ctrl_chord`,
`every_ctrl_chord_has_a_plain_key_for_the_same_command` and
`help_overlay_scrolls_within_its_content` — and demonstration:
- run browse under `script` with and without `NO_COLOR=1` and compare the SGR
  codes: color codes such as `38;5;6` appear only without it;
- run it with `LANG=C`, and with `--glyphs ascii` under `LANG=C.UTF-8`, and find
  no non-ASCII character in the frame;
- press `j` twice and find, before the closing `ESC[?1049l`, one `ESC[?25h` per
  frame and a last cursor-position sequence to the first inner cell of the node
  list's third row (`ESC[5;2H` at 80×24).

#### Scenario: NO_COLOR
- **WHEN** browse runs with `NO_COLOR=1`
- **THEN** no cell carries a foreground or background color, the cursor row
  is still reversed, the focused border bold, and search matches underlined

#### Scenario: Non-UTF-8 locale
- **WHEN** browse runs with `LANG=C`, neither `LC_ALL` nor `LC_CTYPE` set, and
  no `--glyphs`
- **THEN** borders draw as `+`, `-` and `|`, arrows as `>` and `<`, and the
  frame carries no non-ASCII glyph of browse's own

#### Scenario: Forced ASCII in a UTF-8 locale
- **WHEN** browse runs with `LANG=ja_JP.UTF-8` and `--glyphs ascii`
- **THEN** the frame is drawn with the ASCII stand-ins

#### Scenario: Forced Unicode in a C locale
- **WHEN** browse runs with `LANG=C` and `--glyphs unicode`
- **THEN** the frame is drawn with browse's Unicode glyphs

#### Scenario: Light terminal theme
- **WHEN** browse runs on a terminal with a light background
- **THEN** no text is dimmed; only connector lanes and the border lines of an
  unfocused pane are

#### Scenario: Markdown preview with front matter
- **WHEN** the user previews a Markdown file with front matter and a footnote
- **THEN** no cell of the preview carries the dim modifier

#### Scenario: Cursor follows the selection
- **WHEN** the user moves the node-list cursor down two rows
- **THEN** the terminal cursor is shown at the first cell inside the pane
  border on the newly selected row

#### Scenario: Cursor in the detail pane
- **WHEN** the detail pane has focus
- **THEN** the terminal cursor sits on the detail pane's selected row, not the
  node list's

#### Scenario: Cursor in an empty pane
- **WHEN** a search filter leaves the focused pane with no rows
- **THEN** the terminal cursor sits at the pane's first inner cell

#### Scenario: Cursor in a tiny frame
- **WHEN** browse draws into a 20×5 terminal
- **THEN** the terminal cursor position lies inside the frame

### Requirement: Key table
Browse SHALL resolve every key through one key table. A row names the
layer it belongs to, the views it runs in, an optional state guard, its
keys with their modifiers, and the command it runs. Rows in the always
layer (`Ctrl-C`, `Ctrl-F`) SHALL fire in every mode. Otherwise the state
selects exactly one layer — an overlay (the file preview, the action launcher
menu, the action confirmation, the action output pane), then the input mode, then normal — and at most
one row of that layer SHALL match a given key. A
layer may also hold a catch-all row, which fires only when no other row
of the layer matches. A
Control or Alt chord SHALL match only a row that declares that modifier;
Shift SHALL be ignored. The help overlay's rows and the footer's key
labels SHALL be generated from the table. `Q` SHALL quit from normal mode,
the file preview, the action launcher menu, the action confirmation, the
action output pane, both
filter pickers and the help overlay; the search prompt takes it as text.

Verified by: `cargo test -p latt --locked`
(`tui::dispatch_tests`: the dispatch and help baselines, and the
one-row-per-key and table-shape checks).

#### Scenario: An undeclared chord does nothing
- **WHEN** the user presses `Ctrl-D` in the tree view
- **THEN** the edge direction does not change

#### Scenario: Shift does not change a binding
- **WHEN** the terminal reports `Q` with the Shift modifier in normal mode
- **THEN** browse quits, as it does for `Q` without it

#### Scenario: Q quits from an overlay
- **WHEN** the user presses `Q` with the file preview, the action launcher menu,
  the action output pane or a filter picker open
- **THEN** browse quits

#### Scenario: Ctrl-C quits from any mode
- **WHEN** the user presses `Ctrl-C` with the search prompt, a filter
  picker, the help overlay or the file preview open
- **THEN** browse quits

### Requirement: Node list view
The landing view SHALL display all nodes grouped by kind in profile
declaration order, with node ID and a one-line `summary_attr` value
(truncated to terminal width). If `summary_attr` is absent for a kind,
the view SHALL show kind + ID only. When the ID ends with the summary, as when
a register uses a title as the ID, the row SHALL show kind + ID only. Non-string
values SHALL render as their JSON representation, truncated.

The view SHALL support:
- Attr condition filter (`--filter`-style conditions applied at startup).
- Sort mode cycle (`o`) — kind order (default) and finding severity
  (worst unsuppressed severity first, then count).
- Schema toggle (`S`) — switch between instance view (actual nodes) and
  schema view (profile node kinds with instance counts and zero-count
  allowed triples highlighted as structural gaps).

The schema view SHALL have a cursor moved by the navigation keys
that never rests on a separator row, SHALL window its rows to the terminal
height around the cursor, and SHALL show a scrollbar when the rows overflow,
matching the other views. Toggling back to the instance view SHALL restore
the node-list cursor to where it was.

Verified by: `cargo test -p latt --locked` (app-state
unit tests for filtering, sorting, schema toggle and schema cursor; a
`TestBackend` rendering test of the node list against the `mini-toml`
fixture) and demonstration.

#### Scenario: Default landing view
- **WHEN** browse starts
- **THEN** the node list shows all nodes grouped by kind in profile order

#### Scenario: Rendered node list
- **WHEN** browse renders the `mini-toml` fixture at 80×24
- **THEN** the first content row carries the first fixture node's kind and ID
  and the cursor row is highlighted; a scrollbar is present only when the
  node count exceeds the visible rows

#### Scenario: Kind filter toggle
- **WHEN** the user presses `f` with the list focused in the node list,
  findings, tree or graph view
- **THEN** a picker opens listing all profile kinds with checkboxes;
  `j`/`k` navigate, Space toggles, Enter, Esc or `f` closes without
  toggling
- **WHEN** a kind is toggled off in the node list
- **THEN** nodes of that kind are hidden from the list
- **WHEN** a kind is toggled off in the findings view
- **THEN** findings attached to nodes of that kind are hidden;
  unattached findings remain visible
- **WHEN** a kind is toggled off in the tree view
- **THEN** nodes of that kind and their descendants are hidden
- **WHEN** a kind is toggled off in the Provenance view
- **THEN** rows for nodes of that kind are hidden from the lane

#### Scenario: Severity sort
- **WHEN** the user presses `o` to switch to severity sort
- **THEN** nodes with the worst unsuppressed findings appear first

#### Scenario: Schema toggle
- **WHEN** the user presses `S`
- **THEN** the view switches to show profile kinds with instance counts and
  zero-count allowed triples, with the cursor on the first kind

#### Scenario: Schema view scrolls
- **WHEN** the schema view has more rows than the terminal shows and the
  user presses `j` past the last visible row
- **THEN** the window scrolls to keep the cursor visible and a scrollbar is
  shown
- **WHEN** the user presses `j` on the last kind row
- **THEN** the cursor lands on the first triple row, skipping the separator
- **WHEN** the user presses `S` to return to the instance view
- **THEN** the node-list cursor is where it was before entering schema mode

### Requirement: Node detail view
Pressing Enter on a node in the node list SHALL open a split-pane detail
view below the list. The detail SHALL show:
- Attrs table (all node attributes), one row each; a line break in a string
  value SHALL render as a space.
- Outgoing edges grouped by edge kind.
- Incoming edges grouped by edge kind.
- Expected-but-missing edges — edges the profile allows from/to this
  node's kind that do not exist in the instance, visually distinct
  (dimmed or otherwise differentiated). Each row SHALL name the side that is
  missing, outgoing or incoming, so a triple whose source and target kinds are the
  same never reads as missing on a node that has the edge on the other side. A node
  whose kind sets `orphan_ok: true` SHALL show no expected-but-missing rows.
- Pathway position — for each pathway the node participates in, the
  ordered positions with the current position highlighted.
- Attached findings with severity and code.

Tab SHALL switch focus between the list pane and the detail pane.
Pressing `1` SHALL maximize the list (close the detail pane).
The detail pane SHALL be scrollable.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for detail content, focus switching) and demonstration.

#### Scenario: Open detail
- **WHEN** the user presses Enter on a node in the list
- **THEN** a detail pane opens below showing attrs, edges, expected edges,
  pathway position, and findings

#### Scenario: Expected edges shown
- **WHEN** a node's kind allows edge kind "verifies" to kind "test" but no
  such edges exist
- **THEN** the detail shows this as an expected-but-missing outgoing edge

#### Scenario: Same-kind triple with only one side present
- **WHEN** the profile allows `issue —[depends_on]→ issue` and an issue has an
  outgoing `depends_on` edge but no incoming one
- **THEN** the detail shows one expected-but-missing row, for the incoming side, and
  none for the outgoing side

#### Scenario: Orphan-ok kind
- **WHEN** a node's kind sets `orphan_ok: true` and has no edges
- **THEN** the detail shows no expected-but-missing rows

#### Scenario: Focus switching
- **WHEN** the user presses Tab
- **THEN** focus switches between the node list pane and the detail pane

#### Scenario: Edge follow
- **WHEN** the user presses Enter on an edge row in the detail
- **THEN** the target node's detail is pushed onto the view stack

#### Scenario: Dangling edge target
- **WHEN** the user follows an edge to an undeclared target
- **THEN** the detail shows a VACANCY marker, not a phantom node

### Requirement: Findings list view
Pressing `3` SHALL show all validation findings, sortable by severity
(`o` cycles sort mode). Enter on a finding SHALL jump to the attached
node's detail (if any), and the node list beside it SHALL select that
node's row when a row shows it; `q` from that detail SHALL return to the
findings view with cursor position preserved.

Unattached findings (no `node_id` or `node_id` names no declared node)
SHALL appear in a separate "unattached" group, visibly labeled, after every
attached finding in either sort mode.

A finding that carries a note SHALL show the note text on an indented line under it,
in the findings list and among a node detail's findings, marked expired when the note
has expired.

Pressing `A` on a finding that an entry matches SHALL suspend browse and open `$EDITOR`
on the notes file at the line of that entry, or with no line when the entry has none
(a flow-style list). Pressing `A` on a finding no entry matches
SHALL first show the entry to add (`- finding: {code: <code>, node: <node>}` and
`note:`) on the status line; pressing `A` again, with nothing pressed between, SHALL open
`$EDITOR` on the notes file at its last line. When the editor exits, browse SHALL reload
the profile and the register as `R` does. `A` SHALL instead leave browse up and
say why on the status line when the profile declares no `annotations`, when the notes
file does not exist (naming the resolved path), when the finding has no node, and when
`$EDITOR` is unset. `A` acts on the selected finding in the findings list and on a
finding row in a focused node detail.
Browse SHALL NOT write the notes file.

Suppressed findings SHALL be hidden by default. Pressing `.` in any view
SHALL toggle their visibility, in the findings list and in node detail.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for findings grouping, suppression toggle, note lines and `A`) and demonstration.

#### Scenario: View findings
- **WHEN** the user presses `3`
- **THEN** the findings list appears with findings grouped by severity

#### Scenario: Jump to node from finding
- **WHEN** the user presses Enter on a finding with a node_id
- **THEN** the view navigates to that node's detail, and the node list
  highlights that node's row rather than the row selected before

#### Scenario: Unattached findings group
- **WHEN** findings exist with no declared node
- **THEN** they appear in a separate "unattached" group

#### Scenario: Suppressed findings hidden by default
- **WHEN** suppressed findings exist
- **THEN** they are hidden unless the user toggles them visible

#### Scenario: Noted finding shows its note
- **WHEN** a `VACANCY` finding on `UN-3` carries the note `Lands with M2.`
- **THEN** the findings list shows `Lands with M2.` on an indented line under it

#### Scenario: Edit an existing note
- **WHEN** the user presses `A` on that finding and its entry begins on line 6
- **THEN** `$EDITOR` opens the notes file at line 6, and browse reloads when it exits

#### Scenario: Edit a flow-style note
- **WHEN** the notes file is a flow-style list and the user presses `A` on a finding its
  entry matches
- **THEN** `$EDITOR` opens the notes file with no line argument

#### Scenario: Add a note
- **WHEN** the user presses `A` on a `VACANCY` finding on `UN-3` that no entry matches
- **THEN** the status line shows `- finding: {code: VACANCY, node: UN-3}` and says `A`
  again opens the notes file; a second `A` opens `$EDITOR` at the file's last line

#### Scenario: No notes file declared
- **WHEN** the profile declares no `annotations` and the user presses `A`
- **THEN** browse stays up and the status line says the profile declares no notes file

#### Scenario: No editor configured for a note
- **WHEN** `$EDITOR` is unset and the user presses `A` on a finding
- **THEN** browse stays up and the status line says to set `$EDITOR`

#### Scenario: Notes file missing
- **WHEN** `annotations` names a file that does not exist and the user presses `A`
- **THEN** browse stays up and the status line names the resolved path as missing

### Requirement: View stacking
Enter SHALL push a deeper view onto a navigation stack. `q` SHALL pop
back one level. The user SHALL never lose their place — returning to a
previous view SHALL preserve the cursor position and filter state of
that view.

Number keys (`1`, `2`, `3`, `4`, `5`) SHALL reset the stack to a
top-level view.
`Q` SHALL quit entirely.

In the tree view, Enter pushes node detail for the row under the cursor
(consistent with the node list). Space expands a tree node; `-`
collapses it. These are tree-local operations, not stack operations.

In the Graph view, Enter re-centers rather than pushing detail. Tab pushes
detail from the map. `q` SHALL quit or pop the view stack — it SHALL
NOT pop `center_history`. Center-history navigation uses
`Backspace`/`Esc` instead. The shared view stack is used only for
Tab-pushed detail.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for stack push/pop and state preservation).

#### Scenario: Stack push and pop
- **WHEN** the user navigates node list → node detail → edge follow → `q` → `q`
- **THEN** they return to the node list with cursor position preserved

#### Scenario: Number key resets stack
- **WHEN** the user is three levels deep and presses `1`
- **THEN** the stack resets to the node list top-level view

#### Scenario: Tree enter pushes detail
- **WHEN** the user is in the tree view and presses Enter on a node
- **THEN** the node detail is pushed onto the stack; `q` returns to the tree

#### Scenario: Key 2 resets to tree
- **WHEN** the user is in node detail pushed from the tree and presses `2`
- **THEN** the stack resets to the tree top-level view with its state preserved

#### Scenario: Graph enter re-centers
- **WHEN** the user is in the Graph view and presses Enter on a neighbor
- **THEN** the map re-centers on that neighbor

#### Scenario: Graph Tab pushes detail
- **WHEN** the user is in the Graph view and presses Tab on a node
- **THEN** the node detail pane opens; Tab again closes it

#### Scenario: Graph q does not pop center history
- **WHEN** the user has re-centered in the Graph view and presses `q`
- **THEN** browse quits or pops the view stack; center_history is not popped

### Requirement: Search
`/` SHALL open a regex search prompt. Matches SHALL be highlighted in the
current view. `n` and `N` SHALL cycle forward and backward through
matches. Search SHALL highlight, not reduce the visible set.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for match cycling).

#### Scenario: Search highlights matches
- **WHEN** the user types `/REQ` and presses Enter
- **THEN** all items matching "REQ" are highlighted and the cursor moves
  to the first match

#### Scenario: Cycle matches
- **WHEN** matches exist and the user presses `n`
- **THEN** the cursor moves to the next match

### Requirement: Status line
The bottom line SHALL show the current view name, node/edge counts,
active filters, and a finding summary of the unsuppressed findings: the error
and warning counts always, then the info and hint counts when either is nonzero
(e.g. "3 errors, 12 warnings, 155 hints").

Verified by: demonstration, and `cargo test -p latt --locked` for the hint count.

#### Scenario: Status line content
- **WHEN** browse is displaying the node list
- **THEN** the bottom line shows view name, counts, active filters, and
  finding summary

### Requirement: Jump to source
When the selected node's kind declares an `edit` action, pressing `e` SHALL
run that action (see the browse-owner-actions capability) and SHALL NOT open
`$EDITOR`. Otherwise, pressing `e` SHALL suspend the TUI, open `$EDITOR` at the provenance
file:line of the selected node, edge, or finding, and resume the TUI
when the editor exits. A jump that cannot happen SHALL say why on the status
line and not suspend: when `$EDITOR` is unset, and when the provenance, resolved
against the target directory, is not a file in the working tree (a path on a git
branch, an API locator), and when the selected finding is about an input as a whole
(its origin is the profile, a fuse profile or source, or a suggestion document), naming
the finding and the input. Browse SHALL never hand the editor a path that does not
exist.

Verified by: demonstration, and `cargo test -p latt --locked`
(`jump_to_source` unit tests, `e_runs_a_declared_edit_action`,
`e_without_an_edit_action_keeps_the_editor`,
`jump_and_preview_on_a_profile_finding_say_it_has_no_file`).

#### Scenario: Open editor
- **WHEN** the user presses `e` on a node with file provenance
- **THEN** the TUI suspends, `$EDITOR` opens at that file:line, and the
  TUI resumes after the editor exits

#### Scenario: No editor configured
- **WHEN** `$EDITOR` is unset and the user presses `e`
- **THEN** the TUI stays up and the status line says to set `$EDITOR`

#### Scenario: Provenance is not a work-tree file
- **WHEN** the user presses `e` on a node whose provenance is `entomologist-data:5a3f`
- **THEN** the TUI stays up and the status line names the provenance and says it is
  not a file in the working tree

#### Scenario: A finding about the profile has no file to open
- **WHEN** the user presses `e` or `p` on a `SUPPRESS_UNUSED` finding, which is about the
  profile
- **THEN** the TUI stays up and the status line says `SUPPRESS_UNUSED is about the profile,
  not a line in a file`

#### Scenario: A declared edit action replaces the editor
- **WHEN** the user presses `e` on node `ISS-4` whose kind declares
  `edit: {run: ["ent", "edit", "{id}"], mode: interactive}`
- **THEN** `$EDITOR` is not opened, the first-use confirmation shows
  `ent edit ISS-4`, and after `y` it runs and the register is reloaded when it exits

### Requirement: Input mode indicator
When the user is in a non-Normal input mode, the status bar SHALL
display a mode tag indicating the current mode:
- `FILTER` when in KindFilter or EdgeKindFilter mode, with a distinct
  style (bold or colored background).
- `SEARCH` when in Search mode, with a distinct style.
The mode tag SHALL be visible regardless of which view is active.

Verified by: demonstration (enter filter mode and confirm the mode tag
appears in the status bar).

#### Scenario: Filter mode indicator
- **WHEN** the user presses `F` to enter edge-kind filter mode
- **THEN** the status bar shows a `FILTER` mode tag with distinct styling

#### Scenario: Search mode indicator
- **WHEN** the user presses `/` to enter search mode
- **THEN** the status bar shows a `SEARCH` mode tag with distinct styling

### Requirement: Graph search
`/` SHALL open a regex search prompt in the Graph view. Matches SHALL be
highlighted in the map row list (highlight, not reduce). `n` and `N`
SHALL cycle forward and backward through matches. Additionally, pressing
`&` or `Ctrl-F` SHALL toggle a filter mode that reduces the visible map rows
to only those matching the current search query. The filter mode tag
SHALL be shown in the status bar when active.

Verified by: `cargo test -p latt --locked` (unit tests for
search highlight and filter-reduce in the Graph view) and demonstration.

#### Scenario: Search highlights in map
- **WHEN** the user types `/REQ` and presses Enter in the Graph view
- **THEN** all map rows matching "REQ" are highlighted and the cursor
  moves to the first match

#### Scenario: Filter reduces visible rows
- **WHEN** the user has an active search and presses `&` or Ctrl-F
- **THEN** only matching rows remain visible; the status bar shows a
  filter indicator

#### Scenario: Toggle filter off
- **WHEN** filter mode is active and the user presses `&` or Ctrl-F again
- **THEN** all rows become visible again

### Requirement: Per-view state scoping
Search query, filter set, sort mode, and cursor position SHALL be
independent per top-level view. Switching between views SHALL preserve
each view's state. Drill-down via Enter SHALL push a stack frame that
inherits the parent's filter but gets its own cursor.

The tree view's independent state includes: root node, expanded set,
direction, edge-kind filter, cursor position, and search query. These
SHALL be preserved when switching away and back.

Verified by: `cargo test -p latt --locked` (app-state unit tests
for state isolation).

#### Scenario: Independent view state
- **WHEN** the user sets a filter in the node list, switches to findings
  (`3`), and returns to node list (`1`)
- **THEN** the node list filter is still active

#### Scenario: Tree state preserved across view switches
- **WHEN** the user expands nodes in the tree, switches to node list (`1`),
  and returns to tree (`2`)
- **THEN** the tree's root, expansion, direction, and cursor are preserved

### Requirement: Tab bar
The top line SHALL display a tab bar with all five views — `1 Nodes`,
`2 Tree`, `3 Findings`, `4 Provenance`, `5 Graph` — with the active
view highlighted. The tab bar is always visible.

Verified by: demonstration and `cargo test -p latt --locked`
(VIEWS constant tests).

#### Scenario: Tab bar shows active view
- **WHEN** the user switches to a view via its number key
- **THEN** the tab bar highlights the corresponding tab

### Requirement: Reload
`r` SHALL re-run the adapter and validation against the profile already
loaded; `R` SHALL re-read the profile first, then do the same. Browse SHALL
show `reloading…` before the load starts. On success the status line SHALL
report the node count, the nodes added and removed, and the finding count.
State SHALL carry over by node ID and kind name, with a row index only as
the fallback when a node is gone: the
view, each view's cursor node, the tree root, graph centre and centre
history, the view stack, searches, filters, sort, direction, radius,
columns and toggles. A position whose node the new load no longer declares
SHALL be dropped — a view stack is cut at its first missing node, a cursor
falls back to its old row clamped to the list, and a Tree, Provenance or
Graph view whose root is gone returns to the node list with an empty stack. Tree expansion is
not carried. If the load fails, browse SHALL keep the previous load on
screen and report the failure, with the adapter's error text, on the
status line. The exit code SHALL reflect the last load that succeeded.

Verified by: `cargo test -p latt --locked`
(`reload_*` unit tests in `tui::app` for what carries over; `reload_tests` in
the binary for what `r` and `R` load and for a failing adapter's error) and
demonstration for the event loop's part: `reloading…` drawn first, the
previous load kept on screen after a failure, and the exit code following
the last load that succeeded.

#### Scenario: The cursor follows its node
- **WHEN** the cursor is on a node, a row is added above it, and the user
  presses `r`
- **THEN** the cursor is on the same node, one row lower

#### Scenario: A removed node
- **WHEN** the node under the cursor is deleted from the register and the
  user presses `r`
- **THEN** the cursor stays at its row, clamped to the list, and the detail
  pane for the removed node closes

#### Scenario: A failed reload keeps the previous load
- **WHEN** the adapter fails during a reload
- **THEN** the previous nodes and findings stay on screen and the status
  line reports the failure with the adapter's error text

#### Scenario: Only R reads the profile
- **WHEN** the profile file is edited and the user presses `r`
- **THEN** the loaded profile is unchanged; pressing `R` loads the edited one

### Requirement: Navigation keys
The same keys SHALL move the cursor in every view: `j`/Down and `k`/Up by
one row, PageDown/`Ctrl-D` and PageUp/`Ctrl-U` by a page, `g`/Home to the
first row and `G`/End to the last. They move whichever cursor has focus —
the detail pane when it is focused, the schema rows in schema mode,
otherwise the view's list — and scroll the file preview while it is open.

Verified by: `cargo test -p latt --locked`
(`tui::dispatch_tests` dispatch baseline).

#### Scenario: Top and bottom in any view
- **WHEN** the user presses `G` in the tree view
- **THEN** the cursor moves to the last tree row; `g` returns it to the first

#### Scenario: Vim paging
- **WHEN** the user presses `Ctrl-D` in the node list
- **THEN** the cursor moves down a page, as it does for PageDown

### Requirement: Left/Right navigation consistency
Left and Right arrow keys, and their equals `h` and `l`, SHALL use a
unified model across all views:
- **Right** SHALL go one step deeper: if detail is visible but unfocused,
  focus it; otherwise, act as Enter (open detail, re-center in map, etc.)
- **Left** SHALL go one step back using the same cascade as `q`/`Esc`
  (minus quit): pop the view stack, then unfocus detail, then close
  detail, then fall through to view-specific back (Graph: pop center
  history; Tree: collapse current node; others: no-op).

Verified by: `cargo test -p latt --locked` (Left/Right unit tests).

#### Scenario: Right opens and focuses detail
- **WHEN** the user presses Right in the node list
- **THEN** the detail pane opens; pressing Right again focuses it

#### Scenario: Left closes detail and pops stack
- **WHEN** the user is focused on a detail pane and presses Left
- **THEN** detail unfocuses; pressing Left again closes detail;
  pressing Left again pops the view stack if non-empty

### Requirement: Kind filter
Pressing `f` with the list focused SHALL open a kind-filter picker listing
all profile node kinds with checkboxes. Space SHALL toggle the kind under
the cursor; Enter, Esc and `f` SHALL close the picker without toggling.
Each view maintains its own independent kind-filter state. The kind
filter SHALL appear in the status line as `[-kind,…]` when any kind is
toggled off. In Provenance, in schema mode, and with the detail pane
focused, `f` SHALL open nothing and SHALL say why in the status line.

Verified by: `cargo test -p latt --locked` (kind filter unit
tests per view).

#### Scenario: Enter confirms without toggling
- **WHEN** the user presses `f`, Space on one kind, `j`, Space on the next,
  then Enter
- **THEN** both kinds are hidden and the picker is closed

#### Scenario: A refused kind filter says why
- **WHEN** the user presses `f` with the detail pane focused
- **THEN** no picker opens and the status line says the kind filter
  applies to the list

#### Scenario: Kind filter per view
- **WHEN** the user presses `f` with the list focused, in any view except
  Provenance
- **THEN** the kind-filter picker opens for that view
- **WHEN** the user toggles a kind off and confirms
- **THEN** the view hides items of that kind and the status line shows
  the exclusion label

### Requirement: View key with no node to anchor
The Tree (`2`), Provenance (`4`) and Graph (`5`) views are rooted at a node. When one
of their keys is pressed where no declared node can anchor the view (an empty Findings
list, a finding about no node, or an empty node list), browse SHALL leave the current
view unchanged and SHALL say in the status line that the view needs a selected node.

Verified by: `cargo test -p latt --locked` (app-state unit tests).

#### Scenario: Tree key on an empty Findings view
- **WHEN** the register has no findings, the user is on the Findings view, and presses `2`
- **THEN** the view stays on Findings and the status line says Tree needs a selected node

