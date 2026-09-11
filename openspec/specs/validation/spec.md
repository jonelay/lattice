# validation Specification

## Purpose
The validation framework that checks a loaded graph against its profile — severity-tagged
issue codes, built-in validators, adapter-issue channel, and strict-mode enforcement.
## Requirements

### Requirement: Issue model
A validation finding SHALL be an `Issue` with fields: `severity` (error, warning, info,
hint), `code` (string identifier like `UNREFERENCED`), `message` (human-readable),
`provenance` (file + line from the graph element), and `node_id` (the node the finding
concerns, or null for findings not tied to a single node). An issue MAY additionally
carry a `state` (string): a machine-readable qualifier of what the finding established,
set only by validators that define one. An issue with no state carries none — the field
is absent, not null-valued, so findings that never had one are byte-identical to before
the field existed.

#### Scenario: Issue formatting
- **WHEN** an issue with severity=warning, code="UNREFERENCED", provenance=("REQS.md", 42) is formatted
- **THEN** the output includes severity, code, file, line, and message

#### Scenario: Node-scoped finding carries node_id
- **WHEN** validation emits an `ID_FORMAT` issue for node "REQ-1"
- **THEN** the issue's `node_id` is "REQ-1"

#### Scenario: Absent state is absent, not null
- **WHEN** an `UNREFERENCED` issue is serialized to JSON
- **THEN** the entry carries no `state` key

### Requirement: Built-in validators
Core SHALL provide these validators, each identified by a code:

- `ID_FORMAT`: node ID does not match its kind's `id_pattern`
- `UNKNOWN_KIND`: node or edge kind not declared in the profile
- `EDGE_CONSTRAINT`: edge kind used between disallowed source/target kinds.
  The issue SHALL carry `node_id` set to the edge's source node
- `VACANCY`: edge references a node ID that does not exist in the graph.
  The issue SHALL carry `node_id` set to the edge's source node
- `UNREFERENCED`: node has no incoming edges
- `UNTRACED`: node has no outgoing edges
- `ATTR_REQUIRED`: a required attr is missing from a node
- `ATTR_TYPE`: an attr value does not match its declared type
- `ATTR_ENUM`: an enum attr value is not in the declared `values` list
- `ATTR_LIST_ITEMS`: a list element does not match the declared `items` type
- `COVERAGE`: a node of the configured `target_kind` has no incoming edge of the
  configured `edge_kind` (warning by default). An optional `where` condition block
  restricts the target-kind nodes checked; absent `where`, every target is checked
- `SOURCE_MISSING`: a path cited by the register does not resolve on disk
  (warning by default). Emitted by adapters, not by core; core SHALL declare its
  default severity so a profile can override it like any other code
- `CONFIG_ERROR`: a profile's validation configuration is missing required keys or
  names a node/edge kind the profile does not declare (error by default)
- `PATHWAY_UNRESOLVED`: a profile binds a finding code to an ordering pathway the graph does
  not carry (warning by default). Emitted by core
- `PATHWAY_INVALID`: a register declares an ordering pathway that does not hold — a current
  position absent from its order, either value of the wrong shape, one of the pair
  declared without the other, or repeated positions in the order (warning by default).
  Emitted by adapters, not by core; core SHALL declare its default severity so a profile
  can override it like any other code
- `CONSTRAINT`: a node fails a cross-field constraint declared in the profile
  (warning by default). Core evaluates the constraint conditions against node
  attrs; no adapter involvement. A profile MAY declare multiple CONSTRAINT
  entries, each with its own condition set — every entry SHALL be honoured
  independently per the existing repeated-code contract
- `SUPPRESS_UNUSED`: a profile `SUPPRESS` entry whose selector matched no finding
  this run (info by default). Carries no `node_id`; provenance names the profile.
  Emitted by core after finding collection and before severity resolution, so a
  profile override to warning is reachable by `--strict`

A node with no edges in either direction SHALL receive both `UNREFERENCED` and
`UNTRACED` findings. The two codes are independent: a node may be unreferenced
(no incoming) while having outgoing edges, or untraced (no outgoing) while
having incoming edges.

A profile MAY configure the same validation code more than once. The profile's
`validations:` list SHALL be read as a sequence, and every entry SHALL be honoured
independently — a later entry for a code already seen SHALL NOT replace an earlier one.
Silently discarding a declared configuration is the failure this tool exists to prevent.

Severity is a per-code property: the profile holds one severity per code, not one per
entry. When multiple entries for the same code each declare `severity`, the last
declaration in the sequence sets the severity for all findings of that code. This is not
a replacement — the check each entry configures still runs independently, and every
finding it produces is reported. The severity declaration is about the code's weight, not
about which entries are heard.

A pathway binding is the one exception, and it is an exception by rejection rather than by
discard: a second binding for an already-bound code SHALL be a load error, per the
profile-schema spec. Every other repeated configuration for that code, binding or not,
SHALL still be honoured. Nothing declared is ever dropped.

`COVERAGE_DEEP` MAY carry the same optional `where` condition block. It restricts only
which target-kind nodes produce deep-coverage findings. Every target-kind node remains
part of the fixed-point traversal, including a non-matching child needed to establish a
matching parent's coverage.

#### Scenario: Deep coverage filter does not remove intermediates
- **WHEN** a matching parent is deep-covered through a non-matching target-kind child
  that has direct evidence
- **THEN** the parent is covered and no finding is emitted for either node

#### Scenario: Coverage gap
- **WHEN** the profile configures `COVERAGE` with `target_kind: req`, `edge_kind: verifies` and a `req` node has no incoming `verifies` edge
- **THEN** validation emits a `COVERAGE` issue naming that node

#### Scenario: Attribute-filtered coverage
- **WHEN** `COVERAGE` has `where: {status: {not: deferred}}` and uncovered target-kind
  nodes have statuses `active` and `deferred`
- **THEN** validation emits a `COVERAGE` issue only for the active node

#### Scenario: Coverage without where retains all targets
- **WHEN** `COVERAGE` omits `where` and two target-kind nodes are uncovered
- **THEN** validation checks both nodes

#### Scenario: Two coverage validations both apply
- **WHEN** a profile's `validations:` list declares `COVERAGE` twice, once with `edge_kind: verifies` and once with `edge_kind: fulfills`, and a `req` node has an incoming `verifies` edge but no incoming `fulfills` edge
- **THEN** validation emits exactly one `COVERAGE` issue for that node, naming `fulfills`

#### Scenario: Each coverage validation is checked independently
- **WHEN** a profile declares `COVERAGE` twice as above and a `req` node has neither edge
- **THEN** validation emits two `COVERAGE` issues for that node, one per configured `edge_kind`

#### Scenario: Coverage config names an undeclared kind
- **WHEN** a `COVERAGE` config sets `target_kind` to a kind not present in `node_kinds`
- **THEN** validation emits a `CONFIG_ERROR` issue and performs no coverage validation

#### Scenario: One bad config among several does not suppress the others
- **WHEN** a profile declares `COVERAGE` twice and only the first names an undeclared `target_kind`
- **THEN** validation emits a `CONFIG_ERROR` for the first and still performs the second config's coverage validation

#### Scenario: ID format violation
- **WHEN** a node of kind `req` (pattern `^REQ-\d{4}$`) has id "REQ-1"
- **THEN** validation emits an `ID_FORMAT` issue with the node's provenance

#### Scenario: Edge constraint violation
- **WHEN** an edge of kind `verifies` connects a `need` node to a `req` node, but the profile only allows `[test, req]`
- **THEN** validation emits an `EDGE_CONSTRAINT` issue with `node_id` set to the `need` node's ID

#### Scenario: Dangling reference
- **WHEN** an edge of kind `derives` connects `REQ-0604` to `RISK-001` and `RISK-001` does not exist
- **THEN** validation emits a `VACANCY` issue with `node_id` set to `REQ-0604`

#### Scenario: Unreferenced node
- **WHEN** a node has outgoing edges but no incoming edges and its kind is not `orphan_ok`
- **THEN** validation emits `UNREFERENCED` for that node

#### Scenario: Untraced node
- **WHEN** a node has incoming edges but no outgoing edges and its kind is not `orphan_ok`
- **THEN** validation emits `UNTRACED` for that node

#### Scenario: Fully disconnected node
- **WHEN** a node has no edges in either direction and its kind is not `orphan_ok`
- **THEN** validation emits both `UNREFERENCED` and `UNTRACED` for that node

#### Scenario: Connected node
- **WHEN** a node has both incoming and outgoing edges
- **THEN** validation emits neither `UNREFERENCED` nor `UNTRACED` for that node

#### Scenario: Missing required attr
- **WHEN** a `req` node lacks a `text` attr declared as `required: true`
- **THEN** validation emits an `ATTR_REQUIRED` issue

#### Scenario: Invalid enum value
- **WHEN** a node has attr `status: "unknown"` but the profile declares `values: [done, partial, todo, blocked]`
- **THEN** validation emits an `ATTR_ENUM` issue

#### Scenario: List item type mismatch
- **WHEN** a node has attr `tags: ["a", 42]` with `items: string`
- **THEN** validation emits an `ATTR_LIST_ITEMS` issue for element 42

#### Scenario: Valid date
- **WHEN** a node has attr `expires: "2030-01-15"` declared as `date`
- **THEN** validation emits no `ATTR_TYPE` issue for that attr

#### Scenario: Invalid date
- **WHEN** a node has attr `expires: "2030-02-29"` declared as `date`
- **THEN** validation emits an `ATTR_TYPE` issue identifying `date (YYYY-MM-DD)` as the
  expected type and `string` as the runtime type

#### Scenario: Pathway codes are profile-overridable
- **WHEN** a profile sets `PATHWAY_INVALID` severity to `info` and an adapter emits one
- **THEN** the report carries that issue at `info`

### Requirement: Strict mode
When `--strict` is passed, warnings SHALL be promoted to errors. Validation SHALL return
a non-zero exit code if any errors exist (after promotion). `--strict` SHALL NOT promote
`hint` or `info` findings: promotion reaches exactly the warning tier.

Pathway severity resolution SHALL run before `--strict` promotion. The order is load-bearing:
a finding demoted because it is not yet due becomes `info`, and `--strict` promotes only
warnings, so the demotion survives. A register's future obligations are not a reason for a
strict build to fail today. This holds because the demotion target is fixed at `info`; a
configurable target could land on `warning` and be promoted straight back to error.

Verified by: `cargo test --test validation strict`

#### Scenario: Strict mode promotes warnings
- **WHEN** validation runs with `--strict` and finds only warning-severity issues
- **THEN** the exit code is non-zero

#### Scenario: Non-strict mode allows warnings
- **WHEN** validation runs without `--strict` and finds only warning-severity issues
- **THEN** the exit code is zero

#### Scenario: Strict does not promote a pathway-demoted finding
- **WHEN** validation runs with `--strict` and a bound warning-severity finding was
  demoted because its position is after the current one
- **THEN** that finding remains `info` and does not by itself make the exit code non-zero

#### Scenario: Strict does not promote a hint
- **WHEN** validation runs with `--strict` and the only findings are hint-severity
- **THEN** those findings remain `hint` and the exit code is zero

### Requirement: Severity override in profile
Where the profile's `validations` section overrides the default severity of a built-in
validator (e.g. downgrade `UNREFERENCED` from warning to info), core SHALL apply the
overridden severity to that validator's findings. An override for a code core does not
implement SHALL apply the same way to adapter issues carrying that code — a recorded
override that changes nothing is the silently inert setting the profile-schema spec
rejects.

An override MAY demote any code *to* `hint`. An override SHALL NOT promote an issue that
arrives at `hint` to any other severity, whatever its code and whatever emitted it: hint
marks advice the tool itself cannot stand behind as a warning, and promoting it would
launder advice into a gate. Core SHALL report such an override as a `CONFIG_ERROR` naming
the code and leave the finding at `hint`, rather than ignoring the override in silence.
The `CONFIG_ERROR` is emitted once per offending code, not once per finding.

The rule is stated over the severity a finding *arrives at*, not over the code's shipped
default, because core has no default for a code it does not implement — a finding emitted at
`hint` by an adapter or a suggestion overlay would otherwise be promoted by an override with
no `CONFIG_ERROR` raised, which is promotion in exactly the silence this requirement forbids.

Verified by: `cargo test --test validation override` and
`cargo test --test suggestions promoted`

#### Scenario: Override severity
- **WHEN** the profile sets `UNREFERENCED` severity to `info` and validation finds orphans
- **THEN** the `UNREFERENCED` findings have severity `info`, not the default `warning`

#### Scenario: Override applies to an adapter code
- **WHEN** the profile sets `PARSE_ERROR` severity to `info` and the adapter emits a
  `PARSE_ERROR` issue with severity `warning`
- **THEN** the validation report carries that issue with severity `info`

#### Scenario: Demotion to hint is honoured
- **WHEN** the profile sets `UNREFERENCED` severity to `hint` and validation finds orphans
- **THEN** the `UNREFERENCED` findings have severity `hint` and never affect the exit code

#### Scenario: Promotion from a hint-default code is rejected
- **WHEN** the profile sets `COVERAGE_UNKNOWN` severity to `warning`
- **THEN** validation emits a `CONFIG_ERROR` naming the code and the findings stay `hint`

#### Scenario: Promotion of an externally emitted hint is rejected
- **WHEN** an adapter emits an issue at `hint` carrying a code core has no default for, and the
  profile overrides that code to `error`
- **THEN** validation emits a `CONFIG_ERROR` naming the code and the finding stays `hint`

#### Scenario: A demoting override on an external hint still applies
- **WHEN** an adapter emits an issue at `hint` and the profile overrides that code to `hint`
- **THEN** no `CONFIG_ERROR` is emitted and the finding stays `hint`

#### Scenario: An overlay hint cannot be promoted
- **WHEN** the profile overrides `SUGGESTED_EDGE` to `warning` and a suggestion renders
- **THEN** validation emits a `CONFIG_ERROR` naming the code, the finding stays `hint`, and the
  exit code is unaffected

#### Scenario: A refused promotion is reported once per code
- **WHEN** two findings arrive at `hint` under one code the profile overrides to `error`
- **THEN** validation emits exactly one `CONFIG_ERROR` for that code

### Requirement: Adapter issue channel
Core SHALL accept issues from adapters (parse errors, format warnings) through the same
`Issue` model so that adapter-time and graph-time findings appear in one unified report.

#### Scenario: Adapter parse error in report
- **WHEN** the adapter emits an issue with code="PARSE_ERROR", provenance=("REQS.md", 12)
- **THEN** the validation report includes this issue alongside graph validation findings

### Requirement: Suppress findings cascading from an unknown kind
A node whose kind is not declared in the profile SHALL produce a single `UNKNOWN_KIND`
finding. Core SHALL NOT additionally emit `EDGE_CONSTRAINT` findings for edges touching
that node, because its kind cannot be checked against any `allowed` pair.

#### Scenario: Unknown-kind node does not cascade
- **WHEN** a node has kind `mystery` (not in the profile) and an edge connects it to a `req` node
- **THEN** validation emits `UNKNOWN_KIND` for the node and no `EDGE_CONSTRAINT` for the edge

### Requirement: Pathway severity resolution
Where a profile binds a finding code to an ordering pathway, core SHALL resolve that code's
findings against the pathway after collection, and SHALL apply the resolution to findings from
adapters and built-in validators alike.

For a finding carrying a `node_id`, core SHALL read the bound `position_attr` from that
node and resolve as follows:

| the node's position value | resolution |
|---|---|
| a string in the pathway order, at or before the current position | severity unchanged — the finding is due |
| a string in the pathway order, after the current position | severity becomes `info` |
| a string that is **not a member** of the pathway order | severity becomes `info` |
| not a string (int, list, mapping, null) | severity unchanged |
| the attribute is absent | severity unchanged |

Membership in the order is the whole discriminator for string values. A position value the
register never placed on the pathway — a named event, a free-text condition — SHALL demote by
construction, so core never learns the host register's vocabulary for such values.

An absent attribute, or one carrying a non-string value, SHALL leave severity unchanged.
Demotion is a positive claim that a finding is provably not yet due; a node that does not
state its position, or states it in a shape the pathway cannot hold, has proven nothing.
Quieting either would be indistinguishable from dropping it. A wrong-shaped attr is
separately reported by the attribute-type validator where the profile types it.

Resolution SHALL be deterministic and independent of the order findings were collected in.

Verified by: `cargo test --test validation pathway`

#### Scenario: Finding at the current position stays
- **WHEN** the pathway current position is `M0` and a bound node's `trigger` is `M0`
- **THEN** the finding keeps its declared severity

#### Scenario: Finding before the current position stays
- **WHEN** the pathway current position is `M0` and a bound node's `trigger` is `CB`
- **THEN** the finding keeps its declared severity

#### Scenario: Finding after the current position is demoted
- **WHEN** the pathway current position is `M0` and a bound node's `trigger` is `M4`
- **THEN** the finding's severity becomes `info`

#### Scenario: Position value not on the pathway is demoted
- **WHEN** a bound node's `trigger` is `subscribe DbD (precedes M0 wiring)`, which is not
  a member of the pathway order
- **THEN** the finding's severity becomes `info`

#### Scenario: Non-string position value leaves severity unchanged
- **WHEN** a bound node's `trigger` is the integer `0` or a list
- **THEN** the finding keeps its declared severity

#### Scenario: Absent position attribute leaves severity unchanged
- **WHEN** a bound node carries no `trigger` attribute
- **THEN** the finding keeps its declared severity

#### Scenario: Unbound codes are untouched
- **WHEN** a profile binds only `OBLIGATION_UNBACKED` and validation also finds
  `UNREFERENCED`
- **THEN** the `UNREFERENCED` findings keep their declared severity

### Requirement: Findings the pathway pass cannot resolve
The severity-resolution pass SHALL leave a finding untouched when it cannot be resolved,
and SHALL NOT raise. A finding with no `node_id`, or whose `node_id` names no explicitly
added node, SHALL keep its declared severity.

Where a profile binds a code to a pathway the graph does not carry, core SHALL emit one
`PATHWAY_UNRESOLVED` finding **per bound code**, however many findings that code produced, and
SHALL leave every severity as declared. One finding for the pathway would name only one of the
codes bound to it. This SHALL NOT be treated as a setup failure: the profile loaded and the
adapter honoured its contract, so the mismatch is a finding about the pairing, not a
lattice that could not run.

The finding SHALL name the profile rather than a register file and line, as configuration
findings already do — no line of the register is responsible for it.

Verified by: `cargo test --test validation two_codes_bound_to_one_missing_pathway_each_report`
and `cargo test --test cli pathway`

#### Scenario: Finding without a node
- **WHEN** a bound code's finding carries no `node_id`
- **THEN** the finding keeps its declared severity and no error is raised

#### Scenario: Profile binds a pathway the graph lacks
- **WHEN** the profile binds `OBLIGATION_UNBACKED` to pathway `phase` and the graph carries
  no `phase` pathway
- **THEN** one `PATHWAY_UNRESOLVED` finding is emitted, naming the profile rather than a
  register file, and every other severity is as declared

#### Scenario: Two codes bound to one missing pathway each report
- **WHEN** two codes bind to pathway `phase` and the graph carries no `phase` pathway
- **THEN** two `PATHWAY_UNRESOLVED` findings are emitted, one naming each bound code

#### Scenario: An unresolved binding does not produce exit 2
- **WHEN** the CLI runs against a profile binding a pathway the graph lacks
- **THEN** the exit code reflects the findings' severities and is never 2

### Requirement: Severity resolution is shared by every command
Core SHALL expose severity resolution — profile overrides followed by pathway demotion — as
one operation, and every command that reports or exits on adapter issues SHALL apply it.
Two commands run against the same graph and profile SHALL NOT report different severities
for the same finding.

A per-command copy of this logic is how the formats and the exit codes diverge. The
requirement is on the observable outcome, not on any particular internal arrangement.

Verified by: `cargo test --test cli severity`

#### Scenario: Two commands agree on an adapter issue's severity
- **WHEN** the profile demotes an adapter issue via a pathway binding, and both `validate`
  and `summary` are run against the same graph
- **THEN** both report that issue at the same severity

### Requirement: Native message vocabulary
Finding messages that name a value's runtime type SHALL use the JSON vocabulary —
`string`, `int`, `float`, `bool`, `list`, `null`, and `object`. The declared attr-type
vocabulary additionally includes `date`; an invalid date SHALL therefore be reported as
expected `date (YYYY-MM-DD)`, got `string`, because it has no distinct JSON type.
Finding messages that quote a list of allowed values SHALL render it as a JSON array.
Python-dialect renderings
(`NoneType`, `str`, `dict`, `repr` quoting) SHALL NOT appear in core-emitted
messages. Message text SHALL NOT vary with which core produced it.

Verified by: `cargo test --test native_messages`

#### Scenario: Enum finding quotes the allowed list as JSON
- **WHEN** a node's enum attr value is not in the declared `values` list
- **THEN** the `ATTR_ENUM` message renders the allowed list as a JSON array
  (compact, double-quoted, JSON string escaping), e.g. `not in ["can't","done"]`

#### Scenario: Type mismatch names the native type
- **WHEN** a node's attr value does not match its declared type
- **THEN** the `ATTR_TYPE` message names the value's actual type in the native
  vocabulary, e.g. `expected type 'int', got string` — never `str` or `NoneType`

#### Scenario: List element mismatch names the native type
- **WHEN** a list attr element does not match the declared `items` type
- **THEN** the `ATTR_LIST_ITEMS` message names the element's actual type in the
  native vocabulary

### Requirement: Validation config values are typed
A `COVERAGE` config key that names a kind (`target_kind`, `edge_kind`), and a
`COVERAGE_DEEP` config key that names a kind (`target_kind`, `via`,
`evidence`), SHALL be read as a string. A non-string value SHALL be a
`CONFIG_ERROR` naming the key and the value's type in the native vocabulary,
and that config SHALL perform no check — it is never read as absent,
stringified, or dropped in silence. An empty string is a value, not an
absence, and fails the referenced-kind check like any other unknown name.
Each key SHALL be typed and checked against the profile's declared kinds
independently of its siblings: one missing or mistyped key SHALL NOT mask the
faults of another, and every fault the entry carries is reported. SUMMARY's
keys (`node_kind`, `status_attr`, `group_by_attr`) follow the same typing rule
through `summary`'s own error channel (see the `cli` capability), not as
findings.

Verified by: `cargo test --test native_messages config_typing`

#### Scenario: Non-string coverage config value
- **WHEN** a `COVERAGE` config sets `target_kind: true`
- **THEN** validation emits `CONFIG_ERROR` with message `COVERAGE config:
  'target_kind' must be a string, got bool` and performs no coverage validation for
  that config

#### Scenario: Null coverage config value is wrong type, not missing
- **WHEN** a `COVERAGE` config sets `edge_kind: null`
- **THEN** validation emits the wrong-type `CONFIG_ERROR` (`got null`), not the
  missing-keys one

#### Scenario: Empty-string coverage config value is checked, not dropped
- **WHEN** a `COVERAGE` config sets `target_kind: ""`
- **THEN** validation emits `CONFIG_ERROR "COVERAGE config: target_kind '' not
  in profile node kinds"`

#### Scenario: Mixed faults are all reported
- **WHEN** a `COVERAGE_DEEP` config sets `target_kind: 3` and omits `via`
- **THEN** validation emits a `CONFIG_ERROR` for the mistyped key and a
  `CONFIG_ERROR` for the missing key, and performs no check for that config

### Requirement: Hint severity tier
Core SHALL support a fourth severity, `hint`, below `info`. A hint SHALL never affect
the exit code, under any flag. `--strict` SHALL never promote it. It exists to carry
advice — states the tool can surface but cannot stand behind — without ever becoming a
gate.

Verified by: `cargo test --test validation hint`

#### Scenario: Hints never touch the exit code
- **WHEN** validation finds only hint-severity issues
- **THEN** the exit code is zero, with and without `--strict`

#### Scenario: Hint renders in the report
- **WHEN** validation finds a hint-severity issue
- **THEN** the report includes it — a hint is reported, never dropped

### Requirement: Per-kind orphan exemption
Where the profile marks a node kind `orphan_ok: true`, core SHALL NOT emit
`UNREFERENCED` or `UNTRACED` for nodes of that kind. The exemption is policy
about the kind — some kinds legitimately enter the register unconnected — and
SHALL NOT affect any other validator or query: `query orphans` still reports
such nodes as the ask-time fact they are.

Verified by: `cargo test --test validation orphan_ok`

#### Scenario: Exempt kind raises no directional orphan findings
- **WHEN** the profile marks kind `test` with `orphan_ok: true` and a `test` node has no edges
- **THEN** validation emits neither `UNREFERENCED` nor `UNTRACED` for that node

#### Scenario: Non-exempt kinds are unaffected
- **WHEN** the profile marks only `test` as `orphan_ok` and a `req` node has no edges
- **THEN** validation emits both `UNREFERENCED` and `UNTRACED` for the `req` node

### Requirement: Coverage evidence state
A `COVERAGE` finding SHALL carry a `state` distinguishing what the register can prove.
For a coverage config, an *unattributed candidate* is a node whose kind appears as a
source kind in the config's `edge_kind` allowed pairs and which has no outgoing edge of
that kind. When unattributed candidates exist in the graph, every `COVERAGE` finding the
config emits SHALL carry `state: "unknown"` — the gap may be attribution, not absence.
When none exist, the finding SHALL carry `state: "unverified"`. The finding's severity,
code, and message text SHALL NOT vary with the state, so `--strict` semantics and the
plain rendering of existing findings are unchanged.

In addition, when unattributed candidates exist, the config SHALL emit one
`COVERAGE_UNKNOWN` finding (shipped default severity `hint`) reporting the population:
the count, the base population it is drawn from with the ratio, and the kinds involved,
e.g. `12 of 40 node(s) of kind 'test' (30.0%) carry no outgoing 'verifies' edge; coverage
state for 'req' is unknown`. The base rides along because a bare numerator over an
unstated denominator reads as an unwired layer rather than a gap — the misreading was
observed on live data. The finding carries no `node_id`
and names the profile as provenance, as configuration-level findings do. One finding per
config, not per candidate: the population is one fact, and repeating it per node would
bury the per-node findings it qualifies.

Verified by: `cargo test --test validation coverage_state`

#### Scenario: Unverified with no candidates
- **WHEN** a `req` node has no incoming `verifies` edge and every `test` node has an
  outgoing `verifies` edge
- **THEN** the `COVERAGE` finding for that node carries `state: "unverified"` and no
  `COVERAGE_UNKNOWN` is emitted

#### Scenario: Unknown when unattributed candidates exist
- **WHEN** a `req` node has no incoming `verifies` edge and some `test` node has no
  outgoing `verifies` edge
- **THEN** the `COVERAGE` finding carries `state: "unknown"` and one `COVERAGE_UNKNOWN`
  hint reports the candidate count

#### Scenario: Verified nodes get no finding either way
- **WHEN** a `req` node has an incoming `verifies` edge from an existing `test` node
- **THEN** no `COVERAGE` finding is emitted for it, whatever the unattributed population

#### Scenario: Population is reported once per config
- **WHEN** three `req` nodes are unverified and five `test` nodes are unattributed
- **THEN** exactly one `COVERAGE_UNKNOWN` finding is emitted, naming the count 5

#### Scenario: The hint names the base population
- **WHEN** one of three `test` nodes carries no outgoing `verifies` edge
- **THEN** the `COVERAGE_UNKNOWN` message names `1 of 3` and the ratio `33.3%`

### Requirement: CONSTRAINT evaluation
Core SHALL evaluate each CONSTRAINT entry in the `validations:` list against every node
whose kind matches the entry's `kind` key. Evaluation is stateless: each rule examines
one node in isolation, carries no state between nodes, and never inspects another node.

The entry declares up to three condition blocks — `when`, `expect`, `reject` — each a
map of attr-name to condition operator:

- `when` (optional): a guard. If any condition in `when` does not match, the rule is
  skipped for that node — no finding is emitted.
- `expect` (required if no `reject`): all conditions SHALL hold, or a `CONSTRAINT`
  finding is emitted.
- `reject` (required if no `expect`): no condition SHALL hold, or a `CONSTRAINT`
  finding is emitted.
- A rule declaring both `expect` and `reject` SHALL require both to pass.
- A rule declaring neither `expect` nor `reject` SHALL be a load error
  (`CONFIG_ERROR`).

The finding SHALL carry `node_id` set to the failing node and `severity` from the
entry's `severity` key (default `warning`). The `message` SHALL be the entry's
`message` key if declared, or a generated message naming the first failing condition.

Verified by: `cargo test --test validation constraint`

#### Scenario: Constraint passes — no finding
- **WHEN** a profile declares `CONSTRAINT` with `kind: req`, `expect: { status: { present: true } }` and a `req` node has attr `status: "approved"`
- **THEN** validation emits no `CONSTRAINT` finding for that node

#### Scenario: Expect condition fails
- **WHEN** a profile declares `CONSTRAINT` with `kind: req`, `expect: { rationale: { present: true } }` and a `req` node has no `rationale` attr
- **THEN** validation emits a `CONSTRAINT` finding for that node

#### Scenario: Reject condition fires
- **WHEN** a profile declares `CONSTRAINT` with `kind: req`, `reject: { priority: { eq: "" } }` and a `req` node has attr `priority: ""`
- **THEN** validation emits a `CONSTRAINT` finding for that node

#### Scenario: When guard skips non-matching nodes
- **WHEN** a profile declares `CONSTRAINT` with `kind: req`, `when: { status: { eq: "approved" } }`, `expect: { rationale: { present: true } }` and a `req` node has `status: "draft"` and no `rationale`
- **THEN** no `CONSTRAINT` finding is emitted for that node

#### Scenario: When guard passes and expect fails
- **WHEN** a profile declares `CONSTRAINT` with `kind: req`, `when: { status: { eq: "approved" } }`, `expect: { rationale: { present: true } }` and a `req` node has `status: "approved"` and no `rationale`
- **THEN** validation emits a `CONSTRAINT` finding for that node

#### Scenario: Both expect and reject required
- **WHEN** a profile declares `CONSTRAINT` with `expect: { a: { present: true } }` and `reject: { b: { eq: "" } }`, and a node has attr `a: "yes"` and attr `b: ""`
- **THEN** validation emits a `CONSTRAINT` finding (reject failed, even though expect passed)

#### Scenario: Custom message
- **WHEN** a profile declares `CONSTRAINT` with `message: "Approved items need rationale"` and the constraint fails
- **THEN** the finding's message is "Approved items need rationale"

#### Scenario: Generated message names failing condition
- **WHEN** a profile declares `CONSTRAINT` with no `message` and `expect: { rationale: { present: true } }` fails
- **THEN** the finding's message names `rationale` and the failing operator

#### Scenario: Multiple CONSTRAINT entries honoured independently
- **WHEN** a profile declares two CONSTRAINT entries, each with different conditions
- **THEN** both are evaluated and each can independently emit findings

#### Scenario: Kind mismatch skips silently
- **WHEN** a profile declares `CONSTRAINT` with `kind: test` and a node of kind `req` is validated
- **THEN** no CONSTRAINT evaluation runs for that node

#### Scenario: Pathway demotion applies to CONSTRAINT
- **WHEN** a profile binds `CONSTRAINT` to a pathway and a node's position is not yet due
- **THEN** the finding is demoted to `info`

### Requirement: CONSTRAINT condition operators
The condition vocabulary is fixed at compile time. Core SHALL support these operators:

- `eq: <value>` — the attr's value equals `<value>` (string or number comparison)
- `not: <value>` — the attr's value does not equal `<value>`
- `in: [v1, v2, ...]` — the attr's value is one of the listed values
- `lt: <value>` — the attr's value is less than `<value>`
- `gt: <value>` — the attr's value is greater than `<value>`
- `lte: <value>` — the attr's value is less than or equal to `<value>`
- `gte: <value>` — the attr's value is greater than or equal to `<value>`
- `matches: <pattern>` — the attr's string value matches the regex `<pattern>`
- `present: true` — the attr exists on the node
- `present: false` — the attr does not exist on the node

An operator key the core does not recognise SHALL be a load error (`CONFIG_ERROR`),
not a silently inert condition.

For `eq`, `not`, and `in`: comparison SHALL be type-aware — a string `"42"` does not
match an integer `42`. For `lt`, `gt`, `lte`, and `gte`: the threshold value in the
profile SHALL be an integer or a string; any other type (array, object, boolean, null,
float) SHALL be a load error (`CONFIG_ERROR`). At evaluation time, two strings SHALL
compare lexicographically and two integers SHALL compare numerically. This orders
canonical `YYYY-MM-DD` dates chronologically. A missing attr or a type mismatch between
the node's attr and the threshold SHALL not compare and the condition SHALL fail.
For `matches`: the value SHALL be coerced to a string before matching; a non-string attr
that cannot be represented as text SHALL not match.

For `present`: only existence is checked, not the value. `present: true` on an attr
whose value is an empty string or zero passes — the attr exists.

Verified by: `cargo test --test validation constraint`

#### Scenario: eq matches
- **WHEN** `expect: { status: { eq: "approved" } }` and node attr `status` is `"approved"`
- **THEN** the condition passes

#### Scenario: eq does not match
- **WHEN** `expect: { status: { eq: "approved" } }` and node attr `status` is `"draft"`
- **THEN** the condition fails

#### Scenario: not operator
- **WHEN** `expect: { status: { not: "blocked" } }` and node attr `status` is `"approved"`
- **THEN** the condition passes

#### Scenario: in operator matches
- **WHEN** `expect: { status: { in: ["approved", "accepted"] } }` and node attr `status` is `"accepted"`
- **THEN** the condition passes

#### Scenario: in operator does not match
- **WHEN** `expect: { status: { in: ["approved", "accepted"] } }` and node attr `status` is `"draft"`
- **THEN** the condition fails

#### Scenario: matches operator
- **WHEN** `expect: { id: { matches: "^REQ-\\d{4}$" } }` and node attr `id` is `"REQ-0042"`
- **THEN** the condition passes

#### Scenario: present true — attr exists
- **WHEN** `expect: { rationale: { present: true } }` and the node has attr `rationale: ""`
- **THEN** the condition passes (attr exists, even though empty)

#### Scenario: present true — attr missing
- **WHEN** `expect: { rationale: { present: true } }` and the node has no `rationale` attr
- **THEN** the condition fails

#### Scenario: present false — attr missing
- **WHEN** `expect: { rationale: { present: false } }` and the node has no `rationale` attr
- **THEN** the condition passes

#### Scenario: Unknown operator rejected at load time
- **WHEN** a profile declares `expect: { status: { between: [1, 5] } }`
- **THEN** the loader emits a `CONFIG_ERROR` naming the unknown operator `between`

#### Scenario: Date comparison
- **WHEN** `expect: { expires: { lt: "2030-01-01" } }` and the date attr is `"2029-12-31"`
- **THEN** the condition passes

#### Scenario: Integer comparison
- **WHEN** `expect: { count: { gte: 5 } }` and the int attr is `5`
- **THEN** the condition passes

#### Scenario: Type-aware comparison
- **WHEN** `expect: { count: { eq: "42" } }` and node attr `count` is integer `42`
- **THEN** the condition fails (string "42" does not match integer 42)

### Requirement: Finding suppression
A profile MAY declare `SUPPRESS` entries in its `validations` list. Each entry
names a finding `code` and optionally a `node_ids` list. When `node_ids` is
present, only findings matching both the code and a listed `node_id` are
suppressed. When `node_ids` is absent, all findings of that code are
suppressed.

Suppression SHALL be applied after severity resolution (profile overrides and
pathway demotion) and after `--strict` promotion, before the exit-code
decision. A suppressed finding:

- SHALL NOT count toward exit code 1.
- SHALL carry `suppressed: true` in JSON output.
- SHALL be omitted from plain and rich output.
- SHALL retain its resolved severity, code, message, and provenance in JSON.

A suppress entry naming `CONFIG_ERROR` SHALL be rejected at load time. A
profile that silences configuration errors would defeat the invariant that
unreadable input is reported, never dropped.

Multiple suppress entries for the same code SHALL be honoured independently:
their `node_ids` lists merge (union). A suppress-all (no `node_ids`) for a
code supersedes any ID-specific entry for the same code.

`--strict` SHALL NOT unsuppress findings. Suppression is an explicit
declaration that a finding is structurally expected; `--strict` promotes
severity, not policy.

Verified by: `cargo test --test validation suppress`

#### Scenario: Suppress all findings of a code
- **WHEN** the profile suppresses `VACANCY` with no `node_ids` and validation
  produces 59 VACANCY findings
- **THEN** all 59 are suppressed: omitted from plain output, present in JSON
  with `suppressed: true`, and do not affect exit code

#### Scenario: Suppress specific node IDs
- **WHEN** the profile suppresses `VACANCY` with `node_ids: [UN-1, UN-2]` and
  validation produces VACANCY findings for UN-1, UN-2, and UN-3
- **THEN** UN-1 and UN-2 are suppressed; UN-3 is not

#### Scenario: Suppress-all supersedes an ID-specific entry
- **WHEN** the profile declares `SUPPRESS: {code: VACANCY, node_ids: [UN-1]}`
  and also `SUPPRESS: {code: VACANCY}`, and VACANCY findings exist for UN-1
  and UN-2
- **THEN** both findings are suppressed and no `SUPPRESS_UNUSED` is emitted for
  either entry

#### Scenario: Suppressed finding keeps its resolved fields in JSON
- **WHEN** a `VACANCY` finding for UN-1 at severity `error` is suppressed and
  rendered as JSON
- **THEN** the entry carries `"suppressed": true` alongside its `severity`,
  `code`, `message`, `file`, `line`, and `node_id`, unchanged from the
  unsuppressed rendering

#### Scenario: Suppressed finding does not affect exit code
- **WHEN** all error-severity findings are suppressed
- **THEN** the exit code is 0

#### Scenario: Strict does not unsuppress
- **WHEN** `--strict` is passed and a warning-severity finding is suppressed
- **THEN** the finding remains suppressed (not promoted and un-suppressed)

#### Scenario: CONFIG_ERROR suppression rejected at load
- **WHEN** the profile declares `SUPPRESS: {code: CONFIG_ERROR}`
- **THEN** the loader rejects the profile

### Requirement: SUPPRESS_UNUSED built-in code
Core SHALL provide the built-in code `SUPPRESS_UNUSED` (info by default),
emitted by core, carrying no `node_id` and naming the profile as provenance,
as configuration-level findings do.

After finding collection and before severity resolution, core SHALL check each
suppress entry at `(code, node_id)` granularity. A suppress entry whose
selector matched no finding SHALL produce one `SUPPRESS_UNUSED` finding. The
message SHALL state what the core knows: whether the code is a built-in
validator code, whether any finding carried it this run (even if `node_ids`
did not match), and which `node_ids` went unmatched. The core SHALL NOT claim
to distinguish a typo from a dormant suppression — it cannot know an
adapter's full code vocabulary, only what appeared this run.

`SUPPRESS_UNUSED` defaults to info because the observation is about profile
hygiene, not register correctness. It is an ordinary code: a profile MAY
override its severity, and `--strict` promotes it when it arrives at
`warning`, so a profile that overrides it to `warning` gates stale
suppressions under `--strict`. Unused detection therefore runs before
severity resolution — a `SUPPRESS_UNUSED` emitted after the override pass
could never be promoted.

Unused detection SHALL run once. The resulting `SUPPRESS_UNUSED` findings are
then subject to suppression through the normal mechanism, which makes
self-reference deterministic without special-casing.

Verified by: `cargo test --test validation suppress_unused`

#### Scenario: Unmatched suppress produces SUPPRESS_UNUSED
- **WHEN** the profile suppresses `TYPO_CODE` and no finding with that code
  was emitted
- **THEN** validation emits a `SUPPRESS_UNUSED` info naming the unmatched code
  and stating it is not a built-in code and no finding carried it

#### Scenario: Dormant built-in suppress produces SUPPRESS_UNUSED
- **WHEN** the profile suppresses `VACANCY` and validation produces zero
  VACANCY findings
- **THEN** validation emits a `SUPPRESS_UNUSED` info stating VACANCY is a
  built-in code that matched no finding

#### Scenario: Node-id-specific unmatched suppress
- **WHEN** the profile suppresses `VACANCY` with `node_ids: [UN-1]` and
  VACANCY findings exist for UN-2 and UN-3 but not UN-1
- **THEN** validation emits a `SUPPRESS_UNUSED` info for the UN-1 selector,
  stating the code was emitted but not for that node

#### Scenario: SUPPRESS_UNUSED names the profile as provenance
- **WHEN** a `SUPPRESS_UNUSED` finding is emitted
- **THEN** it carries no `node_id` and its provenance names the profile, not a
  register file

#### Scenario: SUPPRESS_UNUSED is overridable and strict-promotable
- **WHEN** the profile sets `SUPPRESS_UNUSED` severity to `warning`, one
  suppress entry matches nothing, and validation runs with `--strict`
- **THEN** the `SUPPRESS_UNUSED` finding is promoted to `error` and the exit
  code is 1

#### Scenario: SUPPRESS_UNUSED is itself suppressible
- **WHEN** the profile declares `SUPPRESS: {code: SUPPRESS_UNUSED}` and two
  other suppress entries match no findings
- **THEN** validation emits two `SUPPRESS_UNUSED` findings (one per stale
  entry), both suppressed, and the SUPPRESS_UNUSED suppress entry is itself
  used (no further SUPPRESS_UNUSED emitted for it)
