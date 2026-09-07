# rm-profile Specification

## Purpose
The requirements-RM profile — a YAML declaration of the node kinds, edge kinds, typed
attributes, and validation configuration for BN/UN/REQ/spec-goal/test/SC traceability.
## Requirements
### Requirement: Node kinds
The RM profile SHALL declare five node kinds: `need` (BN/UN), `req` (REQ-NNNN),
`spec-goal` (spec heading), `test` (pytest function), and `sc` (success criterion).
Each kind SHALL declare a `summary_attr` naming the attr the trace "Key Attr" column
shows: `text` for `need`, `req`, and `sc`; `title` for `spec-goal`; `function` for
`test`.

#### Scenario: All five kinds declared
- **WHEN** the RM profile is loaded
- **THEN** `profile.node_kinds` contains entries for `need`, `req`, `spec-goal`, `test`, and `sc`

#### Scenario: Summary attrs declared
- **WHEN** the RM profile is loaded
- **THEN** `need.summary_attr` is `text`, `req.summary_attr` is `text`,
  `spec-goal.summary_attr` is `title`, `test.summary_attr` is `function`,
  and `sc.summary_attr` is `text`

### Requirement: Need node attrs
Need nodes SHALL have attrs: `text` (string, required), `tier` (enum: business, user, required),
and `traces_to` (list of strings, optional — the BN/UN IDs this need derives from).

#### Scenario: Need node with tier
- **WHEN** a need node has `tier: "user"` and `text: "Compare slot/pole..."`
- **THEN** validation produces no attr errors

### Requirement: Req node attrs
Req nodes SHALL have attrs: `text` (string, required), `domain` (string, required),
and `rationale` (string, optional).

#### Scenario: Req node with domain
- **WHEN** a req node has `domain: "07"` and `text: "..."`
- **THEN** validation produces no attr errors

### Requirement: Spec-goal node attrs
Spec-goal nodes SHALL have attrs: `title` (string, required), `status` (enum: done,
partial, todo, blocked, required), and `file` (string, required).

#### Scenario: Spec-goal status values
- **WHEN** a spec-goal node has `status: "done"`
- **THEN** validation produces no attr errors

### Requirement: Test node attrs
Test nodes SHALL have attrs: `file` (string, required), `function` (string, required),
and `docstring` (string, optional — the test's docstring when it has one, carried for
the phase-5 suggestion sidecar).

The `test` kind SHALL declare `text_attrs: [function, docstring]`. The sidecar ranked both
of those values while it named `docstring` in its own code; once the attr name is profile
data, only this declaration keeps that ranking. Omitting it would fall back to
`summary_attr` alone and silently drop test docstrings — a ranking regression
with no finding, no error and no visible symptom. This
lands as profile version `1.10.0`.

Verified by: `.venv/bin/python -m pytest tests -k rm_profile`

#### Scenario: Test node
- **WHEN** a test node has `file: "tests/test_fem.py"` and `function: "test_fem_linear"`
- **THEN** validation produces no attr errors

#### Scenario: Test node with docstring
- **WHEN** a test node additionally carries `docstring: "Linear FEM field check."`
- **THEN** validation produces no attr errors

#### Scenario: The test kind declares its ranked text
- **WHEN** the profile is loaded
- **THEN** the `test` kind's `text_attrs` is `[function, docstring]`, so a ranking consumer
  reads both without naming either

### Requirement: SC node attrs
SC nodes SHALL have attrs: `text` (string, required), `stakeholder` (string, required),
and `category` (enum: Must-have, Should-have, Nice-to-have, required).

#### Scenario: SC node with category
- **WHEN** an sc node has `text: "Engineering team can run..."`, `stakeholder: "S2"`,
  and `category: "Must-have"`
- **THEN** validation produces no attr errors

### Requirement: Edge kinds
The RM profile SHALL declare three edge kinds: `derives` (need→need, req→need, req→req,
req→sc), `fulfills` (spec-goal→req), and `verifies` (test→req).

#### Scenario: Allowed edge pairs
- **WHEN** a `verifies` edge connects a test to a req
- **THEN** validation produces no edge constraint errors

#### Scenario: Disallowed edge pair
- **WHEN** a `verifies` edge connects a need to a req
- **THEN** validation produces an EDGE_CONSTRAINT error

#### Scenario: Req derives from SC
- **WHEN** a `derives` edge connects a req to an sc
- **THEN** validation produces no edge constraint errors

### Requirement: ID patterns
The profile SHALL enforce: needs match `^(BN|UN)-\d+$`, reqs match `^REQ-\d{4}$`,
spec-goals match `^\d{2}-\d+\.\d+[a-z]?$`, tests match `^[^:]+\.py::(Test.+::)?test_.+$`,
and sc nodes match `^SC-\d{3}$`.

The spec-goal pattern's trailing `[a-z]?` admits a goal inserted between two existing
ones — `3.6a` between `3.6` and `3.7`. The alternative is renumbering the goals after
the insertion point, which changes IDs that other files already cite; node IDs are the
public join key, so an append-only insertion is the only shape that does not break
existing references. The suffix is one lowercase letter on the last component only:
`3.6ab`, `3.6A` and `3a.6` SHALL be rejected, so a 27th insertion forces a decision
rather than silently widening further. This lands as profile version `1.11.0`.

Test IDs are pytest nodeids: a mandatory module
path ending in `.py`, double-colon separated from an optional class and the test
function. The module path is a filesystem path, so it SHALL admit any character
except the `:` separator. Constraining it to a charset only invents ID_FORMAT
errors for directory names the adapter reads correctly.

#### Scenario: Qualified test ID
- **WHEN** a test node has ID "tests/test_fem.py::test_sweep" or
  "tests/test_fem.py::TestSweep::test_heatmap"
- **THEN** no ID_FORMAT error

#### Scenario: Path-qualified test ID
- **WHEN** a test node has ID "tests/unit/test_fem.py::test_sweep"
- **THEN** no ID_FORMAT error

#### Scenario: Punctuated directory name
- **WHEN** a test node has ID "hw-tests/test_fem.py::test_sweep" or "unit+io/test_fem.py::test_sweep"
- **THEN** no ID_FORMAT error

#### Scenario: Bare function name is not a test ID
- **WHEN** a test node has ID "test_fem_linear"
- **THEN** an ID_FORMAT error is produced

#### Scenario: Letter-suffixed spec-goal ID
- **WHEN** a spec-goal node has ID "01-3.6a"
- **THEN** no ID_FORMAT error

#### Scenario: Over-wide letter suffix is not a spec-goal ID
- **WHEN** a spec-goal node has ID "01-3.6ab", "01-3.6A" or "01-3a.6"
- **THEN** an ID_FORMAT error is produced

#### Scenario: Valid req ID
- **WHEN** a req node has ID "REQ-0701"
- **THEN** no ID_FORMAT error

#### Scenario: Invalid req ID
- **WHEN** a req node has ID "REQ-1"
- **THEN** an ID_FORMAT error is produced

#### Scenario: Valid SC ID
- **WHEN** an sc node has ID "SC-001"
- **THEN** no ID_FORMAT error

#### Scenario: Invalid SC ID
- **WHEN** an sc node has ID "SC-1"
- **THEN** an ID_FORMAT error is produced

### Requirement: Spec quotes shipped ID patterns
This spec SHALL quote, verbatim, every `id_pattern` the shipped profile declares, so
that a pattern changed in `profiles/requirements-rm.yaml` without the spec following
is caught as drift rather than inherited silently by the register. The guard is a
meta-check on spec–profile agreement, distinct from the ID_FORMAT enforcement the
patterns themselves configure.

#### Scenario: A pattern changes without the spec following
- **WHEN** a node kind's `id_pattern` source in the shipped profile no longer appears
  verbatim in this spec
- **THEN** the drift-guard tests fail the suite

### Requirement: Coverage validation
The profile SHALL configure verification coverage as a deep rollup: a `COVERAGE_DEEP`
validation over `req` with `via: derives` and `evidence: verifies`, so a requirement
counts as covered by direct test evidence or by every deriving child being covered. A
flat check read a parent covered only through its children as a false positive, which
is why the rollup is deep. Missing coverage is a warning, not an error.

#### Scenario: Req with no test
- **WHEN** a req node has no incoming `verifies` edge and no deriving children
- **THEN** a warning-severity finding is produced

#### Scenario: Req covered through its children
- **WHEN** a req node has no incoming `verifies` edge but every req deriving from it
  is covered
- **THEN** no finding is produced for that node


### Requirement: Summary configuration
The profile SHALL configure a `SUMMARY` validation naming `node_kind: spec-goal`,
`status_attr: status`, and `group_by_attr: file`, so `lattice summary` rolls spec-goals
up by spec file. `lattice summary` depends on this block and cannot run without it.

#### Scenario: Summary groups spec-goals by file
- **WHEN** `lattice summary` runs with this profile
- **THEN** the rollup has one row per spec file, with columns for each `status` enum value

### Requirement: Adapter paths section
The RM profile SHALL declare an `adapter` section with a `paths` map containing three
string entries: `requirements` (path to the requirements markdown file), `spec_dir`
(path to the spec directory), and `tests_dir` (path to the tests directory). All paths
are relative to the target repo root. This section is consumed by the adapter, not by
core — core preserves it verbatim in `Profile.extra`.

The `adapter` section MAY also declare `cited_path_prefixes`, a list of strings. When
present, the adapter checks repo-relative paths cited in spec files that begin with one of
these prefixes, and emits `SOURCE_MISSING` for each that does not resolve. When absent, no
such check runs.

#### Scenario: Target paths declared
- **WHEN** the RM profile is loaded
- **THEN** `profile.extra["adapter"]["paths"]` contains `requirements: "docs/internal/REQUIREMENTS.md"`, `spec_dir: "docs/internal/spec"`, and `tests_dir: "tests"`

#### Scenario: Cited path prefixes declared
- **WHEN** the RM profile is loaded
- **THEN** `profile.extra["adapter"]["cited_path_prefixes"]` contains `"src/"` and `"tests/"`

### Requirement: Requirements need both a test and a spec goal
The RM profile SHALL declare two coverage validations over the `req` kind: the deep
`COVERAGE_DEEP` rollup requiring `verifies` evidence, and a flat `COVERAGE` requiring
an incoming `fulfills` edge. A requirement
that no test verifies and a requirement that no spec goal fulfills are different gaps with
different remedies, and reporting only the first leaves a requirement that is declared and
then never designed invisible.

#### Scenario: Requirement with no spec goal
- **WHEN** a `req` node has an incoming `verifies` edge but no incoming `fulfills` edge
- **THEN** validation emits a `COVERAGE` issue naming the `fulfills` edge kind

#### Scenario: Requirement fully traced
- **WHEN** a `req` node has both an incoming `verifies` edge and an incoming `fulfills` edge
- **THEN** validation emits no `COVERAGE` issue for that node


### Requirement: Test kind is orphan_ok
The `test` kind SHALL declare `orphan_ok: true`. Unmarked tests enter the graph with no
edges by design, and each raising `ORPHAN_NODE` would flip `--strict` exit codes for
every consumer the moment the adapter starts emitting them. This lands as profile
version `1.8.0` together with the optional `docstring` attr.

Verified by: `.venv/bin/python -m pytest tests -k rm_profile`

#### Scenario: Unmarked test raises no ORPHAN_NODE
- **WHEN** validation runs with the RM profile and a `test` node has no edges
- **THEN** no `ORPHAN_NODE` finding is emitted for it

#### Scenario: Other kinds still orphan-checked
- **WHEN** an `sc` node has no edges
- **THEN** an `ORPHAN_NODE` finding is emitted for it, as before
