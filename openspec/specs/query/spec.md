# query Specification

## Purpose
The `lattice query` subcommand family — a closed set of graph questions
(reachability, paths, orphans, counts, and a live two-revision diff) computed
from the register at ask time, never stored.
## Requirements
### Requirement: Query subcommand family
`lattice query` SHALL provide exactly seven queries as subcommands: `reaches`,
`reached-by`, `path`, `orphans`, `counts`, `at`, and `diff`. Each takes
`--profile`, `--adapter`, `--target`, and `--format`; none takes `--strict`.
Every answer SHALL be computed from a live adapter run at invocation time; no
query result is ever written back or read from a stored artifact.

Verified by: `cargo test --test query`

#### Scenario: Query runs the adapter live
- **WHEN** the user runs `lattice query counts --profile p.yaml --adapter ./a --target /repo`
- **THEN** lattice loads the profile, runs the adapter, ingests its document, and answers from that graph

### Requirement: Query exit codes are two-valued
`lattice query` SHALL exit 0 when the question was answered — including an
empty answer — and 2 when the question could not be posed or the run could not
happen (bad profile, adapter failure, unknown node ID or kind argument, git
failure in `diff`). It SHALL NOT exit 1: queries produce no findings, so the
three-valued contract remains `validate`'s alone. `at` carries validation
findings *inside* its answer, but they are content there, not a verdict, and
SHALL NOT affect the exit code.

Adapter issues SHALL be reported on stderr after the answer regardless of
severity (an answer computed from partially unreadable input must say so), and
SHALL NOT affect the exit code — this is the one place error-severity adapter
issues leave the exit code at 0.

Verified by: `cargo test --test query exit`

#### Scenario: Empty answer exits 0
- **WHEN** `lattice query orphans` finds no orphan nodes
- **THEN** the output states the empty result explicitly and the exit code is 0

#### Scenario: Error-severity adapter issues do not move the exit code
- **WHEN** the adapter emitted error-severity issues and a query answer is produced
- **THEN** the answer prints to stdout, the issues print to stderr, and the exit code is 0

#### Scenario: Unknown node ID argument
- **WHEN** `lattice query reaches REQ-NOPE` names an ID not in the graph
- **THEN** lattice exits 2 with an error naming the ID, not an empty answer

### Requirement: Reachability queries
`lattice query reaches <id>` SHALL report every node transitively reachable
from `<id>` along outgoing edges; `lattice query reached-by <id>` the same
along incoming edges. A repeatable `--edge-kind <kind>` flag SHALL restrict
traversal to the named edge kinds; by default all edge kinds are traversed.
An `--edge-kind` naming a kind the profile does not declare SHALL exit 2.
The start node itself is not part of the answer. Results SHALL be ordered by
node ID. An edge endpoint that was never declared as a node is not reported
as reached — reachability covers declared nodes only.

Verified by: `cargo test --test query reach`

#### Scenario: Transitive reach
- **WHEN** T-1 verifies REQ-1 and REQ-1 derives from N-1, and the user runs `lattice query reaches T-1`
- **THEN** the answer lists REQ-1 and N-1

#### Scenario: Edge-kind restriction
- **WHEN** the same graph is queried with `reaches T-1 --edge-kind verifies`
- **THEN** the answer lists REQ-1 only

#### Scenario: Reverse reach
- **WHEN** the user runs `lattice query reached-by N-1` on the same graph
- **THEN** the answer lists REQ-1 and T-1

### Requirement: Path query
`lattice query path <src> <tgt>` SHALL report one path from `<src>` to
`<tgt>` along outgoing edges as an alternating sequence of node IDs and edge
kinds, honouring the same `--edge-kind` restriction as `reaches`. When no
path exists the answer SHALL state that explicitly and exit 0 — "not
connected" is an answer, not a failure. When multiple paths exist, a shortest
one SHALL be reported, ties broken by node ID order, so output is
deterministic.

The reported path SHALL begin with `<src>` and end with `<tgt>`, with exactly
one fewer edge than nodes. A self-loop (`<src>` equals `<tgt>`) is a valid
answer: one node, zero edges.

Verified by: `cargo test --test query path`, `cargo test --test output path_report`

#### Scenario: Connected nodes
- **WHEN** T-1 verifies REQ-1 and the user runs `lattice query path T-1 REQ-1`
- **THEN** the answer shows T-1, the `verifies` edge, and REQ-1

#### Scenario: No path
- **WHEN** no edge chain connects the two named nodes
- **THEN** the output states no path exists and the exit code is 0

### Requirement: Orphans query
`lattice query orphans` SHALL report every declared node that no edge names,
ordered by node ID. `--kind <kind>` SHALL restrict the answer to nodes of
that kind; a kind the profile does not declare SHALL exit 2. An edge counts
for a node whenever it names that node's ID, even when the edge's far
endpoint was never declared — the node is referenced, so it is not standing
alone; the dangling far endpoint is `DANGLING_REF`'s business, not a new
orphan.

Verified by: `cargo test --test query orphans`

#### Scenario: Orphan reported
- **WHEN** a declared node has no incoming or outgoing edges
- **THEN** `lattice query orphans` lists it with its kind and provenance

#### Scenario: Kind filter
- **WHEN** `lattice query orphans --kind req` runs
- **THEN** only orphan nodes of kind `req` are listed

### Requirement: Counts query
`lattice query counts` SHALL report per-kind node tallies and per-kind edge
tallies. Every node and edge kind the profile declares SHALL appear, with an
explicit zero for a kind the register does not instantiate — a zero edge
count must be visible, not absent, so "zero findings over zero edges" is
first-class. A kind present in the register but not declared by the profile
SHALL also appear, so counts never under-reports what was ingested.

Verified by: `cargo test --test query counts`

#### Scenario: Declared kind with no instances
- **WHEN** the profile declares edge kind `mitigates` and the register holds no such edge
- **THEN** counts shows `mitigates: 0`

#### Scenario: Tallies match the register
- **WHEN** the register holds 3 `req` nodes and 2 `verifies` edges
- **THEN** counts shows `req: 3` and `verifies: 2`

### Requirement: Provenance query
`lattice query at <path>` SHALL report what the register holds from one source
path: the entries — nodes with their edges and attached findings, in trace
order — whose provenance names the path, together with every other finding of
that run whose provenance names it (attached to a node declared elsewhere, or
attachable to none), ordered as findings sort. Findings here are answer
content, never a verdict: the exit code stays 0 whatever their severity.

The path SHALL match as given and as joined to the target, lexically, and an
absolute path beneath the target SHALL also match in its target-relative form
— adapters differ in which of the two they write; a directory SHALL match
every provenance beneath it. The question is posed when
the path exists under the target or any provenance in the run names it — a
register file the adapter reported missing is exactly the path worth asking
about, so its absence from disk must not make it unanswerable. A path matching
neither SHALL exit 2 naming the path: a mistyped path must not read as a clean
empty answer.

The answer SHALL be computed from that invocation's adapter run and validation
pass; no reverse index is stored between runs.

Verified by: `cargo test --test query at`

#### Scenario: Entries declared at a path
- **WHEN** `lattice query at r.md` runs and one node's provenance names `r.md`
- **THEN** the answer holds that node's entry with its edges, and no node from
  any other file

#### Scenario: A missing register file stays queryable
- **WHEN** the adapter reported `PARSE_ERROR` at a path that does not exist on disk
- **THEN** `lattice query at` that path reports the finding and exits 0

#### Scenario: A mistyped path exits 2
- **WHEN** the queried path neither exists under the target nor appears in any provenance
- **THEN** lattice exits 2 with an error naming the path

#### Scenario: An existing file with no entries is an empty answer
- **WHEN** the queried path exists on disk and no provenance names it
- **THEN** the output states the empty result explicitly and the exit code is 0

#### Scenario: A directory argument matches beneath it
- **WHEN** `lattice query at specs` runs and a node's provenance is `specs/a.md`
- **THEN** that node's entry is in the answer

#### Scenario: An absolute argument matches target-relative provenance
- **WHEN** `lattice query at /repo/r.md --target /repo` runs and a node's provenance is `r.md`
- **THEN** that node's entry is in the answer rather than a falsely clean empty one

#### Scenario: A finding at the path attached to an entry elsewhere is listed
- **WHEN** an edge written in `r.md` dangles, attaching its finding to a node declared in `t.py`
- **THEN** `lattice query at r.md` lists the finding beside the entries rather than dropping it

### Requirement: Live two-revision diff
`lattice query diff <rev-a> <rev-b>` SHALL materialize the target at each
named git revision, run the adapter against each, and compare the two
resulting graphs — two live runs, never a comparison against a committed
snapshot. It SHALL report nodes added, removed, and changed, and edges added
and removed. Node identity is the ID; a node counts as changed when its kind
or attrs differ between revisions. Edges compare as a multiset of
(source, target, kind). Provenance is excluded from comparison on both, so a
declaration that merely moved lines does not diff. Axis changes (order or
current position) SHALL also be reported. Output is ordered by node ID / edge
tuple.

Both revisions SHALL be materialized at the same filesystem path: adapters
embed the target path in attrs (an adapter's `file` attr can), so
materializing the two revisions at different paths would report every such
node as changed when nothing about it changed.

When the target is not a git repository, or a named revision is unknown,
lattice SHALL exit 2. The working tree of the target SHALL be left untouched.

Verified by: `cargo test --test query diff`

#### Scenario: Self-diff is empty
- **WHEN** `lattice query diff REV REV` names the same revision twice
- **THEN** the answer states no differences and the exit code is 0

#### Scenario: Added node reported
- **WHEN** rev B declares a node rev A does not
- **THEN** the diff lists it as added

#### Scenario: A target path embedded in attrs does not diff
- **WHEN** the adapter writes the absolute target path into a node attr and a revision is diffed against itself
- **THEN** the answer states no differences

#### Scenario: Moved declaration does not diff
- **WHEN** a node's declaration moved to a different line between revisions but its kind and attrs are unchanged
- **THEN** the diff does not report it

#### Scenario: Unknown revision
- **WHEN** `lattice query diff` names a revision the target's git history does not contain
- **THEN** lattice exits 2 with an error naming the revision

### Requirement: Query output follows the tri-format contract
Every query answer SHALL be renderable as `plain`, `json`, and `rich`, routed
through the single output dispatcher. The `json` form of each answer SHALL
carry the result as structured fields (IDs, kinds, tallies, diff entries),
not prose.

Verified by: `cargo test --test query json`

#### Scenario: JSON counts
- **WHEN** `lattice query counts --format json` runs
- **THEN** stdout is a JSON object with per-kind node and edge tallies as numbers
