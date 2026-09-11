# coverage-report Specification

## Purpose

Graph-derived coverage statistics by node kind: how many nodes of each kind
exist, how many have incoming or outgoing edges, and what percentage each
represents. The report a consumer would otherwise build from trace JSON.

## Requirements

### Requirement: Coverage report computation
`lattice coverage` SHALL compute, per node kind, the following statistics
from the ingested graph:

- `total`: count of explicitly added nodes of that kind
- `incoming`: count of those nodes with at least one incoming edge
- `outgoing`: count of those nodes with at least one outgoing edge
- `incoming_pct`: `incoming / total * 100`, rounded to one decimal place
- `outgoing_pct`: `outgoing / total * 100`, rounded to one decimal place

The kinds reported follow `query counts`: every kind the profile declares,
plus any kind the register carries undeclared, ordered by kind name. A
declared kind with zero nodes SHALL be included with all counts at zero and
both percentages at `0.0`.

An edge counts for a node whenever it names that node's ID, on the same terms
as the `orphans` query and the `UNREFERENCED`/`UNTRACED` findings: whether
the far endpoint resolves is `VACANCY`'s business. Edge endpoints that were
never explicitly added as nodes (dangling references) SHALL NOT count toward
any kind's `total` — they are not nodes of any kind.

Verified by: `cargo test --test coverage_report`

#### Scenario: Basic coverage statistics
- **WHEN** the graph has 4 `req` nodes, 2 with incoming edges and 2 with outgoing edges
- **THEN** the report shows `req`: total=4, incoming=2 (50.0%), outgoing=2 (50.0%)

#### Scenario: Kind with zero nodes
- **WHEN** the profile declares kind `risk` but no `risk` nodes exist
- **THEN** the report includes `risk` with total=0, incoming=0 (0.0%), outgoing=0 (0.0%)

#### Scenario: Dangling references excluded
- **WHEN** an edge from `REQ-3` targets node ID `REQ-9` but `REQ-9` was never explicitly added
- **THEN** `REQ-9` does not count toward any kind's total, and `REQ-3` still counts as having an outgoing edge

#### Scenario: Undeclared kind still appears
- **WHEN** the register carries a node of a kind the profile does not declare
- **THEN** the report includes that kind with its counts

### Requirement: Coverage report output
The coverage report SHALL be emitted through `output_result` in all three
formats (plain, json, rich).

The JSON output SHALL be an object with a `kinds` array, each entry carrying
`kind`, `total`, `incoming`, `outgoing`, `incoming_pct`, and `outgoing_pct`.

The plain output SHALL be a whitespace-separated table: one header row naming
the six columns, then one row per kind.

The rich output SHALL align the columns, show each percentage beside its
count, and end with a kind count.

Verified by: `cargo test --test coverage_report`

#### Scenario: JSON output structure
- **WHEN** `lattice coverage --format json` runs
- **THEN** stdout is valid JSON with a `kinds` array containing per-kind statistics

#### Scenario: Plain output is tabular
- **WHEN** `lattice coverage --format plain` runs
- **THEN** the output is a header row `kind total incoming incoming_pct outgoing outgoing_pct` followed by one data row per kind

### Requirement: Coverage report exit codes
`lattice coverage` SHALL exit 0 on success and 2 on setup failure (bad
profile, adapter crash, unparseable adapter output). It SHALL never exit 1 —
coverage is a report, not a validation pass — and it takes no `--strict`.

Adapter issues SHALL be reported on stderr after the report regardless of
severity, on the same terms as `query`, and SHALL NOT affect the exit code.

Verified by: `cargo test --test coverage_report`

#### Scenario: Successful report
- **WHEN** the adapter runs and the graph is built
- **THEN** the command exits 0

#### Scenario: Error-severity adapter issues do not move the exit code
- **WHEN** the adapter emitted an error-severity issue and the report is produced
- **THEN** the report prints to stdout, the issue prints to stderr, and the exit code is 0

#### Scenario: Adapter failure
- **WHEN** the adapter exits non-zero
- **THEN** the command exits 2
