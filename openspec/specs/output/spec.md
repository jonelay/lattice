# output Specification

## Purpose
The tri-format output contract — plain, json, and rich — that routes all lattice output
through a single dispatcher, ensuring consistent structure across formats.
## Requirements
### Requirement: Three output formats
Lattice SHALL support `--format=plain|json|rich`. `plain` emits minimal text suitable
for LLM/agent consumption. `json` emits machine-parseable structured output. `rich`
emits human-readable formatted output (colors, tables).

Whether `rich` carries escape sequences on a stream SHALL be decided in this order:
1. `NO_COLOR` set to a non-empty value: no escapes.
2. `FORCE_COLOR` set to a non-empty value: escapes, even on a stream that is not
   a terminal.
3. `TERM=dumb`: no escapes.
4. Otherwise: escapes only on a stream that is a terminal.

None of these variables SHALL change which format is chosen by default.
`FORCE_COLOR` changes output only under `TERM=dumb` or when `rich` is chosen
explicitly for a stream that is not a terminal, whose default format is `plain`.

Verified by: `cargo test -p latt --bin latt color_tests`,
`cargo test --test cli rich_output_carries_no_ansi`,
`cargo test --test cli rich_output_forced_color` (every subprocess test sets
`NO_COLOR`, `FORCE_COLOR` and `TERM` explicitly), and demonstration (run
`latt validate --format rich` under `script` with and without `NO_COLOR=1`,
and with `TERM=dumb`: SGR color codes appear only in the plain run).

#### Scenario: Plain format
- **WHEN** `latt validate --format=plain` runs on a graph with one warning
- **THEN** the output is one line per finding: `WARNING UNREFERENCED REQS.md:42 node 'REQ-9999' has no incoming edges`

#### Scenario: JSON format
- **WHEN** `latt validate --format=json` runs on a graph with one warning
- **THEN** the output is a JSON object with a `findings` array, each entry having `severity`, `code`, `file`, `line`, `message`, and `node_id`

#### Scenario: Rich format
- **WHEN** `latt validate --format=rich` runs with a TTY attached
- **THEN** the output uses colored severity labels, aligned columns, and a summary line

#### Scenario: Rich format under NO_COLOR
- **WHEN** `latt validate --format=rich` runs with a TTY attached and `NO_COLOR=1`
- **THEN** the output keeps its columns, severity words and summary line and carries no escape sequences

#### Scenario: Rich format on a dumb terminal
- **WHEN** `latt validate --format=rich` runs with a TTY attached, `TERM=dumb`,
  and `FORCE_COLOR` unset
- **THEN** the output carries no escape sequences

#### Scenario: FORCE_COLOR through a pipe
- **WHEN** `latt validate --format=rich` writes to a pipe with `FORCE_COLOR=1`
  and `NO_COLOR` unset
- **THEN** the output carries its color escape sequences

#### Scenario: NO_COLOR beats FORCE_COLOR
- **WHEN** `latt validate --format=rich` runs with both `NO_COLOR=1` and `FORCE_COLOR=1`
- **THEN** the output carries no escape sequences

### Requirement: Format compatibility
`json` SHALL be the machine contract: across releases it changes only as the
versioning policy allows, so an agent or script that parses lattice output SHALL be
pointed at `--format json`. `plain` and `rich` SHALL be stable in meaning only. A
patch release MAY reword, reorder or realign them, as long as they report the same
findings, nodes and counts. Within one version all three formats SHALL be
repeatable: the same input gives byte-identical output. The byte baselines pin that
repeatability as regression instruments. They are not a compatibility promise for
`plain` or `rich`.

Verified by: inspection of this spec and `README.md` (a parser is pointed at `json`), and
the byte baselines (`cargo test -p latt-core --test trace_baseline --test fuse_baseline`).

#### Scenario: A consumer parses output
- **WHEN** a script needs output that survives a patch upgrade
- **THEN** it reads `--format json`, and `plain` or `rich` text carries no such promise

#### Scenario: Plain text changes in a patch release
- **WHEN** a patch release rewords a `plain` line without changing what it reports
- **THEN** that is not a breaking change, and the byte baseline is regenerated and reviewed

### Requirement: Map-shaped JSON objects sort their keys
In `json` output, node and edge attrs, and any object nested in an attr value, SHALL
serialize with keys in ascending byte order, whatever `serde_json` features the
embedding host builds `latt-core` with. Struct-shaped objects SHALL keep their
pinned field order, which is not alphabetical everywhere. In `plain` and `rich` an
object-valued attr, and the `browse` detail pane, follow the map's iteration order,
which is not part of any contract.

Verified by: `cargo test -p latt-core --features serde_json/preserve_order --locked`
(the `output::sorted` unit tests and the byte baselines, run with the feature on).

#### Scenario: A host unifies preserve_order
- **WHEN** `latt-core` is built with `serde_json/preserve_order` and renders attrs
  inserted out of order, including an object nested in an attr value
- **THEN** `--format json` output is byte-identical to the default build's

### Requirement: Output dispatcher
All batch commands SHALL route output through `output_result(data, format)` rather than
printing directly; `browse` owns its terminal and `show` prints the profile document
verbatim (the cli capability). This ensures format consistency across commands. The dispatcher
SHALL select its formatter from the payload type, so a new payload adds formatters
without adding a printing path outside the dispatcher.

#### Scenario: Dispatcher selects format
- **WHEN** a command produces results and the user passed `--format=json`
- **THEN** `output_result` serializes the data as JSON

#### Scenario: Summary uses the dispatcher
- **WHEN** `latt summary` renders its configured rollup or its zero-config
  structural report in any of the three formats
- **THEN** it does so by calling `output_result`, not by printing directly

#### Scenario: Trace uses the dispatcher
- **WHEN** `latt trace` renders its trace report in any of the three formats
- **THEN** it does so by calling `output_result`, not by printing directly

#### Scenario: Coverage uses the dispatcher
- **WHEN** `latt coverage` renders its report in any of the three formats
- **THEN** it does so by calling `output_result`, not by printing directly

#### Scenario: Adapter issues use the dispatcher
- **WHEN** `latt summary --format=json` runs on a target with adapter issues
- **THEN** the issues are rendered by `output_result` and written to stderr, so stdout
  stays a parseable rollup payload and the issues honour the requested format

### Requirement: Deterministic output ordering
Output SHALL be deterministically ordered, so repeated runs on the same input produce
byte-identical output. Findings SHALL be sorted by provenance file, line, code, node ID,
then message. Payloads that are not findings SHALL declare their own total order; the
status rollup is ordered by group key, with status columns in a fixed order.

#### Scenario: Stable output across runs
- **WHEN** `latt validate --format=json` runs twice on the same input
- **THEN** both outputs are byte-identical

#### Scenario: Stable summary across runs
- **WHEN** `latt summary --format=json` runs twice on the same input
- **THEN** both outputs are byte-identical

### Requirement: Trace report plain format
Plain format for a trace report SHALL show one line per node, every field labelled as
`name:value` so the line reads in any order: `id:<id> kind:<kind> edges:<n>
findings:<n>`, where `edges` is the edge target count and `findings` the visible finding
count. When the kind declares a `summary_attr` and the node carries it, the line SHALL
end with ` <summary_attr>:<value>`, labelled with the attr's own name and carrying the
whole value with each whitespace run folded to one space, so a multi-line value stays
on the node's line; otherwise the field is absent. The line SHALL carry no padding. Findings
SHALL be expanded below the node that owns them. Unattachable findings SHALL appear in a
footer section. `query at` SHALL render its entries with the same line.

Verified by: `cargo test --test output plain_trace`,
`cargo test --test trace_baseline`

#### Scenario: Plain trace with findings
- **WHEN** `latt trace --format=plain` runs on a graph where `REQ-0101` has one
  `COVERAGE` warning
- **THEN** output shows a line beginning `id:REQ-0101 ` followed by the finding on the
  next line

#### Scenario: Plain trace labels the summary attr
- **WHEN** kind `widget` declares `summary_attr: label` and `W-1` has one edge, no
  findings and `label` "A gizmo whose label runs past thirty characters"
- **THEN** its line is `id:W-1 kind:widget edges:1 findings:0 label:A gizmo whose label
  runs past thirty characters`

#### Scenario: Plain trace without a summary attr
- **WHEN** a node's kind declares no `summary_attr`
- **THEN** its line ends at `findings:<n>`

#### Scenario: Plain trace unattachable findings
- **WHEN** the trace has unattachable findings
- **THEN** they appear in a footer section after all node entries

### Requirement: Trace report rich format
Rich format for a trace report SHALL show a table with columns for ID, kind, key attr
(the profile's `summary_attr` for that kind, or blank), edges, and findings, with
color-coded severity. Findings SHALL be expanded below the owning node. Unattachable
findings SHALL appear in a footer section.

#### Scenario: Rich trace table
- **WHEN** `latt trace --format=rich` runs with a TTY attached
- **THEN** output shows a colored table with node rows and inline findings

### Requirement: Trace report JSON format
JSON format for a trace report SHALL output the full `TraceReport` structure: header,
entries array, unattachable findings array, and pathways array. The header SHALL carry `trace_version`
`"2"`. Each entry's `edges` SHALL be an array of objects carrying `tgt`, `kind`,
`attrs`, and nested `provenance` with `file` and `line`.

#### Scenario: JSON trace is parseable
- **WHEN** `latt trace --format=json` runs
- **THEN** stdout is valid JSON containing `header`, `entries`, `unattachable_findings`, and `pathways`,
  with `header.trace_version` equal to `"2"`

### Requirement: Key attr scalars render as JSON scalars
In the trace report's key-attr column, a string value SHALL render bare (no
quotes) and a non-string scalar SHALL render as its JSON form (`true`, `false`,
`null`, the number's JSON rendering) — never Python's `True`/`False`/`None`.

Verified by: `cargo test --test native_messages`

#### Scenario: Boolean summary attr
- **WHEN** a node's `summary_attr` value is boolean true
- **THEN** the trace row's key-attr column shows `true`

### Requirement: Summary rollup keys are JSON scalars
In the summary rollup, a non-string status or group attr value SHALL key its
row or column by its JSON form (`true`, `false`, `null`, a number's JSON
rendering), never Python's `True`/`False`/`None`. A node without the attr keys
as `unknown`, unchanged. This governs the JSON payload's keys as well as the
rendered formats.

Verified by: `cargo test --test native_messages summary_rollup`

#### Scenario: Boolean status attr keys the rollup as JSON
- **WHEN** a node's status attr value is boolean true and `latt summary`
  runs with `--format json`
- **THEN** the rollup counts it under the key `true`

### Requirement: Suppressed finding rendering
A suppressed finding SHALL render in `json` with `"suppressed": true` and every
other field it would carry unsuppressed — severity, code, message, file, line,
node_id, and state where present. An unsuppressed finding SHALL omit the
`suppressed` key entirely, so every finding that was never suppressed
serializes byte-identically to before the field existed.

`plain` and `rich` SHALL omit suppressed findings.

Suppressed findings SHALL keep their place in the deterministic ordering of the
JSON `findings` array — they are not moved to a separate section.

Verified by: `cargo test --test output suppressed`

#### Scenario: JSON retains a suppressed finding
- **WHEN** `latt validate --format=json` runs and a `VACANCY` finding for
  `UN-1` is suppressed
- **THEN** the `findings` array contains that entry with `"suppressed": true`
  and its `severity`, `code`, `file`, `line`, `message`, and `node_id` intact

#### Scenario: Unsuppressed finding carries no suppressed key
- **WHEN** an `UNREFERENCED` finding that no suppress entry matches renders as JSON
- **THEN** the entry has no `suppressed` key and is byte-identical to the
  pre-change shape

#### Scenario: Plain omits a suppressed finding
- **WHEN** `latt validate --format=plain` runs and one of two findings is
  suppressed
- **THEN** the output is one line, for the unsuppressed finding

#### Scenario: Rich omits a suppressed finding
- **WHEN** `latt validate --format=rich` runs and one of two findings is
  suppressed
- **THEN** the table shows the unsuppressed finding only, and the summary line
  counts one finding

### Requirement: Hint severity rendering
All three formats SHALL render hint-severity findings. `plain` labels the line `HINT`,
in the same one-line shape as the other severities. `json` carries `"severity": "hint"`.
`rich` renders a hint with its own label and a colour distinct from info. A hint is
part of the report wherever findings render — dropping it in any format would make the
formats diverge on what was found. Suppression is the sole exception to that rule, and
it is a declared one: a suppressed hint is omitted from `plain` and `rich` like any
other suppressed finding, and stays in `json` with `suppressed: true` (see the
suppressed-finding rendering requirement).

Verified by: `cargo test --test output hint`

#### Scenario: Plain hint line
- **WHEN** `latt validate --format=plain` reports a hint finding
- **THEN** the line reads `HINT <CODE> <file>:<line> <message>`

#### Scenario: JSON hint severity
- **WHEN** `latt validate --format=json` reports a hint finding
- **THEN** the findings entry carries `"severity": "hint"`

#### Scenario: Suppressed hint follows the suppression rule
- **WHEN** a hint-severity finding is suppressed by the profile
- **THEN** `plain` and `rich` omit it and `json` carries it with
  `"severity": "hint"` and `"suppressed": true`

### Requirement: Findings JSON carries state when present
A finding that carries a `state` SHALL expose it in the JSON findings entry as a
`state` string. A finding without one SHALL omit the key entirely, so every finding
that never had a state serializes byte-identically to before the field existed. The
plain and rich formats SHALL NOT change for state-carrying findings — the message text
is the human surface, and the state qualifies rather than replaces it.

Verified by: `cargo test --test output state`

#### Scenario: State serialized
- **WHEN** a `COVERAGE` finding with `state: "unknown"` renders as JSON
- **THEN** the entry carries `"state": "unknown"`

#### Scenario: Stateless finding unchanged
- **WHEN** an `UNREFERENCED` finding renders as JSON
- **THEN** the entry has no `state` key and is byte-identical to the pre-change shape

### Requirement: Suggestion rendering
A rendered suggestion SHALL be a finding with code `SUGGESTED_EDGE` at `hint` severity, routed
through the output dispatcher like every other finding, so all three of `plain|json|rich`
carry it. Its `node_id` SHALL be the suggestion's `src` and its provenance SHALL be that
node's provenance, so the reviewer is sent to the file and line they would edit to act on it.

Its message SHALL name the proposed edge kind, the target ID, the score and the producer, and
SHALL carry the `basis` text verbatim.

The core SHALL NOT add a field to the findings JSON for suggestions. An overlay that widened
the findings schema would make every consumer of that schema care about a feature that is
optional, advisory and off by default.

Verified by: `cargo test --test output suggestion`

#### Scenario: Plain suggestion line
- **WHEN** `latt validate --format=plain --suggestions s.json` renders a suggestion
- **THEN** the line reads as a `HINT SUGGESTED_EDGE` finding at the source node's file and
  line, naming the edge kind, the target ID, the score, the producer and the basis

#### Scenario: JSON suggestion carries no new fields
- **WHEN** `latt validate --format=json --suggestions s.json` renders a suggestion
- **THEN** the findings entry has the same field set as any other finding, with
  `"severity": "hint"` and `"code": "SUGGESTED_EDGE"`

#### Scenario: Rich format renders it as a hint
- **WHEN** `latt validate --format=rich --suggestions s.json` renders a suggestion
- **THEN** it appears with the hint label and colour, not with a presentation of its own

### Requirement: Noted finding rendering
A finding that carries a note SHALL expose it in its JSON findings entry as a `note`
object with `text`, and `until`, `by` and `at` when the entry declares them, `until`
being the date string or the `{pathway, position}` object as written. The object SHALL
carry `expired: true` when the note has expired and omit `expired` otherwise. A finding
without a note SHALL omit the `note` key, so output from a profile without notes is
byte-identical to before the key existed.

The plain format SHALL keep one line per finding: a noted finding's line SHALL end with
` | note: ` and the note text. The rich format SHALL print the note text on an indented
line directly under its finding, prefixed `note:`. In both, `(expired)` follows the
prefix when the note has expired, and `by` and `until` follow the text in brackets when
declared. Suppressed findings stay out of the plain and rich formats, and so do their
notes.

Verified by: `cargo test -p latt-core --locked --test output note`.

#### Scenario: Note serialized
- **WHEN** a `VACANCY` finding carries a note with text `Lands with M2.` and
  `until: 2026-12-31`
- **THEN** its JSON entry carries
  `"note": {"text": "Lands with M2.", "until": "2026-12-31"}`

#### Scenario: Expired note serialized
- **WHEN** the same note has expired
- **THEN** the `note` object also carries `"expired": true`

#### Scenario: Un-noted finding unchanged
- **WHEN** an `UNREFERENCED` finding with no note renders as JSON
- **THEN** the entry has no `note` key and is byte-identical to the pre-change shape

#### Scenario: Plain note line
- **WHEN** the same `VACANCY` finding renders as plain, with `by: jone`
- **THEN** its one line ends `| note: Lands with M2. [jone, until 2026-12-31]`

#### Scenario: Rich note line
- **WHEN** the same finding renders as rich
- **THEN** the line after the finding reads `  note: Lands with M2. [jone, until 2026-12-31]`

### Requirement: Validate reports its scope
`validate` SHALL report what the graph it checked holds, so a clean run that read
nothing reads differently from a clean run over a populated register.

- In `--format json`, the output object SHALL carry a `scope` key beside `findings`:
  `scope.nodes` maps each node kind the profile declares to its count of declared nodes,
  `scope.edges` maps each edge kind the profile declares to its count of edges, and
  `scope.files` is the number of distinct source files named by the provenance of those
  nodes and edges. A declared kind with no instance SHALL appear with 0. Its maps sort
  their keys, as every map-shaped JSON object does.
- In `--format plain` and `--format rich`, `validate` SHALL write one line to stderr after
  the findings: `scope: <files> files; nodes <kind> <n>, …; edges <kind> <n>, …`, kinds
  in name order. Stdout is unchanged, so a clean `plain` run still writes
  nothing to stdout.
- The scope SHALL leave the exit code alone.

Verified by: `cargo test -p latt --locked --test cli validate_scope`, and
`.venv/bin/python -m pytest tests/test_docs_json.py`.

#### Scenario: Scope over a register that read no tests
- **WHEN** the profile declares node kinds `requirement` and `test` and edge kind
  `verifies`, the adapter reads three requirements from one file and nothing else, and
  `latt validate --format json` runs
- **THEN** `scope` is `{"edges": {"verifies": 0}, "files": 1, "nodes": {"requirement": 3, "test": 0}}`

#### Scenario: A clean plain run
- **WHEN** `latt validate --format plain` finds nothing over a register of 6 nodes in 2
  files
- **THEN** stdout is empty, stderr is one `scope:` line, and the exit code is 0
