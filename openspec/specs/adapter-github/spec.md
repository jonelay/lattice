# adapter-github Specification

## Purpose
Reads a GitHub repository's issues through `gh api` into an interface document, so
lattice can validate and query a hosted issue register without copying GitHub's
labels, body references, or milestones into repository files.

## Requirements
### Requirement: Resolve the repository and fetch every issue through gh

The adapter SHALL use `adapter.repo` when the resolved profile supplies an
`owner/repo` slug. Otherwise it SHALL run `git config --get remote.origin.url` in
the target and derive that slug from an HTTPS, HTTP, scp-style SSH, or `ssh://`
GitHub origin, removing a `.git` suffix or trailing slashes before validating the
slug. The target is the repository context, not the data source: GitHub owns the
issue register, and the origin identifies which register belongs to that
checkout.

The adapter SHALL run
`gh api repos/<owner>/<repo>/issues --paginate -q .[]` from the target. It SHALL
consume every JSON value in the response stream and SHALL also flatten a JSON
array value, because `gh` and the hermetic fixture may frame the same issue set
differently without changing its meaning.

A missing or unreadable origin, a non-GitHub origin, a `git` or `gh` program that
cannot be run, or a non-zero `gh api` result SHALL produce one `PARSE_ERROR` and
an otherwise unpopulated read, then exit 0. These are unreadable register input,
not permission to report an empty repository as sound.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_github.py -k "gate or gh_api_failure or missing_gh"`, and `cargo test -p adapter-github`

#### Scenario: Repository derived from origin

- **WHEN** the profile has no `adapter.repo` and the target's origin is
  `https://github.com/test-org/test-repo`
- **THEN** the adapter requests `repos/test-org/test-repo/issues` through `gh api`

#### Scenario: Profile supplies the repository

- **WHEN** the resolved profile declares `adapter.repo: test-org/test-repo`
- **THEN** the adapter uses that slug without consulting the target's origin

#### Scenario: gh cannot fetch the register

- **WHEN** `gh api` cannot be run or exits non-zero
- **THEN** the adapter emits a `PARSE_ERROR` naming the GitHub repository, emits
  no nodes, and exits 0

### Requirement: Build one typed node per GitHub issue

The adapter SHALL emit one node per well-formed issue, with ID `#<number>` and
provenance `github:<owner>/<repo>#<number>` at line 0. The number is GitHub's
positive integer issue number; prefixing it with `#` preserves the public name
used in issue prose and is the join key that body references name.

The node kind SHALL be the profile mapping for the first issue label, in GitHub's
label order, that appears in `adapter.label_map`. If no label maps, the adapter
SHALL use `adapter.default_kind`. Label meaning is profile data rather than
adapter vocabulary: the real profile can map `requirement` and `bug` while still
retaining an ordinary `issue` fallback.

An issue with neither a mapped label nor a default kind SHALL produce a
`PARSE_ERROR` at that issue's public GitHub provenance and SHALL NOT produce a
node, edges, or milestone contribution. Inventing a kind would conceal missing
profile coverage.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_github.py -k "labels_select_node_kinds or default_kind_handles_unmapped_labels or provenance_uses_public_github_location"`

#### Scenario: First mapped label selects the kind

- **WHEN** an issue's ordered labels include `requirement` and the profile maps
  that label to the `requirement` kind
- **THEN** the adapter emits a `requirement` node whose ID is the issue number
  prefixed with `#`

#### Scenario: Unmapped labels use the default

- **WHEN** an issue has no label present in `adapter.label_map` and the profile
  declares `default_kind: issue`
- **THEN** the adapter emits the node with kind `issue`

#### Scenario: No kind can be selected

- **WHEN** an issue has no mapped label and the profile declares no
  `default_kind`
- **THEN** the adapter emits a `PARSE_ERROR` naming that issue and emits no node
  for it

### Requirement: Carry only attrs declared for the selected kind

The adapter SHALL offer the GitHub title as `summary`, issue state as `state`,
the ordered label names as `labels`, and the first assignee's string `name` or
`login` as `assignee`. It SHALL insert each value only when the selected node
kind declares that attr. GitHub labels remain the `labels` attr; the adapter
SHALL NOT silently rename them to `tags`. Attr vocabulary and availability are
profile data, so a kind such as `bug` can deliberately carry fewer fields than a
`requirement` even when GitHub returned both.

An issue with no assignee SHALL omit `assignee`. When an issue has a milestone,
the adapter SHALL also offer the milestone title under every profile-declared
pathway name, again only when that node kind declares an attr of that name.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_github.py -k attrs_are_populated_and_filtered_by_kind`

#### Scenario: Requirement carries its declared attrs

- **WHEN** a requirement issue has a title, state, two labels, and an assignee,
  and its kind declares all four corresponding attrs
- **THEN** the node carries `summary`, `state`, the ordered `labels` list, and
  the first `assignee`

#### Scenario: Kind omits GitHub fields

- **WHEN** a bug has labels and an assignee but the `bug` kind declares only
  `summary` and `state`
- **THEN** the node carries only `summary` and `state`

#### Scenario: Issue has no assignee

- **WHEN** an issue's `assignees` array is empty
- **THEN** its node has no `assignee` attr

### Requirement: Extract profile-defined edges from issue bodies

For every regex in `adapter.edge_patterns`, the adapter SHALL emit one edge for
every match in the issue body. The edge SHALL run from the issue's `#<number>`
ID to `#<first capture group>`, with the edge kind mapped to that regex by the
profile and with the source issue's GitHub provenance. The first capture group
is the target because the surrounding words explain the relationship while the
captured issue number is the join key.

A missing or null body SHALL be treated as empty text. The adapter SHALL NOT
check that a captured endpoint exists: an absent issue is a `VACANCY`
finding at validation, and dropping the edge would hide it. A capture group that does not participate in the match SHALL be silently
skipped. A capture that is present but is not an unsigned integer SHALL
instead produce a `PARSE_ERROR` at the source issue and no edge for that
match.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_github.py -k body_references_become_edges`

#### Scenario: Dependency text becomes an edge

- **WHEN** issue `#1` contains `depends on #2` and the matching profile regex
  maps to `depends_on`
- **THEN** the graph contains a `depends_on` edge from `#1` to `#2`

#### Scenario: Multiple matches are retained

- **WHEN** a configured regex matches more than once in one issue body
- **THEN** the adapter emits one edge per match in body order

#### Scenario: Body is absent

- **WHEN** an issue's body is missing or null
- **THEN** the adapter emits the node and no body-derived edges for it

### Requirement: Derive ordering pathways from milestones

The adapter SHALL collect milestones from the well-formed issues it emits,
ordered by positive milestone number, and SHALL use their titles as the order
for every pathway named by the resolved profile. Repeated milestone titles SHALL
appear once in that order. Milestones are the hosted register's ordering state;
the adapter SHALL NOT maintain a second hand-written order in its own code.

For every emitted pathway, `current` SHALL be the title of the lowest-numbered open
milestone. If none is open, it SHALL be the title of the highest-numbered
milestone. If no emitted issue has a milestone, the adapter SHALL attach no
pathway. A profile declaring no pathways likewise yields no pathways, even when issues have
milestones.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_github.py`

#### Scenario: Open milestone selects current

- **WHEN** emitted issues contribute closed milestone `v0.9` number 1 and open
  milestone `v1.0` number 2, and the profile names a pathway
- **THEN** that pathway has order `[v0.9, v1.0]` and current `v1.0`

#### Scenario: Every milestone is closed

- **WHEN** all collected milestones are closed
- **THEN** each configured pathway uses the highest-numbered milestone's title as
  current

#### Scenario: No milestones are collected

- **WHEN** no emitted issue has a milestone
- **THEN** the graph carries no pathways

### Requirement: Reject malformed issue responses without losing sound issues

Each non-pull-request response SHALL be an object with a positive integer
`number`, string `title` and `state`, array `labels`, and array `assignees`.
Items in those arrays SHALL carry a string `name` or `login`. `body` SHALL be a
string, null, or absent; `milestone` SHALL be an object with a positive integer
`number` and string `title` and `state`, or it SHALL be null or absent.

A response that violates this shape SHALL produce a `PARSE_ERROR` naming its
one-based response position and SHALL be skipped while later responses continue
to be read. A malformed JSON value in the `gh` output stream SHALL produce a
`PARSE_ERROR`, preserve values decoded before it, stop reading at the malformed
remainder, and exit 0. Malformed register data is a finding, not an adapter
failure.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_github.py -k malformed_json_from_gh_is_parse_error`

#### Scenario: One issue has the wrong field shape

- **WHEN** one response has a non-array `labels` field among otherwise sound
  issue responses
- **THEN** the adapter emits a `PARSE_ERROR` for that response, skips its node,
  and continues with the remaining responses

#### Scenario: gh emits malformed JSON

- **WHEN** the `gh api` output cannot be decoded as a complete JSON value stream
- **THEN** the adapter emits a `PARSE_ERROR` naming the malformed response and
  exits 0

### Requirement: Filter pull requests returned by the issues endpoint

The GitHub issues endpoint also returns pull requests. The adapter SHALL skip
any response object containing the `pull_request` key before issue-field
validation, and SHALL emit neither a node nor a `PARSE_ERROR` for it. Pull
requests are not issue-register entries merely because GitHub exposes them
through the same endpoint.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_github.py -k pull_requests_are_skipped_without_a_parse_error`

#### Scenario: Issues endpoint includes a pull request

- **WHEN** a response object contains the `pull_request` key
- **THEN** the adapter emits no node, edge, milestone contribution, or parse
  finding for that response

### Requirement: Load only a valid core-resolved adapter configuration

The adapter SHALL read a JSON profile with `resolved_schema: "1"`. A configured
repository SHALL have exactly two non-empty slash-separated parts. Every kind
named by `adapter.label_map` or `adapter.default_kind` SHALL exist in
`node_kinds`; every kind named by `adapter.edge_patterns` SHALL exist in
`edge_kinds`; and every edge regex SHALL compile and contain a capture group.
These checks happen before GitHub is read, because an invalid mapping is a
broken adapter setup rather than a finding about the target register.

An unreadable, unparseable, unresolved, or internally inconsistent profile
SHALL make the adapter exit 2 rather than emit an interface document. For a valid
profile, the adapter SHALL write a version `1.2` interface document and exit 0,
including when GitHub input produced `PARSE_ERROR` issues.

Verified by: `cargo test -p adapter-github`, and `.venv/bin/python -m pytest tests/test_adapter_github.py -k "interface_version_is_1_2 or missing_gh_is_parse_error_with_zero_exit"`

#### Scenario: Edge pattern has no capture group

- **WHEN** a resolved profile configures a compilable edge regex without a
  capture group
- **THEN** the adapter rejects the profile and exits 2 before invoking GitHub

#### Scenario: Mapping names an undeclared kind

- **WHEN** a label, default, or edge mapping names a kind absent from its
  corresponding profile kind declarations
- **THEN** the adapter rejects the profile and exits 2

#### Scenario: Malformed input still yields an interface document

- **WHEN** the profile is valid but GitHub input is malformed
- **THEN** the adapter writes a version `1.2` interface document carrying the
  `PARSE_ERROR` and exits 0

### Requirement: Serve as the GitHub adapter gate

`lattice validate` run with the GitHub profile and adapter against the
builder-constructed mini-GitHub repository, with its stubbed `gh`, SHALL exit 0
and produce no findings. The companion adapter assertions SHALL retain exactly
the fixture's four issue-node IDs, SHALL check its mapped and default kinds and
representative attrs, and SHALL retain both declared body-reference edges. A
clean finding count alone is insufficient: an adapter that silently read no
issues would also have nothing to validate.

The fixture is hermetic: its GitHub API response and command behavior live under
`tests/fixtures/mini-github`, so the gate requires neither network access nor a
live GitHub repository.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_github.py -k gate`, and `.venv/bin/python -m pytest tests/test_adapter_github.py`

#### Scenario: Gate run over the fixture

- **WHEN** `lattice validate` runs with the GitHub profile and adapter against
  the mini-GitHub target while the fixture `gh` is first on `PATH`
- **THEN** it exits 0 with no findings, after the adapter itself has exited 0

#### Scenario: Fixture graph is nonempty and exact

- **WHEN** the adapter reads the fixture response directly
- **THEN** the graph contains issue nodes `#1`, `#2`, `#3`, and `#4`, excludes
  pull request `#5`, and carries the fixture's mapped edges and attrs
