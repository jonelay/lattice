# Writing a profile

A profile is the YAML file that tells lattice what your register contains: which kinds of
node exist, what their IDs look like, which attributes they carry, how they may be
connected, and which of those rules are worth reporting on. The core ships no vocabulary
of its own — it does not know what a requirement, an issue or a test is. Everything
domain-specific arrives from a profile.

This is the reference for writing one. [glossary.md](glossary.md) defines every term
lattice uses; `openspec/specs/profile-schema/spec.md` is the normative contract; where
this guide and that spec disagree, the spec is right.

## What belongs in a profile, and what does not

A profile declares **vocabulary and policy**. An adapter reads **syntax**.

The split matters because it decides where your work goes. If your register is a markdown
table and you want a new column recognised, that is an adapter change — code. If your
register already parses and you want a new ID shape accepted, a new node kind, a different
severity, or a coverage validation, that is a profile change — data, no code at all.

Concretely:

| Question | Answered by |
|---|---|
| What does a heading or table row look like on disk? | adapter |
| Which file does the register live in? | profile (`adapter:` block, read by the adapter) |
| What may an ID look like? | profile (`id_pattern`) |
| What kinds of thing exist, and what do they carry? | profile (`node_kinds`) |
| What may point at what? | profile (`edge_kinds`) |
| Which gaps are errors, warnings, or advice? | profile (`validations`) |

There is deliberately no parser DSL. A profile is data, not a programming language written
in YAML, so a register whose on-disk shape differs from every shipped adapter needs an
adapter — a standalone program taking `--profile` and `--target` and writing an interface
document to stdout. `openspec/specs/adapter-contract/spec.md` defines that interface, and
`crates/adapter-toml/` is a worked example over a non-markdown register.

## A minimal profile

Four keys are required: `name`, `profile_version`, `node_kinds`, `edge_kinds`. This is a
complete, loadable profile for a register of architecture decisions and the documents that
implement them:

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

Check that it loads before wiring an adapter to it:

```sh
lattice resolve --profile profiles/decisions.yaml
```

`resolve` prints the resolved profile document — the same handoff the core writes for an
adapter — so a load error surfaces here rather than halfway through a validation run.

## Top-level keys

| Key | Required | Holds |
|---|---|---|
| `name` | yes | the profile's name |
| `profile_version` | yes | semver string; see [Versioning](#versioning) |
| `node_kinds` | yes | map of kind name → declaration |
| `edge_kinds` | yes | map of kind name → declaration |
| `validations` | no | list of validator configurations |
| `pathways` | no | list of ordering-pathway names |
| `extends` | no | path to a parent profile |

**A top-level key the core does not recognise is preserved, not rejected.** This is how a
profile carries adapter-specific settings the core has no opinion about. Every shipped
profile uses it for an `adapter:` block:

```yaml
adapter:
  paths:
    requirements: "docs/internal/REQUIREMENTS.md"
    spec_dir: "docs/internal/spec"
```

The core never interprets those paths. It passes them through in the resolved document,
and the adapter reads them. Put anything your adapter needs here; the shape is yours.

### Markdown table adapter configuration

The markdown adapter's original single-table form applies one schema to every markdown
table selected by `adapter.paths.files`. The profile must declare exactly one node kind:

```yaml
adapter:
  paths:
    files: ["*.md"]
  table:
    id_column: "ID"
    column_map:
      "Description": summary
    edge_columns:
      "Traces To": traces_to
```

Use `adapter.tables` when one markdown file contains tables with different schemas. Each
entry selects tables by matching a regular expression against the text of the most recent
markdown heading, then supplies that table's node kind and column mappings:

```yaml
adapter:
  paths:
    files: ["**/*.md"]
  tables:
    - heading: '^Domain [0-9]+$'
      kind: requirement
      id_column: "ID"
      column_map:
        "Description": summary
        "Status": status
      edge_columns:
        "Traces To": traces_to

    - heading: '^Stakeholders$'
      kind: stakeholder
      id_column: "ID"
      column_map:
        "Name": name
        "Role": role
      edge_columns: {}
```

Patterns are tried in list order and the first match wins. They see heading text such as
`Domain 01`, without the leading `##`. The nearest preceding heading at any level is the
entire context: a `### Notes` heading replaces an earlier `## Requirements` heading rather
than inheriting from it. A table before any heading has an empty context. In multi-table
mode, an unmatched table produces a `PARSE_ERROR` and no nodes or edges; in the singular
form, an implicit empty pattern matches every table for backward compatibility. Every
`kind` named by `adapter.tables` must also be declared in `node_kinds`.

### TOML adapter configuration

The TOML adapter selects files with glob patterns in `adapter.paths.files` and maps each
configured top-level array of tables to a node kind. `adapter.tables` is a map keyed by
the TOML table name; every entry declares the node `kind`, the source key holding its ID
(`id_key`), attribute mappings (`key_map`), and edge mappings (`edge_keys`):

```yaml
adapter:
  paths:
    files: ["registers/*.toml"]
  id_prefix: file_stem
  tables:
    item:
      kind: item
      id_key: id
      key_map:
        id: id
        stage: stage
      edge_keys:
        register: belongs_to
        refs: references
```

With `adapter.id_prefix: file_stem`, row IDs are qualified as `<file-stem>/<id>`; omit it
to keep the value of `id_key` unchanged. The only supported prefix mode is `file_stem`.

For a one-file-per-record register, set `adapter.mode: directory`. Each matched `.toml`
file's root table is then read as one record. `adapter.tables` is still required, but
only its first entry by key order is used; that entry's map key is ignored and its
`kind`, `id_key`, `key_map`, and `edge_keys` describe every file-root record:

```yaml
adapter:
  mode: directory
  paths:
    files: ["records/*.toml"]
  tables:
    record:
      kind: device
      id_key: id
      key_map:
        name: name
      edge_keys:
        refs: references
```

`mode` defaults to `tables`, preserving the array-of-tables behavior above. Directory
mode supports `id_prefix` and `pathway`, but not `header`: the whole file is already the
record. A readable target with no files matching the configured globs produces an
info-severity `NO_MATCHING_FILES` finding.

An optional `adapter.header` reads one singleton table per selected file. It names the
TOML `table`, output node `kind`, ID source (`id`), and attribute `key_map`. Set `id` to
`file_stem` to derive the node ID from the filename, or name a key in the header table:

```yaml
adapter:
  header:
    table: register
    kind: register
    id: file_stem
    key_map:
      register_version: register_version
      status: status
```

An optional `adapter.pathway` reads an ordering pathway from one file beneath the target.
`source_file` identifies that file, `name` names the profile pathway, and `order_key` and
`current_key` are dotted paths to the ordered values and current value:

```yaml
adapter:
  pathway:
    source_file: registers/index.toml
    name: stage
    order_key: register.stage_order
    current_key: register.current_stage
```

See `profiles/toml.yaml` for a worked example combining table dispatch, a header node,
file-stem ID qualification, and a pathway reader.

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

**`id_pattern`** — a regex the node's ID must match. A node whose ID does not match its
kind's pattern gets an `ID_FORMAT` finding. Note the doubled backslashes: this is a YAML
string, so `\d` must be written `\\d`. An invalid regex is a load error naming the kind.

The pattern is the whole ID policy, and it is where most schema customisation happens.
Widening `^\\d+\\.\\d+$` to `^\\d+\\.\\d+[a-z]?$` to admit a letter-suffixed insertion is a
one-character edit with no code change anywhere.

**`summary_attr`** — names which attr the trace output uses as its human-readable summary
column. It must be a declared attr and must not be a `list`. A kind without one renders an
empty cell.

**`text_attrs`** — names which attrs a text-ranking consumer reads, in the order given.
Each must be declared and must be `string` or `enum` (an enum value is a string at
runtime); `int`, `bool` and `list` are rejected, because a consumer reading only strings
would skip them in silence and leave you with a key that does nothing.

Absent and empty are different, deliberately. No `text_attrs` means "fall back to
`summary_attr`". `text_attrs: []` means "this kind offers nothing to rank."

**`text_chunk_line_prefix`** — a **literal** line prefix at which a ranking consumer
subdivides that kind's text. It is compared with a string prefix test and is never a
regex: the core validates it and a separate program applies it, and a pattern language
whose engines differ between the two is not a guarantee. Rejected if it is not a string,
is empty or whitespace-only, contains a newline, or sits on a kind that offers no rankable
text.

**`orphan_ok`** — `true` exempts the kind from `ORPHAN_NODE`. Use it where having no edges
is the normal case rather than a gap: unmarked tests, or a flat issue tracker whose issues
mostly link to nothing. It is validation policy only — an exempt node still appears in
`query orphans`, still counts, and still raises every other finding it earns.

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
- `date` is an ISO 8601 calendar date string in exact `YYYY-MM-DD` form. Calendar-invalid
  dates, datetimes, and non-zero-padded dates fail with `ATTR_TYPE`.
- `list` must declare `items`, naming a **scalar** type only — `string`, `int`, `bool` or `date`.
  Lists of lists and lists of enums are rejected, which keeps list validation one flat pass.
- A kind with no `attrs` key loads with an empty map. A kind that carries no typed
  attributes needs no placeholder.

Violations surface as `ATTR_REQUIRED`, `ATTR_TYPE`, `ATTR_ENUM` and `ATTR_LIST_ITEMS`.

Declaring an enum here rather than in the adapter is the point of the split: a state your
tracker adds later becomes an `ATTR_ENUM` finding you can see, never an adapter that
silently rejects or rewrites it.

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

An edge outside the allowed pairs gets an `EDGE_CONSTRAINT` finding carrying the source
node's ID. A pair naming a kind not in `node_kinds` is a load error.

**`cross_source`** — `true` marks an edge kind whose targets may live in another source.
When a target does not resolve in a standalone run, its `VACANCY` is retained at
hint severity instead of the configured or default severity. The program composition
layer resolves the same plain target ID against its source-qualified allowed pairs.
Absent means `false`; source-local edge kinds keep the existing behavior.

```yaml
edge_kinds:
  external_verifies:
    cross_source: true
```

**An edge kind with no `allowed` key permits nothing.** It loads, but every edge of that
kind whose endpoints both exist with declared kinds produces `EDGE_CONSTRAINT`. If you
mean "any pairing", enumerate the pairings.

Edges may name endpoints that were never declared as nodes. That is not silently tolerated
and not treated as a node either — it surfaces as `VACANCY`.

## Validations

`validations` is a **list**, read as a sequence. Each entry maps a validator code to its
configuration.

```yaml
validations:
  - COVERAGE:
      target_kind: req
      edge_kind: verifies
      severity: warning

  - ORPHAN_NODE:
      severity: info
```

Every entry is honoured independently. A profile may configure the same code more than
once — two `COVERAGE` rules over different edge kinds both run — and a later entry never
replaces an earlier one. Silently discarding a declared configuration is the failure this
tool exists to prevent.

**Unknown keys are rejected, not ignored.** `target_kinds` where you meant `target_kind` is
a load error rather than a setting that quietly does nothing.

### The built-in codes

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
| `ORPHAN_NODE` | node has no incoming or outgoing edges | warning |
| `COVERAGE` | target node has no incoming edge of the configured kind | warning |
| `COVERAGE_DEEP` | target uncovered under the rollup rule | warning |
| `SOURCE_MISSING` | a cited path does not resolve on disk (adapter-emitted) | warning |
| `PATHWAY_UNRESOLVED` | a pathway binding names a pathway the graph does not carry | warning |
| `PATHWAY_INVALID` | a register's declared pathway does not hold (adapter-emitted) | warning |
| `COVERAGE_UNKNOWN` | evidence-bearing nodes exist that attribute to nothing | hint |
| `SUGGESTED_EDGE` | the suggestion sidecar proposes an edge | hint |
| `SUGGESTION_UNRESOLVED` | a suggestion names something that does not resolve | hint |

The three hint-tier codes are overlay output — advice rendered alongside findings. They
never affect the exit code and cannot be promoted; see [Severities](#severities).

Configuration keys by code:

- `COVERAGE` — `target_kind`, `edge_kind`, `where`, `severity`
- `COVERAGE_DEEP` — `target_kind`, `via`, `evidence`, `where`, `severity`
- `CONSTRAINT` — `kind`, `when`, `expect`, `reject`, `message`, `severity`
- `SUMMARY` — `node_kind`, `status_attr`, `group_by_attr`, `severity`
- any other code — `severity` alone

That last line is what lets a profile set the severity of a validator the core does not
implement, including codes your own adapter emits.

### Deep coverage

`COVERAGE` is flat: does this node have an incoming edge of that kind? `COVERAGE_DEEP` is
a rollup — a node is covered if it has direct evidence, **or** it has children and every
one of them is covered.

Both coverage validators accept an optional `where` block using the same condition syntax
as `CONSTRAINT`. It limits which target-kind nodes are checked. Omitting it checks every
target, as before.

```yaml
  - COVERAGE:
      target_kind: req
      edge_kind: verifies
      where:
        status: {not: "deferred"}
```

```yaml
  - COVERAGE_DEEP:
      target_kind: req
      via: derives        # edge kind whose *targets* are the parents
      evidence: verifies
      where:
        status: {not: "deferred"}
      severity: warning
```

Use it wherever requirements decompose. A flat validation reads a parent covered only through
its children as a false positive; the rollup does not. It is computed per run as a least
fixed point and stored nowhere, so a childless target with no evidence stays uncovered and
an evidence-free cycle stays uncovered. `where` filters only the targets that may produce a
finding; non-matching target-kind nodes remain in the traversal and can still carry coverage
between descendants and ancestors.

### SUMMARY is not a finding

`SUMMARY` is the one entry in `validations` that reports nothing. It configures the
`lattice summary` command's status rollup, and that command **cannot run without it** —
a profile with no `SUMMARY` block makes `lattice summary` exit 2.

```yaml
  - SUMMARY:
      node_kind: spec-goal
      status_attr: status
      group_by_attr: file
```

The rollup has one row per distinct value of `group_by_attr`, with a column for each value
of the `status_attr` enum. Above, that is one row per spec file and one column per status.

A non-string value for `node_kind`, `status_attr` or `group_by_attr` makes `lattice
summary` exit 2 with an error rather than producing a partial table.

### Cross-field constraints

`CONSTRAINT` lets a profile express per-node invariants as data. Each entry declares
which node kind to check, an optional guard (`when`), and one or both of `expect` (all
conditions must hold) and `reject` (no condition may hold).

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

The `when` block is a guard: if any condition fails, the rule is silently skipped for
that node. Then `expect` and `reject` are checked — a failure in either emits a
`CONSTRAINT` finding. A rule must declare at least one of `expect` or `reject`.

The condition operators are:

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
operators compare integers numerically and strings lexically, which gives chronological
ordering for validated `YYYY-MM-DD` dates. Missing attrs, mixed types, floats, and other
JSON values do not compare. An unknown operator is a load error, not a silently inert
condition.

Multiple `CONSTRAINT` entries are honoured independently, following the same repeated-code
contract as `COVERAGE`. Pathway demotion and `--strict` promotion apply normally.

### Severities

Four tiers: `error`, `warning`, `info`, `hint`.

`--strict` promotes `warning` to `error` and reaches exactly that tier — never `info`,
never `hint`. Exit codes are 0 clean, 1 error-severity findings, 2 lattice could not run at
all.

An override may demote any code, including to `hint`. **An override may not promote a
finding that arrives at `hint`.** Hint marks advice the tool cannot stand behind as a
warning, and promoting it would launder advice into a gate; an attempt is reported as
`CONFIG_ERROR` naming the code, once, and the finding stays at `hint`.

The rule is stated over the severity a finding *arrives at*, not over its shipped default,
because the core has no default for a code it does not implement.

## Ordering pathways

A pathway lets a finding's severity depend on how far the target has actually progressed —
so that a reference to something from a stage the register has not reached yet reads as
advice rather than as a gap.

Declare the pathway, then bind a code to it:

```yaml
pathways:
  - stage

validations:
  - VACANCY:
      pathway: stage
      position_attr: stage
```

`pathways` is a list of bare names and nothing else. It must not carry the ordered positions or
the current position: those are target state, read from the register by the adapter at
ingest. A copy held in the profile would advance without lattice noticing, which is the
hand-maintained derived state the tool exists to eliminate.

Rules worth knowing before you write one:

- `pathway` and `position_attr` are required together. A partial binding is a load error,
  because a setting that silently does nothing is indistinguishable from one that works.
- `pathway` must name a pathway in the same profile's `pathways` list.
- One binding per code. A second is a load error naming the duplicated code — with two
  bindings there would be two answers for one finding and no rule to choose. Repeated
  *non-binding* configuration of the same code is still honoured.
- A demoted finding becomes `info`, always. There is no configurable target: demotion to
  `warning` would be promoted straight back by `--strict`.

Bindings work on codes the core does not implement, on the same terms, so an adapter's own
codes can be bound too.

## Inheritance

`extends` names a single parent profile, resolved relative to the file that declares it.
The parent merges into the child before any structural validation, and `extends` is
stripped from the result.

```yaml
name: decisions-strict
profile_version: "1.1.0"
extends: decisions.yaml

validations:
  - ORPHAN_NODE:
      severity: error
```

Merge rules:

- **Scalars** — the child wins.
- **Lists** — the child replaces the parent's list whole. There is no element merging: a
  child restating `allowed` or `validations` owns that list entirely.
- **Mappings** — recursive deep merge. A child kind declaring only a new `id_pattern`
  keeps the parent's `attrs`.

`profile_version` comes from the child alone. A child that omits it is rejected even when
the parent declares one, because the child is the document of record for compatibility.

Chains (A extends B extends C) work; cycles, self-extension, a non-string `extends`, a
missing parent file, and a parent that is not a YAML mapping are all load errors.

Node-kind order in the merged result is parent-first, then child-only additions. A child
kind overriding a parent kind keeps the parent's position. Order matters because it sets
`declared_index`, which controls trace grouping.

## Versioning

`profile_version` is a semver string, required in every profile. The core rejects a profile
whose major version exceeds what it supports.

Bump it when you change what the profile means — and note that **node IDs are public**.
They are the join key for anything downstream, so changing an `id_pattern` in a way that
changes existing IDs breaks every reference to them. Widening a pattern to admit a new
shape is additive and safe; renumbering is not.

## What is not configurable, and why

- **The parser.** No profile-driven grammar. Registers whose on-disk shape differs need an
  adapter.
- **Pathway values.** The ordered positions and the current position are read from the
  register, never declared in the profile.
- **Promotion out of `hint`.** See [Severities](#severities).
- **Caching.** Nothing computed is written back. Coverage, rollups and orphans are derived
  on demand, every run. A profile key that stored a computed answer would be the drift this
  tool exists to catch.

## Checking your work

```sh
lattice resolve  --profile profiles/yours.yaml
lattice validate --profile profiles/yours.yaml --adapter ./adapters/yours \
    --target /path/to/register-repo --format plain
```

`resolve` catches load errors on their own. `validate` exercises the profile against real
data.

**A finding count of zero proves nothing on its own.** Zero dangling references over zero
edges is satisfied by an adapter that built no edges at all. Check the edge count beside
the finding count — `lattice query counts` reports per-kind node and edge tallies, showing
declared kinds even at zero.
