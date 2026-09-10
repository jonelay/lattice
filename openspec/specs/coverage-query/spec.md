# coverage-query Specification

## Purpose
Coverage and summary queries — the first payoff from the graph: REQs with no test,
orphan IDs, and a computed status rollup replacing the hand-maintained index table.
## Requirements
### Requirement: Coverage check
The validation SHALL report every `req` node that has no incoming `verifies` edge from an
existing source node as a coverage warning. This is the "REQ with no test" query.

An edge whose source node was never added does not count as coverage: it is a dangling
reference, already reported as `VACANCY`, and treating it as a test would let a
typo in a marker silently satisfy a requirement.

Coverage — flat and deep alike — measures **declared evidence**: the presence of
profile-configured evidence edges from explicit source nodes. It SHALL NOT be read as
asserting execution, pass/fail, assertion relevance, or evidence scope; an adapter that
expands one broad marker onto many tests yields many edges recording one attribution.
Verification of the evidence itself is the register owner's, outside this tool.

The report SHALL distinguish three states per `req` node, so a reader is never pushed
toward writing tests that may already exist (design-basis §6: "untested" and "untagged"
were reported identically):

- *verified* — an incoming `verifies` edge from an existing test node; no finding.
- *unknown* — no incoming `verifies` edge, and test nodes with no outgoing `verifies`
  edge exist in the register. The gap may be attribution rather than absence, and the
  register owner resolves it by adding markers, not by writing tests.
- *unverified* — no incoming `verifies` edge, and every test node in the register is
  attributed. The gap is real absence as far as the register can show.

The state SHALL be machine-readable in the JSON payload (the `COVERAGE` finding's
`state` field, per the validation spec), not only prose. The unattributed population
itself is reported once, as a `COVERAGE_UNKNOWN` hint carrying the count.

Verified by: `cargo test --test validation coverage_state`

#### Scenario: Uncovered REQ
- **WHEN** REQ-0401 has no test with `@pytest.mark.req("REQ-0401")`
- **THEN** validation reports a coverage warning for REQ-0401

#### Scenario: Covered REQ
- **WHEN** REQ-0604 has tests with `@pytest.mark.req("REQ-0604")`
- **THEN** no coverage warning for REQ-0604

#### Scenario: Dangling verifies edge is not coverage
- **WHEN** a `verifies` edge targets REQ-0401 but its source node does not exist in the graph
- **THEN** REQ-0401 is still reported as uncovered

#### Scenario: Unmarked tests make coverage unknown
- **WHEN** REQ-0401 has no incoming `verifies` edge and the register holds test nodes
  with no outgoing `verifies` edge
- **THEN** REQ-0401's coverage finding carries `state: "unknown"` and a
  `COVERAGE_UNKNOWN` hint reports how many tests carry no marker

#### Scenario: Fully attributed register reports real absence
- **WHEN** REQ-0401 has no incoming `verifies` edge and every test node has an outgoing
  `verifies` edge
- **THEN** REQ-0401's coverage finding carries `state: "unverified"`

### Requirement: Summary subcommand
`lattice summary` SHALL output a computed status rollup: for each spec file, count
done/partial/todo/blocked spec-goals. Output follows the tri-format contract
(plain/json/rich). The grouping node kind, status attr, and group-by attr come from the
profile's `SUMMARY` validation config; `lattice summary` SHALL exit 2 when the profile
declares no usable `SUMMARY` config, since it has nothing to roll up.

#### Scenario: Summary matches spec file counts
- **WHEN** `lattice summary` runs against phase-sweep with the RM profile and adapter
- **THEN** the output contains per-file counts matching the actual spec heading markers

#### Scenario: Summary JSON format
- **WHEN** `lattice summary --format=json` runs
- **THEN** the output is a JSON object with a `files` array, each entry having `file`,
  `done`, `partial`, `todo`, `blocked`, `total`, plus a `totals` object holding the
  column sums

#### Scenario: Profile has no SUMMARY config
- **WHEN** `lattice summary` runs with a profile that declares no `SUMMARY` validation config
- **THEN** lattice exits 2 with an error naming the missing config

Adapter issues SHALL be reported on stderr after the rollup regardless of severity, so a
rollup computed from partially unreadable input says so. Only error-severity issues
affect the exit code.

#### Scenario: Adapter errors surface after the rollup
- **WHEN** the adapter emitted error-severity issues and the rollup is produced
- **THEN** lattice prints the rollup, reports those errors on stderr, and exits 1

#### Scenario: Adapter warnings surface after the rollup
- **WHEN** the adapter emitted only warning-severity issues
- **THEN** lattice prints the rollup, reports those warnings on stderr, and exits 0

### Requirement: Orphan detection
The existing ORPHAN_NODE validator SHALL flag IDs that appear in edges but not as nodes
(via VACANCY) and nodes with no connections (via ORPHAN_NODE). No new validator
needed — the built-in validators cover this when the adapter builds the graph correctly.

#### Scenario: Cited REQ absent from REQUIREMENTS.md
- **WHEN** a spec heading cites `[REQ-9999]` but REQ-9999 is not in REQUIREMENTS.md
- **THEN** validation reports a VACANCY finding

### Requirement: Deep coverage rollup
A profile MAY declare a `COVERAGE_DEEP` validation entry with config keys
`target_kind` (a declared node kind), `via` (a declared edge kind whose edge
*targets* are the parents — e.g. `derives`, where a child derives from its
parent), and `evidence` (a declared edge kind — e.g. `verifies`).

The validation SHALL compute, per run and storing nothing, the **least fixed
point** of the coverage validation over the nodes of `target_kind`:

- A target's *children* are the existing nodes of `target_kind` that are
  sources of a `via` edge whose target is that node. `via` edges whose source
  is of another kind, or whose source node was never added, contribute no
  child.
- The covered set starts as the *directly evidenced* targets — those with an
  incoming `evidence` edge from an existing source node. An edge from a node
  that was never added SHALL NOT count as evidence.
- The set then grows by one rule, applied until nothing changes: a target
  enters when it has at least one child and every one of its children is
  already in the set.

Because the least fixed point is taken, a childless target with no direct
evidence is uncovered (the quantifier over zero children discharges nothing),
an evidence-free cycle stays uncovered, and a cycle one of whose members has
direct evidence can propagate coverage through and out of the cycle.

Every node of `target_kind` outside the covered set SHALL get exactly one
`COVERAGE_DEEP` finding per config, however many parents share it. The finding
message SHALL name the immediate cause: the node's uncovered children by ID,
or that it has no evidence and no children. For nodes in an uncovered cycle —
a strongly connected component of the `via` subgraph over uncovered targets —
the message SHALL instead name the cycle's member IDs in lexicographic order,
identically for every member. Findings are deterministic for a given document.

The finding SHALL carry the same machine-readable `state` field as `COVERAGE`,
by the same global rule: `unknown` when the register holds nodes of a kind the
`evidence` edge kind admits as source that have no outgoing `evidence` edge,
`unverified` otherwise. Each `COVERAGE_DEEP` config SHALL report that
unattributed population once as a `COVERAGE_UNKNOWN` hint, exactly as a
`COVERAGE` config does.

`COVERAGE_DEEP` SHALL default to `warning` severity, like `COVERAGE`; a
profile MAY demote it (to `hint` where advice is wanted). It SHALL NOT default
to `hint`: hint is never promotable, and a hint default would foreclose
gating on deep coverage for every profile permanently.

This validation is a universal rollup, deliberately distinct from existential
reachability ("does any evidence path reach this node"), which belongs to the
`reaches` query and answers a different question over the same edges.

**Verified by:** `cargo test --test coverage_deep`

#### Scenario: Parent covered through fully verified children
- **WHEN** SYS-1 has no direct evidence, SUB-3 and SUB-4 derive from SYS-1, and
  both have incoming `verifies` edges from existing test nodes
- **THEN** SYS-1 gets no `COVERAGE_DEEP` finding

#### Scenario: One unverified child leaves the parent uncovered
- **WHEN** SUB-3 is verified but SUB-4 has no evidence and no children
- **THEN** SYS-1 and SUB-4 each get a `COVERAGE_DEEP` finding, and SYS-1's
  message names SUB-4

#### Scenario: Direct evidence covers a parent regardless of its children
- **WHEN** SYS-2 has an incoming `verifies` edge from an existing test node and
  an uncovered deriving child
- **THEN** SYS-2 gets no `COVERAGE_DEEP` finding and the child still gets one

#### Scenario: Childless target without evidence is uncovered
- **WHEN** SUB-9 has no incoming `evidence` edge and no deriving children
- **THEN** SUB-9 gets a `COVERAGE_DEEP` finding whose message says it has no
  evidence and no children

#### Scenario: Dangling evidence is not coverage
- **WHEN** the only `verifies` edge into SUB-4 names a source node that was
  never added
- **THEN** SUB-4 is not deep-covered

#### Scenario: Dangling via source contributes no child
- **WHEN** the only `derives` edge into SYS-3 has a source node that was never
  added, and SYS-3 has no direct evidence
- **THEN** SYS-3 is uncovered as a node with no children

#### Scenario: Child of another kind is not part of the rollup
- **WHEN** the `via` edge kind admits `[other, req]` and an `other`-kind node
  derives from REQ-1, which has no `req`-kind children and no direct evidence
- **THEN** REQ-1 is uncovered as a node with no children

#### Scenario: A shared child is one finding, evaluated once
- **WHEN** uncovered SUB-7 derives from both SYS-4 and SYS-5
- **THEN** SUB-7 gets exactly one `COVERAGE_DEEP` finding, and both parents'
  findings name SUB-7

#### Scenario: Evidence-free cycle stays uncovered and is reported
- **WHEN** SUB-5 and SUB-6 derive from each other and neither has direct
  evidence
- **THEN** validation terminates, both get `COVERAGE_DEEP` findings, and each
  message names the cycle members `SUB-5, SUB-6` identically

#### Scenario: Anchored cycle propagates coverage out
- **WHEN** SUB-5 and SUB-6 derive from each other and SUB-5 has an incoming
  `verifies` edge from an existing test node
- **THEN** neither SUB-5 nor SUB-6 gets a `COVERAGE_DEEP` finding

#### Scenario: State split propagates
- **WHEN** SYS-1 is uncovered and the register holds evidence-source-kind
  nodes with no outgoing `verifies` edge
- **THEN** SYS-1's `COVERAGE_DEEP` finding carries `state: "unknown"` and the
  config emits one `COVERAGE_UNKNOWN` hint

#### Scenario: A faulty entry does not suppress a valid one
- **WHEN** a profile declares two `COVERAGE_DEEP` entries and one names a `via`
  edge kind the profile does not declare
- **THEN** the faulty entry emits `CONFIG_ERROR` and performs no check, and the
  valid entry's findings are unaffected
