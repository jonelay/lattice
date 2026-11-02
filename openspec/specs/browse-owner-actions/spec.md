# browse-owner-actions Specification

## Purpose

Lets a browse user hand the selected node to the register's owner tool (`ent`, `gh`,
`glab`, or any program a profile names) without lattice writing anything itself.

## Requirements

### Requirement: Selected node for an action
An action SHALL run against the selected node. In any view whose cursor row is a node
(the node list, tree, provenance and graph views), that is the node under the cursor. In a detail pane it is the node the pane
shows. In the findings list it is the finding's node. When nothing is selected, or the
selected finding has no node, an action key SHALL leave browse as it is and say on the
status line that no node is selected.

Verified by: `cargo test -p latt --locked owner_action`.

#### Scenario: A finding without a node
- **WHEN** the user presses `c` on a `PARSE_ERROR` finding that carries no node ID
- **THEN** nothing runs and the status line says no node is selected

#### Scenario: A finding with a node
- **WHEN** the user presses `c` on a `VACANCY` finding for node `UN-3`, whose kind
  declares `comment`
- **THEN** the `comment` action runs against `UN-3`

### Requirement: Argument resolution
Browse SHALL build an action's argv by replacing each placeholder in each `run` argument
with the selected node's value: `{id}` the node ID, `{kind}` its kind,
`{attrs.<name>}` the attr's value, `{provenance.file}` and `{provenance.line}` its
provenance. A placeholder may sit inside a longer argument. A string attr is substituted
as written. An int or bool is substituted as its decimal or `true`/`false` text. `{{` and
`}}` stand for literal braces. Each `run` element SHALL stay exactly one argument: browse
does not split, glob or expand a substituted value, and does not invoke a shell.

The launch SHALL be refused, with a status-line message naming the action and the reason:
when a placeholder's attr is absent or null on the node; when the attr is a list; when
`{provenance.line}` is requested and the line is unknown (0).

Verified by: `cargo test -p latt --locked owner_action::resolve`.

#### Scenario: An attr inside a longer argument
- **WHEN** the action is `["gh", "issue", "view", "--repo={attrs.repo}", "{attrs.number}"]`
  and the node has `repo: a/b` and `number: 12`
- **THEN** the argv is `gh issue view --repo=a/b 12`

#### Scenario: A value with spaces stays one argument
- **WHEN** `{attrs.title}` resolves to `fix the parser`
- **THEN** the program receives `fix the parser` as one argument

#### Scenario: A missing attr refuses the launch
- **WHEN** the selected node has no `number` attr and the action uses `{attrs.number}`
- **THEN** nothing runs and the status line names the action and `number`

### Requirement: Leading-dash guard
An argument that begins with a placeholder, and whose resolved text begins with `-`,
SHALL refuse the launch unless an earlier element of the same `run` array is exactly
`--`. The status line SHALL name the action and say the value would be read as an
option. An argument that begins with literal template text is never refused, because the
profile wrote its start: a value inside `--repo={attrs.repo}` cannot become an option.

Verified by: `cargo test -p latt --locked owner_action::guard`.

#### Scenario: An ID that looks like an option
- **WHEN** the action is `["ent", "show", "{id}"]` and the node ID is `--exec=x`
- **THEN** nothing runs and the status line says the value would be read as an option

#### Scenario: A value inside a literal option
- **WHEN** the action is `["gh", "--repo={attrs.repo}"]` and `repo` is `-x`
- **THEN** the argv is `gh --repo=-x`

#### Scenario: The template ends options first
- **WHEN** the action is `["ent", "show", "--", "{id}"]` and the node ID is `-1`
- **THEN** the argv is `ent show -- -1`

### Requirement: Program lookup before launch
Before it suspends or spawns anything, browse SHALL check that the program can be found.
A bare name is looked up on `PATH`. A name containing `/` is resolved against the target
directory. When the program cannot be found, browse SHALL stay up and say so on the
status line. The terminal SHALL NOT be suspended.

Verified by: `cargo test -p latt --locked owner_action::lookup`.

#### Scenario: Tool not installed
- **WHEN** the action's program is `ent` and no `ent` is on `PATH`
- **THEN** the TUI stays up and the status line says `ent` was not found

### Requirement: Action keys
In normal mode, `c`, `s`, `a`, `t` and `e` SHALL run the selected node kind's `comment`,
`state`, `assign`, `tag` and `edit` action respectively. For `c`, `s`, `a` and `t`, a
kind that declares no such verb SHALL leave browse as it is and say on the status line
which verb the kind lacks. The rows SHALL be in the key table, so help and the footer list
them. Actions SHALL run only on a key press, subject to first-use confirmation.
Loading, reloading and moving the cursor never run one.

Verified by: `cargo test -p latt --locked` (`tui::dispatch_tests`
baselines; `owner_action::keys`).

#### Scenario: Undeclared verb
- **WHEN** the user presses `s` on a node whose kind declares no `state` action
- **THEN** nothing runs and the status line names the kind and `state`

#### Scenario: Reload does not run an action
- **WHEN** the user presses `r` with a profile that declares actions
- **THEN** no action program is started

### Requirement: First-use confirmation
The first time in a browse session that a letter key would run a given action, browse
SHALL show the resolved argv and ask for confirmation instead of running it. `y` SHALL
run the action and mark it confirmed. `Q` and `Ctrl-C` quit browse. `Ctrl-F` keeps its
meaning in every mode and leaves the prompt open. Any other key SHALL
cancel without running anything. An action is identified by its node kind, verb and
`run` template, so a later run against another node of the kind is not asked again,
but a `run` template changed by `R` is. Confirmations SHALL survive `r` and `R` for the
rest of the session and SHALL NOT be stored anywhere. Running an action from the `!` menu
SHALL NOT ask, and SHALL mark the action confirmed, because the menu row already showed
the argv.

Refusals (missing placeholder value, leading-dash guard, program not found) SHALL be
reported before the confirmation, so the user is never asked to approve a launch that
cannot happen.

Verified by: `cargo test -p latt --locked owner_action::confirm`.

#### Scenario: First press asks
- **WHEN** the user presses `c` on `ISS-4` for the first time in the session, and the
  kind declares `comment: {run: ["ent", "comment", "{id}"], mode: interactive}`
- **THEN** nothing runs and browse asks to confirm `ent comment ISS-4`

#### Scenario: Confirmed action runs directly afterwards
- **WHEN** the user has confirmed `comment` with `y` and then presses `c` on `ISS-7`
- **THEN** `ent comment ISS-7` runs without asking

#### Scenario: Any other key cancels
- **WHEN** browse asks to confirm an action and the user presses `n`
- **THEN** nothing runs, and the next `c` asks again

#### Scenario: The menu confirms
- **WHEN** the user runs `comment` from the `!` menu and then presses `c`
- **THEN** `comment` runs without asking

#### Scenario: A changed template asks again
- **WHEN** the user confirmed `comment`, edits the profile's `comment` `run`, and presses `R`
- **THEN** the next `c` asks to confirm the new argv

### Requirement: Launcher menu
`!` SHALL open a menu of every action the selected node's kind declares, in declared
order. Each row SHALL show the verb and either its resolved argv or the reason it cannot
be resolved. `Enter` SHALL run the row under the cursor, and the refusals for a
row that cannot be resolved are the same as for its key. `Esc`, `q` and `!` SHALL close
the menu without running anything. A kind with no actions SHALL get a status-line message
instead of an empty menu.

Verified by: `cargo test -p latt --locked owner_action::menu` and
the `tui::dispatch_tests` baselines.

#### Scenario: Menu shows what will run
- **WHEN** the user presses `!` on node `ISS-4` whose kind declares
  `show: {run: ["ent", "show", "{id}"], mode: capture}`
- **THEN** the menu lists `show` with `ent show ISS-4`

#### Scenario: Menu closes without running
- **WHEN** the menu is open and the user presses `Esc`
- **THEN** the menu closes and nothing runs

### Requirement: Interactive mode
An `interactive` action SHALL restore the terminal and run the program in the target
directory with browse's stdin, stdout and stderr. When the program exits, browse SHALL
re-enter, show the exit code on the status line, and reload the register as `r` does,
whatever the exit code. Signals during the run SHALL be handled as for `$EDITOR`.

Verified by: demonstration under `script`, and `cargo test -p latt-core
--all-features --locked --test cli` (`an_interactive_action_reloads_after_it_exits`;
Linux).

#### Scenario: Comment then reload
- **WHEN** the user presses `c` on an ent issue whose kind declares
  `comment: {run: ["ent", "comment", "{id}"], mode: interactive}`
- **THEN** `ent comment <id>` owns the terminal; when it exits browse resumes, the
  status line shows the exit code, and the register is reloaded

### Requirement: Capture mode
A `capture` action SHALL run the program in the target directory with stdin closed. It
captures stdout and stderr, and browse draws `running <verb>…` while it waits. When the
program exits, an output pane SHALL show stdout, then stderr under its own heading, and
the exit code. The pane scrolls with the navigation keys and closes with `Esc` or `q`.
Captured output beyond 1 MiB per stream SHALL be cut and the pane SHALL say it was cut.
`Esc` while the program runs SHALL terminate the program, and the pane SHALL say it was
stopped. `Ctrl-C` and `Q` keep their meaning and quit browse: the program is terminated
first, and is never left running. A capture action does not reload.

Verified by: `cargo test -p latt --locked owner_action::capture`
(the pane, its scrolling, the 1 MiB cut, the command's directory and arguments) and
`cargo test -p latt --locked --test cli` (`a_capture_shows_the_program_output`,
`esc_stops_a_running_capture`, `ctrl_c_during_a_capture_quits_and_stops_the_program`; Linux).

#### Scenario: Viewer output
- **WHEN** the user runs a `capture` action whose program prints 200 lines and exits 0
- **THEN** a pane shows the 200 lines with exit code 0 and scrolls with `j`/`k`

#### Scenario: Stopping a slow tool
- **WHEN** a `capture` action is still running and the user presses `Esc`
- **THEN** the program is terminated and the pane says it was stopped

#### Scenario: Failure shows stderr
- **WHEN** a `capture` action exits 1 after writing `not found` to stderr
- **THEN** the pane shows exit code 1 and `not found` under the stderr heading

### Requirement: Detached mode
A `detached` action SHALL start the program in the target directory with null stdin,
stdout and stderr and SHALL NOT wait for it. The status line reports that it started.
The program SHALL NOT be left as a zombie for the rest of the session.

Verified by: `cargo test -p latt --locked owner_action::detached`
(on Linux, `/proc` shows the exited program is not left a zombie).

#### Scenario: Open in browser
- **WHEN** the user runs a `detached` action `web` as `gh issue view 12 --web`
- **THEN** browse stays up without suspending, and the status line reads `web started (gh)`
