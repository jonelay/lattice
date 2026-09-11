# Profile reference

A profile declares the node kinds, ID patterns, attributes, edge kinds, and validation
rules for a register. Lattice doesn't know what a "requirement" or a "test" is until you
tell it in a profile.

[glossary.md](glossary.md) defines terms; `openspec/specs/profile-schema/spec.md`
describes the full behavior.

## Profiles vs adapters

The profile says what things mean. The adapter says how to read the file.

| Question | Answered by |
|---|---|
| What does a heading or table row look like on disk? | adapter |
| Which file does the register live in? | profile (`adapter:` block, read by the adapter) |
| What may an ID look like? | profile (`id_pattern`) |
| What kinds of thing exist, and what do they carry? | profile (`node_kinds`) |
| What may point at what? | profile (`edge_kinds`) |
| Which gaps are errors, warnings, or advice? | profile (`validations`) |

If none of the shipped adapters can read your register's format, you can write your own.
It takes `--profile` and `--target` and writes an interface document to stdout. See
`openspec/specs/adapter-contract/spec.md` and `crates/adapter-toml/`.

## Minimal profile

Four keys are required: `name`, `profile_version`, `node_kinds`, `edge_kinds`:

```yaml
name: decisions
profile_version: "1.0.0"

node_kinds:
  decision:
    id_pattern: "^ADR-\\d{4}$"
    summary_attr: title
    attrs:
      title:  { type: string, required: true }
      status: { type: enum, values: [proposed, accepted, superseded], required: true }

  doc:
    id_pattern: "^docs/.+\\.md$"
    summary_attr: title
    attrs:
      title: { type: string, required: true }

edge_kinds:
  implements:
    allowed:
      - [doc, decision]
  supersedes:
    allowed:
      - [decision, decision]
```

Check that it loads with `lattice resolve --profile profiles/decisions.yaml` before
wiring an adapter to it.

## Top-level keys

| Key | Required | Holds |
|---|---|---|
| `name` | yes | the profile's name |
| `profile_version` | yes | three-part numeric version (X.Y.Z); see [Versioning](#versioning) |
| `node_kinds` | yes | map of kind name → declaration |
| `edge_kinds` | yes | map of kind name → declaration |
| `validations` | no | list of validator configurations |
| `pathways` | no | list of ordering-pathway names |
| `extends` | no | path to a parent profile |

**A top-level key the core does not recognise is preserved, not rejected.** Keys the
core doesn't use get passed along to the adapter as-is. Every shipped profile uses this
for an `adapter:` block whose shape is adapter-specific.

Per-adapter `adapter:` block configuration is in [adapters.md](adapters.md).

## Node kinds

Each entry in `node_kinds` declares one kind. Only `id_pattern` is required.

```yaml
node_kinds:
  req:
    id_pattern: "^REQ-\\d{4}$"
    summary_attr: text
    text_attrs: [text, rationale]
    text_chunk_line_prefix: "#### Scenario:"
    orphan_ok: false
    attrs:
      text:      { type: string, required: true }
      rationale: { type: string }
```

**`id_pattern`.** Regex the node's ID must match (`ID_FORMAT` on mismatch). Double
backslashes in YAML: `\d` → `\\d`. An invalid regex is a load error.

**`summary_attr`.** Attr for the trace output's summary column. Must be declared, not
a `list`.

**`text_attrs`.** Attrs a text-ranking consumer reads, in order. Must be `string` or
`enum`. Absent means fall back to `summary_attr`; `[]` means no text to rank.

**`text_chunk_line_prefix`.** Literal line prefix (not regex) at which a ranking consumer
subdivides text. Rejected if blank, contains a newline, or the kind has no rankable text.

**`orphan_ok`.** `true` exempts the kind from `UNREFERENCED`/`UNTRACED`.
This only affects findings; exempt nodes still show up in `query orphans`.

## Attributes

Six types: `string`, `int`, `bool`, `date`, `enum`, `list`.

```yaml
attrs:
  text:     { type: string, required: true }
  count:    { type: int }
  active:   { type: bool }
  expires:  { type: date }
  status:   { type: enum, values: [done, partial, todo], required: true }
  tags:     { type: list, items: string }
  dates:    { type: list, items: date }
```

- `type` is required; `required` defaults to `false`.
- `enum` must declare `values`.
- `date` is `YYYY-MM-DD` exactly. Calendar-invalid dates and datetimes fail `ATTR_TYPE`.
- `list` must declare `items` as a scalar type (`string`, `int`, `bool`, `date`).
- A kind with no `attrs` key loads with an empty map.

Violations: `ATTR_REQUIRED`, `ATTR_TYPE`, `ATTR_ENUM`, `ATTR_LIST_ITEMS`.

## Edge kinds

An edge kind declares which `[source_kind, target_kind]` pairs are legal.

```yaml
edge_kinds:
  verifies:
    allowed:
      - [test, req]
  derives:
    allowed:
      - [need, need]
      - [req, need]
      - [req, req]
```

An edge outside the allowed pairs gets `EDGE_CONSTRAINT`. A pair naming a kind not in
`node_kinds` is a load error. An edge kind with no `allowed` key permits nothing.

**`cross_source: true`.** Marks an edge kind whose targets may live in another source.
Unresolved targets get `VACANCY` at hint severity instead of the default, for later
resolution during fuse composition.

Edges naming undeclared endpoints surface as `VACANCY`.

## Validations

`validations` is a **list**, read as a sequence. Each entry maps a validator code to its
configuration.

```yaml
validations:
  - COVERAGE:
      target_kind: req
      edge_kind: verifies
      severity: warning

  - UNREFERENCED:
      severity: info
  - UNTRACED:
      severity: info
```

Every entry is honoured independently. The same code may appear more than once (e.g. two
`COVERAGE` rules over different edge kinds). Unknown keys are rejected, not ignored.

### Finding codes

| Code | Fires when | Default |
|---|---|---|
| `ID_FORMAT` | node ID does not match its kind's `id_pattern` | error |
| `UNKNOWN_KIND` | node or edge kind not declared in the profile | error |
| `EDGE_CONSTRAINT` | edge kind used between disallowed kinds | error |
| `VACANCY` | edge references an ID not in the graph | error |
| `ATTR_REQUIRED` | a required attr is missing | error |
| `ATTR_TYPE` | attr value does not match its declared type | error |
| `ATTR_ENUM` | enum value not in `values` | error |
| `ATTR_LIST_ITEMS` | list element does not match `items` | error |
| `CONFIG_ERROR` | validation config is malformed or names an undeclared kind | error |
| `CONSTRAINT` | a node fails a cross-field constraint declared in the profile | warning |
| `UNREFERENCED` | node has no incoming edges | warning |
| `UNTRACED` | node has no outgoing edges | warning |
| `COVERAGE` | target node has no incoming edge of the configured kind | warning |
| `COVERAGE_DEEP` | target uncovered under the rollup rule | warning |
| `SOURCE_MISSING` | a cited path does not resolve on disk (adapter-emitted) | warning |
| `PATHWAY_UNRESOLVED` | a pathway binding names a pathway the graph does not carry | warning |
| `PATHWAY_INVALID` | a register's declared pathway does not hold (adapter-emitted) | warning |
| `SUPPRESS_UNUSED` | a `SUPPRESS` entry matched no finding this run | info |
| `COVERAGE_UNKNOWN` | evidence-bearing nodes exist that attribute to nothing | hint |
| `SUGGESTED_EDGE` | the suggestion sidecar proposes an edge | hint |
| `SUGGESTION_UNRESOLVED` | a suggestion names something that does not resolve | hint |

Hint-tier codes cannot be promoted; see [Severities](#severities).

Configuration keys per code:

- `COVERAGE` - `target_kind`, `edge_kind`, `where`, `severity`
- `COVERAGE_DEEP` - `target_kind`, `via`, `evidence`, `where`, `severity`
- `CONSTRAINT` - `kind`, `when`, `expect`, `reject`, `message`, `severity`
- `SUMMARY` - `node_kind`, `status_attr`, `group_by_attr`, `severity`
- `SUPPRESS` - `code`, `node_ids` (no `severity`; see [Suppressing findings](#suppressing-findings))
- any other code (including adapter-emitted) - `severity` alone

### Deep coverage

`COVERAGE` is flat: does this node have an incoming edge of that kind? `COVERAGE_DEEP`
is a rollup. A node counts as covered when it has a direct incoming edge of the
`evidence` kind, **or** all nodes reachable from it along the `via` edge kind are
themselves covered. The rollup propagates until no more nodes change. Leaf nodes
with no evidence and evidence-free cycles stay uncovered.

Both accept an optional `where` block (same syntax as `CONSTRAINT`) to limit which
target-kind nodes are checked:

```yaml
  - COVERAGE_DEEP:
      target_kind: req
      via: derives
      evidence: verifies
      where:
        status: {not: "deferred"}
```

### SUMMARY

`SUMMARY` configures `lattice summary`'s status rollup - a table grouping nodes by
one attribute and counting values of another. Without it, summary falls back to a
structural report (node/edge/finding counts by kind).

```yaml
  - SUMMARY:
      node_kind: spec-goal
      status_attr: status
      group_by_attr: file
```

One row per distinct `group_by_attr` value, one column per `status_attr` enum value.

### CONSTRAINT

Per-node invariants. Each entry declares the node kind, an optional guard (`when`), and
one or both of `expect` (all must hold) and `reject` (none may hold).

```yaml
  - CONSTRAINT:
      kind: requirement
      when:
        status: { eq: "approved" }
      expect:
        rationale: { present: true }
      reject:
        priority: { eq: "" }
      message: "Approved requirements must have rationale and non-empty priority"
      severity: warning
```

`when` is a guard: if any condition fails, the rule is skipped. A rule must declare
at least one of `expect` or `reject`.

Condition operators:

| Operator | Meaning |
|---|---|
| `eq: <value>` | attr equals value (type-aware) |
| `not: <value>` | attr does not equal value |
| `in: [v1, v2]` | attr is one of the listed values |
| `lt: <value>` | attr is less than value |
| `gt: <value>` | attr is greater than value |
| `lte: <value>` | attr is less than or equal to value |
| `gte: <value>` | attr is greater than or equal to value |
| `matches: <regex>` | attr string matches the pattern |
| `present: true` | attr exists (even if empty) |
| `present: false` | attr does not exist |

Comparison is type-aware: a string `"42"` does not match an integer `42`. Ordering
operators (`lt`, `gt`, `lte`, `gte`) require an integer or a string as the threshold
value; arrays, objects, booleans, nulls, and floats are rejected at load time.
Integers compare numerically and strings compare lexically, which gives chronological
ordering for validated `YYYY-MM-DD` dates. At evaluation time, a missing attr or a
type mismatch between the node's attr and the threshold does not compare (the
condition evaluates as unsatisfied). An unknown operator is a load error.

Multiple `CONSTRAINT` entries are honoured independently, following the same repeated-code
contract as `COVERAGE`. Pathway demotion and `--strict` promotion apply normally.

### Severities

Four tiers: `error`, `warning`, `info`, `hint`. `--strict` promotes `warning` → `error`
only. Exit codes: 0 clean, 1 error-severity findings, 2 lattice could not run.

An override may demote any code, including to `hint`. **Hint cannot be promoted.** An
attempt is reported as `CONFIG_ERROR` and the finding stays at `hint`.

### SUPPRESS

Declares a finding as structurally expected without hiding it from JSON.

```yaml
  - SUPPRESS:
      code: VACANCY
      node_ids: [UN-1, UN-2]   # optional; omit to suppress all findings of the code
  - SUPPRESS:
      code: UNTRACED
```

A suppressed finding never counts toward exit code 1, is omitted from `plain`/`rich`,
and stays in `json` with `"suppressed": true`. Entries for the same code merge their
`node_ids`; one without `node_ids` supersedes ID-specific entries.

Suppression runs **after** `--strict` promotion. `CONFIG_ERROR` cannot be suppressed.

**Stale entries** produce `SUPPRESS_UNUSED` (info). Override to `warning` and `--strict`
gates on stale config. `SUPPRESS_UNUSED` is itself suppressible.

## Pathways

Findings bound to a pathway are demoted to `info` when the target has not reached the
bound stage.

```yaml
pathways:
  - stage

validations:
  - VACANCY:
      pathway: stage
      position_attr: stage
```

`pathways` is a list of bare names. The ordered positions and current position are read
from the register by the adapter, never declared in the profile.

- `pathway` and `position_attr` are required together; a partial binding is a load error.
- `pathway` must name a pathway in the profile's `pathways` list.
- One binding per code (a second is a load error).
- Demoted findings become `info`, always.
- Bindings work on adapter-emitted codes too.

## Inheritance

`extends` names a parent profile (relative path). Merged before validation, stripped
from the result.

```yaml
name: decisions-strict
profile_version: "1.1.0"
extends: decisions.yaml

validations:
  - UNREFERENCED:
      severity: error
  - UNTRACED:
      severity: error
```

Merge: scalars - child wins. Lists - child replaces whole. Mappings - recursive deep
merge. `profile_version` comes from the child alone. Chains work; cycles are load errors.
Node-kind order is parent-first, child additions after.

## Versioning

`profile_version` is X.Y.Z (no leading zeros), required. The core rejects a major version
it does not support. **Node IDs are public.** Widening an `id_pattern` is safe;
renumbering breaks downstream references.

## Verification

```sh
lattice resolve  --profile profiles/yours.yaml
lattice validate --profile profiles/yours.yaml --adapter ./adapters/yours \
    --target /path/to/register-repo --format plain
```

`resolve` catches load errors. `validate` exercises the profile against real data.
Zero findings over zero edges just means nothing was checked. Verify the edge count
with `lattice query counts`.
