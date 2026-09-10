# output Specification

## Purpose
The tri-format output contract — plain, json, and rich — that routes all lattice output
through a single dispatcher, ensuring consistent structure across formats.
## Requirements
### Requirement: Three output formats
Lattice SHALL support `--format=plain|json|rich`. `plain` emits minimal text suitable
for LLM/agent consumption. `json` emits machine-parseable structured output. `rich`
emits human-readable formatted output (colors, tables).

#### Scenario: Plain format
- **WHEN** `lattice validate --format=plain` runs on a graph with one warning
- **THEN** the output is one line per finding: `WARNING ORPHAN_NODE REQS.md:42 node 'REQ-9999' has no edges`

#### Scenario: JSON format
- **WHEN** `lattice validate --format=json` runs on a graph with one warning
- **THEN** the output is a JSON object with a `findings` array, each entry having `severity`, `code`, `file`, `line`, `message`, and `node_id`

#### Scenario: Rich format
- **WHEN** `lattice validate --format=rich` runs with a TTY attached
- **THEN** the output uses colored severity labels, aligned columns, and a summary line

### Requirement: Output dispatcher
All commands SHALL route output through `output_result(data, format)` rather than
printing directly. This ensures format consistency across commands. The dispatcher
SHALL select its formatter from the payload type, so a new payload adds formatters
without adding a printing path outside the dispatcher.

#### Scenario: Dispatcher selects format
- **WHEN** a command produces results and the user passed `--format=json`
- **THEN** `output_result` serializes the data as JSON

#### Scenario: Summary uses the dispatcher
- **WHEN** `lattice summary` renders its rollup in any of the three formats
- **THEN** it does so by calling `output_result`, not by printing directly

#### Scenario: Trace uses the dispatcher
- **WHEN** `lattice trace` renders its trace report in any of the three formats
- **THEN** it does so by calling `output_result`, not by printing directly

#### Scenario: Adapter issues use the dispatcher
- **WHEN** `lattice summary --format=json` runs on a target with adapter issues
- **THEN** the issues are rendered by `output_result` and written to stderr, so stdout
  stays a parseable rollup payload and the issues honour the requested format

### Requirement: Deterministic output ordering
Output SHALL be deterministically ordered, so repeated runs on the same input produce
byte-identical output. Findings SHALL be sorted by provenance file, line, code, node ID,
then message. Payloads that are not findings SHALL declare their own total order; the
status rollup is ordered by group key, with status columns in a fixed order.

#### Scenario: Stable output across runs
- **WHEN** `lattice validate --format=json` runs twice on the same input
- **THEN** both outputs are byte-identical

#### Scenario: Stable summary across runs
- **WHEN** `lattice summary --format=json` runs twice on the same input
- **THEN** both outputs are byte-identical

### Requirement: Trace report plain format
Plain format for a trace report SHALL show one line per node: ID, kind, a key attr
(the profile's `summary_attr` for that kind, or blank when none is declared), edge
target count, and finding count. Findings SHALL be expanded below the node that owns
them. Unattachable findings SHALL appear in a footer section.

#### Scenario: Plain trace with findings
- **WHEN** `lattice trace --format=plain` runs on a graph where `REQ-0101` has one
  `COVERAGE` warning
- **THEN** output shows a line for `REQ-0101` followed by the finding on the next line

#### Scenario: Plain trace unattachable findings
- **WHEN** the trace has unattachable findings
- **THEN** they appear in a footer section after all node entries

### Requirement: Trace report rich format
Rich format for a trace report SHALL show a table with columns for ID, kind, key attr
(the profile's `summary_attr` for that kind, or blank), edges, and findings, with
color-coded severity. Findings SHALL be expanded below the owning node. Unattachable
findings SHALL appear in a footer section.

#### Scenario: Rich trace table
- **WHEN** `lattice trace --format=rich` runs with a TTY attached
- **THEN** output shows a colored table with node rows and inline findings

### Requirement: Trace report JSON format
JSON format for a trace report SHALL output the full `TraceReport` structure: header,
entries array, unattachable findings array, and pathways array. The header SHALL carry `trace_version`
`"2"`. Each entry's `edges` SHALL be an array of objects carrying `tgt`, `kind`,
`attrs`, and nested `provenance` with `file` and `line`.

#### Scenario: JSON trace is parseable
- **WHEN** `lattice trace --format=json` runs
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
- **WHEN** a node's status attr value is boolean true and `lattice summary`
  runs with `--format json`
- **THEN** the rollup counts it under the key `true`

### Requirement: Hint severity rendering
All three formats SHALL render hint-severity findings. `plain` labels the line `HINT`,
in the same one-line shape as the other severities. `json` carries `"severity": "hint"`.
`rich` renders a hint with its own label and a colour distinct from info. A hint is
part of the report wherever findings render — dropping it in any format would make the
formats diverge on what was found.

Verified by: `cargo test --test output hint`

#### Scenario: Plain hint line
- **WHEN** `lattice validate --format=plain` reports a hint finding
- **THEN** the line reads `HINT <CODE> <file>:<line> <message>`

#### Scenario: JSON hint severity
- **WHEN** `lattice validate --format=json` reports a hint finding
- **THEN** the findings entry carries `"severity": "hint"`

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
- **WHEN** an `ORPHAN_NODE` finding renders as JSON
- **THEN** the entry has no `state` key and is byte-identical to the pre-change shape

### Requirement: Suggestion rendering
A rendered suggestion SHALL be a finding with code `SUGGESTED_EDGE` at `hint` severity, routed
through the output dispatcher like every other finding, so all three of `plain|json|rich`
carry it. Its `node_id` SHALL be the suggestion's `src` and its provenance SHALL be that
node's provenance, so the reviewer is sent to the file and line they would edit to act on it.

Its message SHALL name the proposed edge kind, the target ID, the score and the producer, and
SHALL carry the `basis` text verbatim. A suggestion the reviewer cannot trace back to a
producer and a rationale is not reviewable, and a ranked list nobody can audit is the kind of
authority this tool refuses to claim.

The core SHALL NOT add a field to the findings JSON for suggestions. An overlay that widened
the findings schema would make every consumer of that schema care about a feature that is
optional, advisory and off by default.

Verified by: `cargo test --test output suggestion`

#### Scenario: Plain suggestion line
- **WHEN** `lattice validate --format=plain --suggestions s.json` renders a suggestion
- **THEN** the line reads as a `HINT SUGGESTED_EDGE` finding at the source node's file and
  line, naming the edge kind, the target ID, the score, the producer and the basis

#### Scenario: JSON suggestion carries no new fields
- **WHEN** `lattice validate --format=json --suggestions s.json` renders a suggestion
- **THEN** the findings entry has the same field set as any other finding, with
  `"severity": "hint"` and `"code": "SUGGESTED_EDGE"`

#### Scenario: Rich format renders it as a hint
- **WHEN** `lattice validate --format=rich --suggestions s.json` renders a suggestion
- **THEN** it appears with the hint label and colour, not with a presentation of its own
