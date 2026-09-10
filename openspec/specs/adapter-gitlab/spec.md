# adapter-gitlab Specification

## Purpose
Reads GitLab issues and their issue links through `glab api` into an interface
document, so a GitLab project can serve as a typed lattice register without
copying its issue data into repository files.

## Requirements
### Requirement: Load mapping policy from the resolved profile

The adapter SHALL read a core-resolved JSON profile with `resolved_schema: "1"`.
It SHALL take the project override, label-to-node-kind map, required default node
kind, description edge patterns, issue-link mappings, and the attrs declared by
each node kind from that document. An empty `adapter.link_type_map` SHALL mean
GitLab's standing mappings: `relates_to` becomes a forward `relates_to` edge,
`is_blocked_by` becomes a forward `depends_on` edge, and `blocks` becomes a
reversed `depends_on` edge. In an explicit mapping, `reverse` SHALL default to
false.

The adapter SHALL reject an unreadable or malformed profile, a document whose
resolved schema is not `1`, an empty default kind or configured project, a mapped
or default node kind absent from `node_kinds`, and an edge pattern whose edge
kind is empty or undeclared, whose regex is invalid, or whose regex has no capture
group. These are broken setup, not target findings: the adapter SHALL write an
error to stderr and exit 2 rather than emit a partial interface document.

Verified by: `cargo test -p adapter-gitlab`, and `.venv/bin/python -m pytest tests/test_adapter_gitlab.py`

#### Scenario: Link mappings are omitted

- **WHEN** the resolved profile declares no `adapter.link_type_map`
- **THEN** `relates_to`, `is_blocked_by`, and `blocks` use the standing mappings,
  including direction reversal for `blocks`

#### Scenario: Description pattern has no target capture

- **WHEN** an `adapter.edge_patterns` regex has no capture group for the target IID
- **THEN** profile loading fails and the adapter exits 2 without emitting a
  interface document

#### Scenario: Adapter names an undeclared node kind

- **WHEN** `adapter.default_kind` or a value in `adapter.label_map` names no
  declared node kind
- **THEN** profile loading fails and the adapter exits 2

### Requirement: Resolve the GitLab project and address it as one API identifier

The adapter SHALL use `adapter.project` when configured. Otherwise it SHALL run
`git -C <target> config --get remote.origin.url` and derive the namespace/project
path from an origin using GitLab.com's scp-style SSH, `ssh`, `https`, or `http`
URL form. It SHALL trim whitespace and a trailing slash, remove one trailing
`.git`, and require a nonempty namespace and project separated by `/`. A missing,
unreadable, non-UTF-8, or unsupported origin SHALL produce a `PARSE_ERROR` at
provenance `.` and no API request; project discovery is input identification, so
the adapter still emits a document and exits 0.

The complete namespace/project path SHALL be percent-encoded as one GitLab API
identifier — including `/` as `%2F` — before it is placed beneath `projects/`.
The adapter SHALL fetch `projects/<encoded-project>/issues` with `--paginate`,
delegating pagination to `glab`, because an unpaginated first page is a
plausible-looking partial register.

Verified by: `cargo test -p adapter-gitlab parses_supported_gitlab_remotes`,
`cargo test -p adapter-gitlab encodes_the_complete_project_path_as_one_api_identifier`, and
`.venv/bin/python -m pytest tests/test_adapter_gitlab.py -k gate`

#### Scenario: Project is discovered from HTTPS origin

- **WHEN** the target's origin is
  `https://gitlab.com/test-group/test-project.git`
- **THEN** the adapter requests
  `projects/test-group%2Ftest-project/issues --paginate`

#### Scenario: Profile supplies the project

- **WHEN** `adapter.project` names a project
- **THEN** the adapter uses that project without reading the target's origin

#### Scenario: Origin is not a supported GitLab.com URL

- **WHEN** project discovery reads an origin outside the supported GitLab.com URL
  forms
- **THEN** the adapter emits a `PARSE_ERROR` at `.`, emits no nodes, and exits 0

### Requirement: Append one typed node per decodable GitLab issue

The adapter SHALL append one node for every issue response that decodes with an
`iid`, title, state, and string-label list. Its ID SHALL be `#<iid>` using GitLab's
project-scoped `iid`, not its instance-wide issue number. The adapter SHALL NOT
deduplicate repeated IIDs: preserving every declaration lets the core report a
duplicate instead of allowing ingest to silently displace one response.

The first label in the response whose exact text appears in
`adapter.label_map` SHALL choose the node kind; if none does, the required
`adapter.default_kind` SHALL choose it. Provenance SHALL be
`gitlab:<namespace/project>#<iid>` at line 0, giving every node and issue-derived
edge a stable source even though the data did not come from a file.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_gitlab.py -k "interface_version_and_all_issue_nodes or labels_choose_node_kinds or provenance_names"`, and
`cargo test -p adapter-gitlab maps_gitlab_fields_labels_and_description_edges`

#### Scenario: A mapped label chooses the kind

- **WHEN** an issue's labels contain `requirement` and the label map assigns it
  to the `requirement` kind
- **THEN** the adapter emits a `requirement` node with ID `#<iid>`

#### Scenario: No label is mapped

- **WHEN** none of an issue's labels appears in `adapter.label_map`
- **THEN** the adapter emits the issue with `adapter.default_kind`

#### Scenario: Two responses carry the same IID

- **WHEN** two decodable API rows declare the same project-scoped IID
- **THEN** the adapter appends both nodes and leaves duplicate detection to the
  core

### Requirement: Carry only attrs declared for the selected GitLab kind

For an attr declared on the selected node kind, the adapter SHALL map the GitLab
title to `summary`, pass `state` through unchanged, map an assignee to the
assignee's username, and carry GitLab labels as a list of strings in `labels`.
It SHALL omit `assignee` when GitLab supplies none. It SHALL also omit each of
these fields when the selected kind does not declare the corresponding attr,
even if the API response contains it; the profile owns the graph vocabulary.
GitLab labels SHALL remain `labels`, not be renamed to or duplicated as `tags`,
and response fields for which the adapter has no mapping SHALL not become attrs.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_gitlab.py -k gitlab_attrs_use_title_username_and_plain_string_labels`

#### Scenario: Kind declares all GitLab attrs

- **WHEN** an assigned issue's selected kind declares `summary`, `state`,
  `assignee`, and `labels`
- **THEN** the node carries its title, state, assignee username, and plain string
  labels under those four attrs

#### Scenario: Kind does not declare labels

- **WHEN** a GitLab issue has labels but its selected kind does not declare the
  `labels` attr
- **THEN** the node carries no `labels` or `tags` attr

#### Scenario: Issue is unassigned

- **WHEN** GitLab returns no assignee for an issue whose kind declares `assignee`
- **THEN** the adapter omits `assignee` rather than inventing a value

### Requirement: Extract configured edges from issue descriptions

For every configured `adapter.edge_patterns` regex, the adapter SHALL find every
non-overlapping match in an issue's description and parse capture group 1 as the
target IID. Each valid capture SHALL append an edge from the current `#<iid>` to
the captured `#<iid>` with the pattern's configured edge kind and the current
issue's provenance. A missing description SHALL contribute no text edges and is
not a finding.

The adapter SHALL NOT require a captured target to appear in the fetched issue
set: endpoint resolution belongs to core validation, where an absent target can
surface as `VACANCY`. A match whose first capture is absent or is not an
unsigned integer SHALL instead produce a `PARSE_ERROR` on the source issue and no
edge for that match, while later matches and issues continue.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_gitlab.py -k description_patterns_and_issue_links_become_edges`, and
`cargo test -p adapter-gitlab maps_gitlab_fields_labels_and_description_edges`

#### Scenario: Description names a dependency

- **WHEN** a configured pattern `depends on #(\d+)` matches `depends on #2` in
  issue `#1`
- **THEN** the graph contains a `depends_on` edge from `#1` to `#2`

#### Scenario: Description is absent

- **WHEN** an issue has no description
- **THEN** the adapter emits its node and no description-derived edge or issue

#### Scenario: Capture is not an IID

- **WHEN** a configured regex matches but capture group 1 cannot be parsed as an
  unsigned integer
- **THEN** the adapter emits a `PARSE_ERROR` naming the pattern and captured text
  and creates no edge for that match

### Requirement: Fetch issue links with bounded concurrency

The adapter SHALL request `projects/<encoded-project>/issues/<iid>/links` once for
every decodable issue. It SHALL use at most eight scoped worker threads — or one
per issue when fewer than eight issues were fetched — so a large register does
not create an unbounded thread or request fan-out. Results SHALL remain associated
with their source issue and SHALL be applied in issue-response order after all
workers finish.

A normal link request failure, including a non-zero `glab` status or malformed
link JSON, SHALL produce a `PARSE_ERROR` at that issue's GitLab provenance. The
node and other issues SHALL remain in the document and the adapter SHALL exit 0.
A worker panic is not an input finding: the joining thread SHALL resume that panic
so the adapter fails non-zero rather than emitting a plausible partial document;
the core surfaces an adapter-process failure as exit 2.

Verified by: `cargo test -p adapter-gitlab link_fetches_are_bounded_and_failures_stay_with_their_issues`, and
`.venv/bin/python -m pytest tests/test_adapter_gitlab.py -k "description_patterns_and_issue_links_become_edges or glab_api_failure"`

#### Scenario: More than eight issues are fetched

- **WHEN** sixteen valid issues require link requests
- **THEN** link fetching reaches no more than eight concurrent workers and each
  result stays with the issue whose IID was requested

#### Scenario: One issue's link request fails

- **WHEN** the link request for issue `#7` fails while the other requests succeed
- **THEN** all sixteen nodes are emitted and a `PARSE_ERROR` with issue `#7`'s
  provenance records that request failure

#### Scenario: A link worker panics

- **WHEN** a link-fetch worker panics
- **THEN** the adapter propagates the panic and does not emit a partial interface document
  document as a successful read

### Requirement: Map only confirmed local issue links to directed edges

The adapter SHALL create an issue-link edge only when both the source issue and
linked issue carry `project_id`, those IDs are equal, and the linked IID appears
among the decodable issues fetched for this project. It SHALL then look up the
GitLab `link_type` in `adapter.link_type_map`, use the mapping's edge kind, and
emit current-to-linked endpoints unless `reverse` is true, in which case it SHALL
swap them. Thus the standing mapping reads “A blocks B” as a `depends_on` edge
from B to A, while `is_blocked_by` reads as a `depends_on` edge from the current
issue to the linked issue.

If either project identity is absent, the adapter SHALL emit a `PARSE_ERROR` and
no edge: an IID alone cannot establish locality because GitLab IIDs are only
project-scoped. A different project ID, or a linked IID absent from the local
issue set, SHALL instead emit an info-severity `EXTERNAL_REF` and no edge. A
confirmed local link whose `link_type` has no mapping SHALL produce a
`PARSE_ERROR` and no edge. These checks prevent a cross-project IID collision
from becoming a false local relationship.

Verified by: `cargo test -p adapter-gitlab cross_project_link_is_reported_without_creating_an_edge`,
`cargo test -p adapter-gitlab missing_project_identity_is_reported_without_creating_an_edge`, and
`.venv/bin/python -m pytest tests/test_adapter_gitlab.py -k description_patterns_and_issue_links_become_edges`

#### Scenario: Forward local relation

- **WHEN** issue `#2` has a confirmed-local `relates_to` link to issue `#1`
- **THEN** the adapter emits a `relates_to` edge from `#2` to `#1`

#### Scenario: Reverse mapping

- **WHEN** a confirmed-local link uses `blocks` and the standing mapping applies
- **THEN** the adapter emits a `depends_on` edge from the linked issue to the
  current issue

#### Scenario: Link crosses projects

- **WHEN** source and linked issues carry different `project_id` values, even if
  the linked IID collides with one in the local issue set
- **THEN** the adapter emits an info-severity `EXTERNAL_REF` and no edge

#### Scenario: Project identity is missing

- **WHEN** either side of an issue link omits `project_id`
- **THEN** the adapter emits a `PARSE_ERROR` explaining that locality cannot be
  determined and creates no edge

#### Scenario: Local link type is not configured

- **WHEN** a confirmed-local link has a `link_type` absent from
  `adapter.link_type_map`
- **THEN** the adapter emits a `PARSE_ERROR` naming that link type and creates no
  edge

### Requirement: Report malformed API input without losing sound issues

The adapter SHALL decode the paginated issues response first as a JSON array and
then decode each element as a GitLab issue. If the complete issues response is
malformed JSON, `glab` cannot be run, or `glab api` exits non-zero, the adapter
SHALL emit one `PARSE_ERROR` at `gitlab:<namespace/project>`, emit no nodes, write
the interface document, and exit 0. If one array element has a malformed issue
shape, the adapter SHALL emit a `PARSE_ERROR` identifying its one-based response
position, skip only that element, and continue with every decodable issue.

This boundary is deliberate: API data and command availability are facts about
the attempted input read and must not disappear as an adapter crash. The adapter
SHALL emit interface version `1.2`; a successfully written document means adapter
exit 0 even when its only content is a `PARSE_ERROR`. Profile-loading,
serialization, stdout-write, and propagated panic failures SHALL instead exit 2.
The adapter itself SHALL NOT use exit 1; that is the core's result for validation
findings after it has successfully received a document.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_gitlab.py -k "glab_api_failure or malformed_api_response or missing_glab"`, and
`cargo test -p adapter-gitlab malformed_issue_is_reported_without_losing_valid_issues`

#### Scenario: Issues endpoint returns malformed JSON

- **WHEN** `glab api` succeeds but its issues response is not valid JSON
- **THEN** the adapter emits a project-level `PARSE_ERROR`, emits no nodes, and
  exits 0

#### Scenario: One issue row is malformed

- **WHEN** one response element cannot decode as an issue and a later element is
  sound
- **THEN** the adapter emits a `PARSE_ERROR` for the malformed element and still
  emits the sound issue's node

#### Scenario: glab is unavailable

- **WHEN** the project is known but the `glab` program cannot be run
- **THEN** the adapter writes an interface document carrying `PARSE_ERROR` and exits
  0

### Requirement: Serve as the hermetic GitLab gate

`lattice validate` run with `profiles/gitlab.yaml` and the GitLab adapter against
the builder-constructed `tests/fixtures/mini-gitlab` repository SHALL exit 0 and
produce no error-severity findings. The fixture's `glab` stub SHALL prove that the
issues endpoint is project-encoded and paginated and that per-issue link endpoints
are called without the issues pagination flag. The adapter-level fixture checks
SHALL additionally establish interface version `1.2`, exactly nodes `#1` through
`#4`, label-selected and default kinds, declared attrs and provenance, and both
description-derived and issue-link-derived edges. Checking graph content beside
the clean finding result prevents an empty read from passing as success.

The fixture is hermetic: the test copies it into a temporary git repository,
adds the GitLab.com origin, and places its stub ahead of the ambient `PATH`, so
the gate requires neither network access nor an installed live `glab` client.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_gitlab.py -k gate`, and
`.venv/bin/python -m pytest tests/test_adapter_gitlab.py`

#### Scenario: Gate run over the fixture

- **WHEN** `lattice validate` runs with the GitLab profile and adapter against the
  temporary repository built from `tests/fixtures/mini-gitlab`
- **THEN** it exits 0, reports no error-severity findings, and the adapter fixture
  checks establish the declared nonempty node and edge content

#### Scenario: Adapter is a program that exits 0

- **WHEN** the adapter is invoked directly with `--profile` and `--target` and
  the stub returns malformed API input
- **THEN** it writes an interface document to stdout and exits 0, carrying the
  malformed input as `PARSE_ERROR` rather than using a non-zero adapter exit
