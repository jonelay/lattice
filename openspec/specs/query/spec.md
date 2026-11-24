# query Specification

## Purpose
The `latt query` subcommand family — a closed set of graph questions
(reachability, paths, orphans, counts, and a live two-revision diff) computed
from the register at ask time, never stored.
## Requirements
### Requirement: Query subcommand family
`latt query` SHALL provide exactly seven queries as subcommands: `reaches`,
`reached-by`, `path`, `orphans`, `counts`, `at`, and `diff`. Each takes
`--profile`, `--target`, and `--format`, with optional `--adapter` resolved
according to the `builtin-adapter-dispatch` capability; none takes `--strict`.
Every CLI query answer SHALL be computed from a live adapter run at invocation time; no
query result is ever written back or read from a stored artifact.

The node-returning queries `reaches`, `reached-by`, `orphans`, `counts`, and `at`
SHALL accept a repeatable `--filter <condition>` option. All filters are evaluated
against each candidate node's attrs and all SHALL match for that node to be included.
No `--filter` means no filtering. Reachability filtering SHALL restrict only the
reported closure, never traversal. Counts SHALL filter node tallies while edge tallies
remain totals over the graph. For `at`, filters SHALL restrict trace entries; findings
at the selected path remain answer content independently of whether their attached node
matches.

Each condition SHALL split at the earliest position where a recognized operator
(`>=`, `<=`, `!=`, `~=`, `>`, `<`, `=`) begins, taking the longest operator that begins
there, so an attr name never contains an operator and a value may. The operators map to the
shared condition evaluator as equality, inequality, regex match, and the four ordering
comparisons. Equality and inequality values SHALL parse as integer, then boolean, then
string. Ordering values SHALL parse as integer, then string. Conditions are type-aware,
matching profile condition semantics. A condition with no recognized operator, an empty
attr name, or an invalid regex, SHALL make the command un-runnable and exit 2.

A filter whose attr appears on no node in the graph SHALL be reported on stderr, naming
the attr, and SHALL NOT change the exit code: the answer is still the answer (an empty
one), and an attr no profile declares may be one an adapter emits. Whether that answer
is acceptable is the caller's policy, not the query's.

Verified by: `cargo test --test query` and `cargo test --test native`.

#### Scenario: Query runs the adapter live
- **WHEN** the user runs `latt query counts --profile p.yaml --adapter ./a --target /repo`
- **THEN** lattice loads the profile, runs the adapter, ingests its document, and answers from that graph

#### Scenario: Repeated attribute filters use AND
- **WHEN** a node-returning query has `--filter status=active --filter priority>=2`
- **THEN** its answer includes only nodes satisfying both conditions

#### Scenario: Invalid attribute filter
- **WHEN** a node-returning query is given `--filter status` with no recognized operator
- **THEN** lattice exits 2 without running the query

#### Scenario: Filter splits at the earliest operator
- **WHEN** a node-returning query is given `--filter status=a>b`
- **THEN** the condition is `status` equal to the string `a>b`, not `status=a` above `b`

#### Scenario: Longest operator at the split position
- **WHEN** a node-returning query is given `--filter rank>=3`
- **THEN** the condition is `rank` at or above `3`, not `rank` above `=3`

#### Scenario: Empty attr name rejected
- **WHEN** a node-returning query is given `--filter =x`
- **THEN** lattice exits 2 naming the filter, without running the query

#### Scenario: Filter attr on no node
- **WHEN** a node-returning query is given `--filter stauts=done` and no node carries a `stauts` attr
- **THEN** the answer is empty, stderr names `stauts` as appearing on no node, and the exit code is 0

### Requirement: Query exit codes are two-valued
`latt query` SHALL exit 0 when the question was answered — including an
empty answer — and 2 when the question could not be posed or the run could not
happen (bad profile, adapter failure, unknown node ID or kind argument, git
failure in `diff`). It SHALL NOT exit 1: queries produce no findings, so the
three-valued contract applies to `validate`, `summary`, and `trace`. `at` carries validation
findings *inside* its answer, but they are content there, not a verdict, and
SHALL NOT affect the exit code.

Adapter issues SHALL be reported on stderr after the answer regardless of
severity (an answer computed from partially unreadable input must say so), and
SHALL NOT otherwise affect the exit code. `query` and `coverage` are the commands where
error-severity adapter issues leave the exit code at 0.
When a service query fails after loading a register, the CLI SHALL report its
retained adapter diagnostics on stderr before the operational error, applying the
same profile suppression and output-format policy as for a successful query.

A register that could not be read at all — the adapter declared no node and
reported an unsuppressed error-severity issue — cannot be asked a question: an
answer computed from it ("No orphans.", a table of zeros) reads as a clean
register. In that case the command SHALL print no answer, SHALL report the adapter
issues and that no node could be read on stderr, and SHALL exit 2. A register that
is empty but readable carries no such issue and is answered with exit 0. `at` is
exempt: its answer carries the findings at a path, so it shows the failure rather than
hiding it, and is the query to ask of an unreadable register.

`diff` applies the same test to each revision separately. A revision at which the
register is empty but readable is a legitimate end of a comparison and is answered. A
revision at which it could not be read SHALL make `diff` print no answer, report that
revision's adapter issues under its name and that no node could be read at it on
stderr, and exit 2.

The test reads adapter issues as the envelope carries them: after the profile's severity
overrides, and never marked suppressed, since `SUPPRESS` marks payload findings and not
adapter diagnostics (graph-service spec). A profile that does not want an adapter error
to block a query demotes its severity.

Verified by: `cargo test --test query exit`, and
`cargo test --test query -- unreadable_register empty_but_readable diff_unreadable`

#### Scenario: Empty answer exits 0
- **WHEN** `latt query orphans` finds no orphan nodes
- **THEN** the output states the empty result explicitly and the exit code is 0

#### Scenario: Error-severity adapter issues do not move the exit code
- **WHEN** the adapter emitted error-severity issues and a query answer is produced
- **THEN** the answer prints to stdout, the issues print to stderr, and the exit code is 0

#### Scenario: Unknown node ID argument
- **WHEN** `latt query reaches REQ-NOPE` names an ID not in the graph
- **THEN** lattice exits 2 with an error naming the ID, not an empty answer

#### Scenario: Unknown node with partially unreadable input
- **WHEN** `query reaches` or `query path` names an unknown node and the adapter reported `PARSE_ERROR` for another file
- **THEN** lattice prints the `PARSE_ERROR` and the unknown-node error on stderr, prints no answer, and exits 2

#### Scenario: Diff with an unreadable revision
- **WHEN** `latt query diff A B` runs and at `B` the register's only file is
  unparseable, so `B` has no node and an unsuppressed error-severity `PARSE_ERROR`
- **THEN** no diff is printed, stderr names `B` and carries its issues, and the exit code is 2

#### Scenario: Diff with an empty but readable revision
- **WHEN** `latt query diff A B` runs and at `B` the register holds no node and no error
- **THEN** the diff lists every node of `A` as removed and the exit code is 0

#### Scenario: Diff with a demoted error at an empty revision
- **WHEN** `latt query diff A B` runs, `B` has no node, and the profile overrides the
  severity of its only error-severity issue to `warning`
- **THEN** the diff is printed and the exit code is 0

### Requirement: Reachability queries
`latt query reaches <id>` SHALL report every node transitively reachable
from `<id>` along outgoing edges; `latt query reached-by <id>` the same
along incoming edges. A repeatable `--edge-kind <kind>` flag SHALL restrict
traversal to the named edge kinds; by default all edge kinds are traversed.
An `--edge-kind` naming a kind the profile does not declare SHALL exit 2.
The start node itself is not part of the answer. Results SHALL be ordered by
node ID. An edge endpoint that was never declared as a node is traversed
through but not reported as reached — reachability covers declared nodes
only. Attribute filters apply after the walk, so a non-matching intermediate
node does not prevent matching descendants from being reached.

When `--check-resolved` is passed, each node in the answer SHALL carry a
`tainted` boolean. A node is tainted when every path from the origin to it
passes through at least one undeclared endpoint; a node with at least one
path whose every endpoint is declared is not tainted. Undeclared endpoints
are still not reported. The JSON answer SHALL carry `tainted` per node;
plain and rich output SHALL mark tainted nodes with a visible suffix. Without
the flag the answer is unchanged: no `tainted` field and no second walk.

Verified by: `cargo test --test query reach`

#### Scenario: Transitive reach
- **WHEN** T-1 verifies REQ-1 and REQ-1 derives from N-1, and the user runs `latt query reaches T-1`
- **THEN** the answer lists REQ-1 and N-1

#### Scenario: Edge-kind restriction
- **WHEN** the same graph is queried with `reaches T-1 --edge-kind verifies`
- **THEN** the answer lists REQ-1 only

#### Scenario: Reverse reach
- **WHEN** the user runs `latt query reached-by N-1` on the same graph
- **THEN** the answer lists REQ-1 and T-1

#### Scenario: Reach filter does not stop traversal
- **WHEN** T-1 reaches active N-1 through a non-matching deferred REQ-1 and the query
  carries `--filter status=active`
- **THEN** the answer includes N-1 and excludes REQ-1

#### Scenario: Clean reach through declared nodes
- **WHEN** `reaches T-1 --check-resolved` runs and T-1 → REQ-1 → N-1 are all
  declared nodes
- **THEN** REQ-1 and N-1 appear with `tainted: false`

#### Scenario: Tainted reach through a vacancy
- **WHEN** `reaches T-1 --check-resolved` runs and T-1 → REQ-9 → N-1 where
  REQ-9 is not a declared node
- **THEN** N-1 appears with `tainted: true` and REQ-9 does not appear

#### Scenario: Taint carries past the first declared node
- **WHEN** `reaches T-1 --check-resolved` runs and T-1 → REQ-9 → REQ-1 → N-1
  where only REQ-9 is undeclared
- **THEN** REQ-1 and N-1 both appear with `tainted: true`

#### Scenario: One clean path is enough
- **WHEN** T-1 reaches N-1 both via T-1 → REQ-1 → N-1 (all declared) and via
  T-1 → REQ-9 → N-1 (REQ-9 undeclared)
- **THEN** N-1 appears with `tainted: false`

#### Scenario: A longer clean path still clears a shorter tainted one
- **WHEN** T-1 reaches N-1 via T-1 → REQ-9 → N-1 (REQ-9 undeclared) and via
  the longer T-1 → REQ-1 → REQ-2 → N-1 (all declared)
- **THEN** N-1 appears with `tainted: false`

#### Scenario: Without the flag there is no tainted field
- **WHEN** `reaches T-1` runs without `--check-resolved` on a graph with a
  vacancy on the only path
- **THEN** the reached node carries no `tainted` field

### Requirement: Path query
`latt query path <src> <tgt>` SHALL report one path from `<src>` to
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
- **WHEN** T-1 verifies REQ-1 and the user runs `latt query path T-1 REQ-1`
- **THEN** the answer shows T-1, the `verifies` edge, and REQ-1

#### Scenario: No path
- **WHEN** no edge chain connects the two named nodes
- **THEN** the output states no path exists and the exit code is 0

### Requirement: Orphans query
`latt query orphans` SHALL report every declared node that no edge names,
ordered by node ID. `--kind <kind>` SHALL restrict the answer to nodes of
that kind; a kind the profile does not declare SHALL exit 2. An edge counts
for a node whenever it names that node's ID, even when the edge's far
endpoint was never declared — the node is referenced, so it is not standing
alone; the dangling far endpoint is `VACANCY`'s business, not a new
orphan.

Verified by: `cargo test --test query orphans`

#### Scenario: Orphan reported
- **WHEN** a declared node has no incoming or outgoing edges
- **THEN** `latt query orphans` lists it with its kind and provenance

#### Scenario: Kind filter
- **WHEN** `latt query orphans --kind req` runs
- **THEN** only orphan nodes of kind `req` are listed

#### Scenario: Orphan attribute filter
- **WHEN** `latt query orphans --filter status=active` runs
- **THEN** only orphan nodes whose `status` attr equals `active` are listed

### Requirement: Counts query
`latt query counts` SHALL report per-kind node tallies and per-kind edge
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
`latt query at <path>` SHALL report what the register holds from one source
path: the entries — nodes with their edges and attached findings, in trace
order — whose provenance names the path, together with every other finding of
that run whose provenance names it (attached to a node declared elsewhere, or
attachable to none), ordered as findings sort. Findings here are answer
content, never a verdict: the exit code stays 0 whatever their severity.

The path SHALL match as given and as joined to the target, lexically, and an
absolute path beneath the target SHALL also match in its target-relative form
— adapters differ in which of the two they write; a directory SHALL match
every provenance beneath it, and the target root (`.`, or the target's own
path) every provenance written target-relative. The question is posed when
the path exists under the target or any provenance in the run names it — a
register file the adapter reported missing is exactly the path worth asking
about, so its absence from disk must not make it unanswerable. A path matching
neither SHALL exit 2 naming the path: a mistyped path must not read as a clean
empty answer. A finding about an input as a whole (a profile finding, written
`<profile>`) has no path: no argument matches it, the target root included, and
its reserved name does not pose the question.

The answer SHALL be computed from that invocation's adapter run and validation
pass; no reverse index is stored between runs.

Verified by: `cargo test --test query at`

#### Scenario: Entries declared at a path
- **WHEN** `latt query at r.md` runs and one node's provenance names `r.md`
- **THEN** the answer holds that node's entry with its edges, and no node from
  any other file

#### Scenario: A missing register file stays queryable
- **WHEN** the adapter reported `PARSE_ERROR` at a path that does not exist on disk
- **THEN** `latt query at` that path reports the finding and exits 0

#### Scenario: A mistyped path exits 2
- **WHEN** the queried path neither exists under the target nor appears in any provenance
- **THEN** lattice exits 2 with an error naming the path

#### Scenario: An existing file with no entries is an empty answer
- **WHEN** the queried path exists on disk and no provenance names it
- **THEN** the output states the empty result explicitly and the exit code is 0

#### Scenario: A directory argument matches beneath it
- **WHEN** `latt query at specs` runs and a node's provenance is `specs/a.md`
- **THEN** that node's entry is in the answer

#### Scenario: An absolute argument matches target-relative provenance
- **WHEN** `latt query at /repo/r.md --target /repo` runs and a node's provenance is `r.md`
- **THEN** that node's entry is in the answer rather than a falsely clean empty one

#### Scenario: The target root includes target-relative provenance
- **WHEN** `latt query at .` or `latt query at /repo --target /repo` runs and a
  node's provenance is `r.md`
- **THEN** that node's entry is in the answer: the root is beneath no path string,
  yet every target-relative provenance lies under it

#### Scenario: A finding at the path attached to a node elsewhere is listed
- **WHEN** an edge written in `r.md` dangles, attaching its finding to a node declared in `t.py`
- **THEN** `latt query at r.md` lists the finding beside the nodes rather than dropping it

#### Scenario: A profile finding has no path
- **WHEN** a SUPPRESS entry matches no finding, so the run holds a `SUPPRESS_UNUSED`
  finding written at `<profile>:0`
- **THEN** `latt query at '<profile>'` exits 2 naming the path, and `latt query at .`
  does not list the finding

### Requirement: Live two-revision diff
`latt query diff <rev-a> <rev-b>` SHALL materialize the target at each
named git revision, run the adapter against each, and compare the two
resulting graphs — two live runs, never a comparison against a committed
snapshot. It SHALL report nodes added, removed, and changed, edges added,
removed, and changed, and pathway changes. Node identity is the ID; a node
counts as changed when its kind or attrs differ between revisions. Edge
identity for diff purposes is (source, target, kind); an edge counts as
changed when it exists in both revisions with the same identity tuple but
different attrs. A changed node or edge is reported with both attr maps
(`attrs_a`, `attrs_b`), so the reader sees what changed rather than only
that something did. Provenance is excluded from comparison on both sides, so a
declaration that merely moved lines does not diff. Pathway changes (order or
current position) SHALL also be reported. Output is ordered by node ID / edge
tuple.

When parallel edges share the same identity tuple (multiplicity > 1), the
diff SHALL match them pairwise in document order. Matched pairs with
differing attrs appear in `edges_changed`; unmatched excess edges appear in
`edges_added` or `edges_removed`.

Both revisions SHALL be materialized at the same filesystem path: adapters
may embed the target path in attrs, so materializing the two revisions at
different paths would report every such node as changed when nothing about
it changed.

When the target is not a git repository, or a named revision is unknown,
lattice SHALL exit 2. The working tree of the target SHALL be left untouched.

Verified by: `cargo test --test query diff`

#### Scenario: Self-diff is empty
- **WHEN** `latt query diff REV REV` names the same revision twice
- **THEN** the answer states no differences and the exit code is 0

#### Scenario: Added node reported
- **WHEN** rev B declares a node rev A does not
- **THEN** the diff lists it as added

#### Scenario: Edge attr change is reported
- **WHEN** rev A has edge `A --verifies {confidence: low}--> B` and rev B has
  `A --verifies {confidence: high}--> B`
- **THEN** the diff lists the edge in `edges_changed` with both attribute sets

#### Scenario: Edge structural change is added/removed
- **WHEN** rev A has edge `A --verifies--> B` and rev B has `A --verifies--> C`
- **THEN** the diff lists `A→B` as removed and `A→C` as added, not as changed

#### Scenario: Parallel edges with mixed changes
- **WHEN** rev A has two `A --links--> B` edges with attrs `{w: 1}` and `{w: 2}`,
  and rev B has two with `{w: 1}` and `{w: 3}`
- **THEN** the first pair matches (no change), the second pair appears in
  `edges_changed`

#### Scenario: A target path embedded in attrs does not diff
- **WHEN** the adapter writes the absolute target path into a node attr and a revision is diffed against itself
- **THEN** the answer states no differences

#### Scenario: Moved declaration does not diff
- **WHEN** a node's declaration moved to a different line between revisions but its kind and attrs are unchanged
- **THEN** the diff does not report it

#### Scenario: Unknown revision
- **WHEN** `latt query diff` names a revision the target's git history does not contain
- **THEN** lattice exits 2 with an error naming the revision

### Requirement: Query output follows the tri-format contract
Every query answer SHALL be renderable as `plain`, `json`, and `rich`, routed
through the single output dispatcher. The `json` form of each answer SHALL
carry the result as structured fields (IDs, kinds, tallies, diff entries),
not prose.

Verified by: `cargo test --test query json`

#### Scenario: JSON counts
- **WHEN** `latt query counts --format json` runs
- **THEN** stdout is a JSON object with per-kind node and edge tallies as numbers

### Requirement: Diff materialization is bounded
`latt query diff` SHALL wait no longer than the profile's `adapter.timeout_seconds`
(default 300 seconds) for the `git --version` probe, the `git rev-parse`, `git ls-tree` and
`git cat-file` runs that materialize one revision, one bound for all four. A probe that
expires is not remembered: the next run asks again. When the bound expires lattice SHALL terminate the
running process, remove the partial materialization, and exit 2 with a message naming
the revision and the bound,
exactly as it exits 2 for an unknown revision. The bound is the adapter run's bound
rather than a separate key because the materialization is a step of the same load the
profile governs, and a fixed bound could not be exercised by a hermetic test.

Before extracting a revision, lattice SHALL clear the earlier one; when it cannot, it
SHALL exit 2 naming the directory rather than extract over it, since a revision read
over a partly cleared tree mixes two trees. A `git ls-tree` or `git cat-file` that exits
non-zero SHALL be reported with its own stderr, also when a write failure ended the read
(the SIGPIPE it then dies of is the consequence); a blob that cannot be written SHALL be
reported by its path, and no partial tree is ever a success: the directory is emptied.

Before any git command runs, the `git --version` probe included, lattice SHALL refuse an
empty revision, one that starts with `-`, or one holding NUL, and SHALL resolve the rest to an object id
with `git rev-parse --verify` (`--end-of-options` where git has it). The id, never the
user's text, goes to every later git command. A refusal exits 2 with `'<rev>' is not a revision in
<target>` and git's one-line reason, without git's usage text. The scratch directory
SHALL be created exclusively (an existing path is never adopted) with mode 0700 and an
unpredictable name; the second revision is extracted into the same directory after its
contents are removed, and dropping the guard removes only a real directory it created.
A scratch path that is no longer that directory (renamed and replaced, on Unix by device
and inode) when checked SHALL be neither cleared, written nor removed; the extraction exits 2.
The check precedes the clearing and each write and is not atomic with them: a process
running as the same user that can rename entries in the directory holding the scratch
directory (`TMPDIR`) while extraction runs is outside what it defends against.

Verified by: `cargo test --test query diff_materialization_exceeding_its_bound_exits_two`,
`cargo test --test query diff_materialization_reports_a_failed_blob_read_with_gits_stderr`,
`cargo test -p latt-core --lib a_blob_that_cannot_be_written_is_an_error_and_leaves_no_tree`,
`cargo test --test query diff_refuses_a_revision_that_looks_like_an_option`,
`cargo test --test query diff_refuses_an_option_revision_without_running_git`,
`cargo test --test query diff_bounds_a_git_version_probe_that_stalls`,
`cargo test --test query diff_bound_holds_when_a_git_descendant_keeps_the_pipe_open`,
`cargo test -p latt-core --lib the_directory_is_exclusive_private_and_owned`,
`cargo test -p latt-core --lib a_second_materialization_keeps_the_directory_and_its_mode`,
`cargo test -p latt-core --lib a_revision_resolves_to_an_object_id_with_or_without_end_of_options`,
`cargo test -p latt-core --lib a_ref_git_cannot_read_is_bounded_by_the_timeout`,
`cargo test -p latt-core --lib a_directory_renamed_and_replaced_is_neither_cleared_nor_removed`,
and `cargo test -p latt-core --lib a_tree_that_cannot_be_cleared_is_an_error_not_a_merge`.

#### Scenario: git cat-file fails while a revision is read
- **WHEN** the `git` on `PATH` writes `No space left on device` to stderr and exits 128
  for `cat-file`
- **THEN** `latt query diff` exits 2 with a message naming `git cat-file` and the
  revision and carrying `No space left on device`

#### Scenario: A blob cannot be written
- **WHEN** the scratch directory is not writable while a revision is extracted
- **THEN** the extraction fails naming the first path (`cannot materialize 'a.txt' at
  'HEAD': ...`) and the directory holds no partial tree

#### Scenario: git does not answer
- **WHEN** the profile declares `adapter: { timeout_seconds: 1 }` and the `git` on
  `PATH` does not exit within one second for `cat-file`
- **THEN** `latt query diff` exits 2 naming the revision and `1` second, and leaves
  no partial materialization behind

#### Scenario: A process git started keeps the blob pipe open
- **WHEN** the profile declares `adapter: { timeout_seconds: 1 }` and a process the
  `git cat-file` run started keeps `cat-file`'s output open after git is killed
- **THEN** `latt query diff` exits 2 within the bound with a message naming `exceeded`;
  the reader does not wait for that process to exit, and no partial materialization is left

#### Scenario: A stalled git version probe is bounded
- **WHEN** the profile declares `adapter: { timeout_seconds: 1 }` and the `git` on
  `PATH` does not answer `--version`
- **THEN** `latt query diff` exits 2 within the bound with a message naming `1` second

#### Scenario: An option revision starts no git process
- **WHEN** `latt query diff -- -x <rev>` runs
- **THEN** it exits 2 with `cannot start with '-'` and no `git` process was started

#### Scenario: A ref git cannot read is bounded
- **WHEN** a revision names a ref that is a FIFO under `.git/refs/heads` and the bound is
  one second
- **THEN** resolving it fails within the bound with a message naming `1 s`

#### Scenario: A replaced scratch directory is left alone
- **WHEN** the scratch directory is renamed and a new directory holding a file is created
  at its path before a revision is extracted
- **THEN** the extraction exits 2 saying the path is no longer the directory lattice
  created, and the file survives both the extraction and the guard's drop

### Requirement: Diff materialization is faithful
`latt query diff` SHALL materialize each revision from its committed blobs: the bytes of
every tree entry as stored, read with `git ls-tree -r -z` and one `git cat-file
--batch`. Attributes (`export-ignore`, `export-subst`, `text`, `eol`, `filter`) SHALL
NOT apply and no smudge filter SHALL run, so the adapter reads what the repository
holds and a register file marked `export-ignore` is read. A tree scoped by running from
a target subdirectory SHALL materialize that subdirectory's content only. Mode `100644`
becomes a file of mode 0644, `100755` of 0755, `120000` a symlink whose target is the
blob text, `160000` (a submodule) an empty directory; any other mode SHALL be refused.
Directories and regular files are created first (a file never replaces an existing path)
and symlinks last, so nothing is written through a link.

Every git command runs in the target's own repository on the objects as stored. The
repository-selecting environment variables git lists as local (`GIT_DIR`,
`GIT_WORK_TREE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
`GIT_INDEX_FILE`, `GIT_COMMON_DIR` and the rest of `git rev-parse --local-env-vars`) and
`GIT_NAMESPACE` SHALL NOT apply, and replacement objects (`refs/replace/*`) SHALL NOT
substitute any blob or tree. Grafts (`.git/info/grafts`) are not covered.

A tree entry whose path is empty, absolute, or has a component that is empty, `.`, `..`
or `.git` (in any letter case), or off Unix a component holding `\` or `:`, SHALL be refused with exit 2 and `cannot materialize
'<path>' at '<rev>': <reason>`, naming the first such path and counting the others;
no entry is skipped in silence.

Verified by: `cargo test --test query diff_reads_a_register_file_marked_export_ignore`,
`cargo test --test query diff_reads_a_register_file_marked_export_ignore_on_the_older_revision`,
`cargo test --test query diff_ignores_export_subst_eol_and_filter_attributes`,
`cargo test --test query diff_of_a_large_tree`,
`cargo test --test query diff_reads_the_target_repository_not_an_inherited_git_dir`,
`cargo test --test query diff_reads_stored_blobs_not_replacement_objects`,
`cargo test --test query diff_from_a_target_subdirectory_scopes_like_archive`,
`cargo test -p latt-core --lib modes_symlinks_and_gitlinks_are_materialized`,
and `cargo test -p latt-core --lib a_path_that_escapes_is_refused`.

#### Scenario: A register file marked export-ignore is read
- **WHEN** a revision commits `.gitattributes` with `doc.json export-ignore` and the
  adapter reads `doc.json` from the materialized target
- **THEN** the adapter finds the file and the diff reports its nodes

#### Scenario: Attributes and filters do not change the bytes
- **WHEN** a revision's `.gitattributes` sets `text eol=crlf`, `export-subst` and a
  `filter` whose smudge command creates a marker file
- **THEN** the adapter reads the committed LF bytes and the literal `$Format:%H$`, and
  the marker file is never created

#### Scenario: An inherited repository variable does not apply
- **WHEN** `latt query diff` runs with `GIT_DIR` set to another repository's `.git`
  (as inside a git hook or `git rebase --exec`)
- **THEN** the diff reports the target repository's changes and none of the other
  repository's

#### Scenario: Replacement objects do not apply
- **WHEN** the target holds a `refs/replace/*` ref that replaces a register blob of
  revision B with revision A's
- **THEN** the diff is the one a repository without the replacement gives

#### Scenario: Modes, symlinks and submodules
- **WHEN** a tree holds a `100755` file, a `120000` link to `../x` and a `160000` entry
- **THEN** the file is executable, the link reads `../x`, and the submodule is an empty
  directory

#### Scenario: A path that escapes is refused
- **WHEN** a tree holds an entry named `../up` or `.git/hooks`
- **THEN** `latt query diff` exits 2 with `cannot materialize '../up' at '<rev>'` and
  nothing is written outside the scratch directory
