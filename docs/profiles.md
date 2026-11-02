# Profiles

A profile declares what a register's records mean: node kinds, ID patterns, attributes,
edge kinds and validation rules. An adapter reads the files; the profile's `adapter:`
block tells it where. If no shipped adapter reads your format, write your own
([Writing your own adapter](adapters.md#writing-your-own-adapter)).
[glossary.md](glossary.md) defines terms.

On this page: [A first profile](#a-first-profile) · [Minimal profile](#minimal-profile) ·
[Top-level keys](#top-level-keys) ·
[Node kinds](#node-kinds) · [Attributes](#attributes) · [Edge kinds](#edge-kinds) ·
[Validations](#validations) · [Finding notes](#finding-notes) · [Pathways](#pathways) ·
[Inheritance](#inheritance) · [Versioning](#versioning) · [Verification](#verification)

## A first profile

Start from the shipped profile for your format in `profiles/`, copied and edited. Say
your requirements live in a Markdown table, `requirements.md`:

```markdown
| ID | Description | Status | Traces To |
|---|---|---|---|
| REQ-1 | Users can sign in | approved | |
| REQ-2 | Sessions expire after an hour | draft | REQ-1 |
| REQ-3 | Failed sign-ins are logged | draft | REQ-4 |
```

`profiles/md.yaml` reads exactly this shape:

```yaml
name: md
profile_version: "1.0.0"

adapter:
  native: md
  paths:
    files: ["*.md"]
  table:
    id_column: "ID"
    column_map:
      "Description": "summary"
      "Status": "status"
    edge_columns:
      "Traces To": "traces_to"

node_kinds:
  requirement:
    id_pattern: "^REQ-[0-9]+$"
    summary_attr: summary
    attrs:
      summary: { type: string, required: true }
      status: { type: string }

edge_kinds:
  traces_to:
    allowed:
      - [requirement, requirement]
```

The `adapter:` block maps the table: `ID` names each row, `Description` and `Status`
become attrs, and `Traces To` holds edges. `native: md` selects the built-in adapter, so
this profile needs no `--adapter`. For your own table, copy the profile and change the
column names, the kind and the ID pattern; `latt show --profile yours.yaml` checks that it
loads. Then validate:

```sh
latt validate --profile yours.yaml --target /path/to/register --format plain
```

```text
WARNING UNTRACED requirements.md:3 node 'REQ-1' has no outgoing edges
WARNING UNREFERENCED requirements.md:4 node 'REQ-2' has no incoming edges
WARNING UNREFERENCED requirements.md:5 node 'REQ-3' has no incoming edges
ERROR VACANCY requirements.md:5 edge 'REQ-3'->'REQ-4' (kind 'traces_to'): target 'REQ-4' does not exist
```

The run exits `1`: `REQ-3` traces to a `REQ-4` the table does not declare. Point it at
`REQ-1` and the run exits `0`, with the warnings left. The rest of this page covers
kinds, IDs, edges and validations.

## Minimal profile

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

Check that it loads with `latt show --profile profiles/decisions.yaml` before you wire
an adapter to it.

## Top-level keys

| Key | Required | Holds |
|---|---|---|
| `name` | yes | the profile's name |
| `profile_version` | yes | X.Y.Z; see [Versioning](#versioning) |
| `node_kinds` | yes | map of kind name → declaration |
| `edge_kinds` | yes | map of kind name → declaration |
| `validations` | no | list of validator configurations |
| `pathways` | no | names of ordered lifecycle sequences such as draft, review, approved ([Pathways](#pathways)) |
| `extends` | no | path to a parent profile |
| `annotations` | no | path to a notes file ([Finding notes](#finding-notes)) |
| `adapter` | no | adapter configuration ([adapters.md](adapters.md)); every shipped profile but `entomologist.yaml` has one |

Any other top-level key is a load error that names it and lists the accepted ones.

The core reads three `adapter:` keys and passes the rest through unchanged:

| Key | Meaning |
|---|---|
| `native` | the [native adapter](adapters.md#native-adapters) to use when `--adapter` is not passed |
| `timeout_seconds` | how long the core waits for an external adapter before killing it and exiting 2 |
| `command_timeout_seconds` | validated by the core, read by adapters |

Timeouts are positive integers in seconds; any other value refuses the profile (exit 2).
Defaults: [Timeouts](adapters.md#timeouts).

## Node kinds

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

| Key | Holds | Rules |
|---|---|---|
| `id_pattern` (required) | regex the ID must match (`ID_FORMAT`) | double backslashes in YAML (`\\d`); an invalid regex is a load error |
| `summary_attr` | attr for trace's summary column | declared, and other than `list` |
| `text_attrs` | attrs a text-ranking consumer reads, in order | `string` or `enum`; absent falls back to `summary_attr`; `[]` means no text |
| `text_chunk_line_prefix` | literal line prefix where a ranking consumer splits text | rejected if blank, multi-line, or the kind has no rankable text |
| `orphan_ok` | `true` exempts the kind from `UNREFERENCED` and `UNTRACED` | nodes still appear in `query orphans` |
| `id_names` | IDs are addresses in the target | [below](#id_names) |
| `attrs` | attributes | [Attributes](#attributes) |
| `actions` | programs `browse` can run on a node | [below](#actions) |

Any other key is a load error. An orphan code applies only in the direction an edge
could supply: `UNREFERENCED` to a kind some `allowed` pair names as a target, `UNTRACED`
to one named as a source. A kind no pair names gets neither.

### id_names

A declared node whose address names nothing gets a `SOURCE_MISSING` warning; an ID named
only in an edge cell stays a `VACANCY`. Addresses are resolved by scanning files, with
nothing run. The md and TOML adapters honour the key; others ignore it.

| Value | Address | Example | Block it stands for |
|---|---|---|---|
| `file` | path relative to the target | `docs/guide.md` | |
| `cargo` | `.rs` path, inline `mod` chain, `fn` | `crates/x/src/lib.rs::tests::splits` | `{ language: rust, separator: "::" }` |
| `pytest` | `.py` path, optional top-level class, name; `[…]` ignored | `tests/test_a.py::TestGroup::test_b[1]` | `{ language: python, separator: "::", params: "[]" }` |
| `gotest` | `_test.go` path, top-level `func` | `pkg/store/store_test.go::TestPut` | `{ language: go, separator: "::" }` |

`separator` is any non-empty string. `params` is an opener and closer; the suffix they
enclose is dropped before splitting.

What counts as a definition:

- **rust**: an `fn` at top level or directly in an inline `mod name { … }`, any depth.
  Out-of-line `mod tests;` is not followed; `fn`s in an `impl`, `trait`, function body
  or macro do not count. For a crate root or integration test the chain matches the name
  `cargo test` prints; elsewhere the file path stands in for the module prefix.
- **python**: a top-level `def`, `async def` or `class`, or a method directly in a
  top-level class (as `adapters/_python_tests.py` counts). Nothing nested in `if`, `try`,
  a function or a nested class.
- **go**: a top-level `func`; methods and `t.Run` subtests do not count.

Rules:

- Absolute paths, `..`, and symlinks leading outside the target do not resolve.
- An unreadable source file is one `PARSE_ERROR` warning; rows naming it go unchecked.
- `null` reads as absent.
- An unknown preset, language or block key is a load error; there is no fallback to a
  file check.
- Macro-generated tests (`rstest`, `proptest!`) report missing; list them in a
  [`SUPPRESS`](#suppress) entry.

### actions

Programs `browse` runs on a node of this kind, keyed by verb, in the target directory.
Use actions only from a profile you trust.

```yaml
node_kinds:
  issue:
    id_pattern: "^[0-9a-f]{8}$"
    attrs:
      number: { type: int }
    actions:
      comment: { run: ["ent", "comment", "--", "{id}"], mode: interactive }
      show:    { run: ["ent", "show", "--", "{id}"], mode: capture }
      web:     { run: ["gh", "issue", "view", "{attrs.number}", "--web"], mode: detached }
```

- `run` is an argument list with no shell, splitting, globbing or expansion. The first
  element, which holds no placeholder, is looked up on `PATH`, or against the target
  directory when it contains `/`.
- Placeholders: `{id}`, `{kind}`, `{provenance.file}`, `{provenance.line}`,
  `{attrs.<name>}` for a declared non-list attr. They may sit inside an argument
  (`--repo={attrs.repo}`); `{{` and `}}` are literal braces. Unknown placeholders,
  undeclared or list attrs, and unmatched braces are load errors.
- An argument starting with a placeholder that resolves to text starting with `-` is
  refused unless an earlier argument is `--`. Put `--` before placeholders.
- `mode`: `interactive` hands over the terminal and reloads the register on exit;
  `capture` runs with stdin closed and shows output in a pane; `detached` does not wait.
- Browse binds the verbs to keys ([browse.md](browse.md#acting-on-a-node)). The first
  run of each action from a letter key in a session shows the command and waits for `y`.

## Attributes

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

- `type` (required) is one of `string`, `int`, `bool`, `date`, `enum`, `list`.
  `required` defaults to `false`. Keys: `type`, `required`, `values`, `items`; others are load errors.
- `enum` declares `values`; `list` declares scalar `items`.
- `date` is exactly `YYYY-MM-DD`; invalid dates and datetimes fail `ATTR_TYPE`.
- A kind with no `attrs` loads with none.

Findings: `ATTR_REQUIRED`, `ATTR_TYPE`, `ATTR_ENUM`, `ATTR_LIST_ITEMS`.

## Edge kinds

An edge kind lists its legal `[source_kind, target_kind]` pairs.

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

- Other pairs get `EDGE_CONSTRAINT`; undeclared endpoints get `VACANCY`.
- A pair naming an undeclared kind is a load error. No `allowed` permits nothing.
- Keys: `allowed`, `cross_source`. Others are load errors.
- `cross_source: true`: targets may live in another source, so unresolved ones get
  `VACANCY` at hint, for fuse to resolve.

## Validations

A list; each entry maps one code to its configuration.

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

- Entries apply independently; a code may repeat. Unknown keys are rejected.
- Retired codes are rejected with their replacement: `ORPHAN_NODE` → `UNREFERENCED`
  and `UNTRACED`, `DANGLING_REF` → `VACANCY`, `AXIS_UNRESOLVED` → `PATHWAY_UNRESOLVED`,
  `AXIS_INVALID` → `PATHWAY_INVALID`.
- A faulty profile is refused once, listing every independent fault. A fault that stops
  reading (unreadable file, invalid YAML, non-mapping top level) is reported alone.
- Severity is per code: the last entry that sets it wins. A `CONSTRAINT`,
  `EDGE_WHERE`, `EDGE_COUNT`, `COVERAGE` or `COVERAGE_DEEP` entry's own `severity`
  governs only its own findings; without one it takes the code's.

### Finding codes

| Code | Fires when | Default |
|---|---|---|
| `ID_FORMAT` | ID does not match `id_pattern` | error |
| `UNKNOWN_KIND` | node or edge kind undeclared | error |
| `EDGE_CONSTRAINT` | edge between disallowed kinds | error |
| `VACANCY` | edge names an ID absent from the graph | error |
| `ATTR_REQUIRED` | required attr missing | error |
| `ATTR_TYPE` | attr value has the wrong type | error |
| `ATTR_ENUM` | value outside `values` | error |
| `ATTR_LIST_ITEMS` | list element does not match `items` | error |
| `CONFIG_ERROR` | a fuse profile's coverage entry names an undeclared kind, or an override promotes a hint (in a standard profile, bad config is a load error, exit 2) | error |
| `CONSTRAINT` | node fails a [`CONSTRAINT`](#constraint) | warning |
| `EDGE_WHERE` | edge fails an [`EDGE_WHERE`](#edge_where) | warning |
| `EDGE_COUNT` | too few or too many edges ([`EDGE_COUNT`](#edge_count)) | warning |
| `UNREFERENCED` | no incoming edges, kind is an `allowed` target | warning |
| `UNTRACED` | no outgoing edges, kind is an `allowed` source | warning |
| `COVERAGE` | target has no incoming edge of the kind | warning |
| `COVERAGE_DEEP` | target uncovered under the rollup | warning |
| `PATHWAY_UNRESOLVED` | binding names a pathway the graph lacks | warning |
| `NOTE_INVALID` | bad or duplicate notes entry ([Finding notes](#finding-notes)) | error |
| `NOTE_FILE_MISSING` | notes file does not exist | warning |
| `NOTE_EXPIRED` | a note's `until` has passed | warning |
| `EMPTY_KIND` | a declared kind has no node or edge; written at `<profile>` | info |
| `SUPPRESS_UNUSED` | a `SUPPRESS` entry matched nothing | info |
| `OVERRIDE_UNUSED` | an override names a code no shipped component emits, and no finding carried it | info |
| `NOTE_UNUSED` | a notes entry matched nothing | info |
| `COVERAGE_UNKNOWN` | evidence-bearing nodes attribute to nothing | hint |
| `SUGGESTED_EDGE` | the suggestion sidecar proposes an edge | hint |
| `SUGGESTION_UNRESOLVED` | a suggestion does not resolve | hint |
| `SUGGESTION_UNRANKED` | a suggestion's producer left candidates unranked | hint |
| `PARSE_ERROR` | part of the register is unreadable: malformed row, unreadable file, repeated ID (adapter-emitted) | error |
| `AMBIGUOUS_REF` | a reference matches several targets; no edge built (adapter-emitted) | warning |
| `UNREAD_ENTRIES` | a mapped array entry has a shape the profile does not read, reported at its line; under a plain mapping its attr stays unset (adapter-emitted) | warning |
| `WRAPPED_REF` | a reference with no file on its line found no heading, and the previous line ends with a path (adapter-emitted) | info |
| `EXTERNAL_REF` | a reference points outside the register; no edge built (adapter-emitted) | info |
| `PATHWAY_INVALID` | a declared pathway does not hold (adapter-emitted) | warning |
| `SOURCE_MISSING` | an [`id_names`](#id_names) address names nothing (adapter-emitted) | warning |
| `NO_MATCHING_FILES` | file globs matched nothing in a readable target (adapter-emitted) | info |
| `FUSE_ERROR` | the manifest or a path it names cannot be read, parsed or resolved; fuse still reports and exits 2 (fuse) | error |
| `SOURCE_FAILURE` | a source's run exited 2, crashed or wrote unreadable output (fuse) | error |
| `CROSS_SOURCE_DUPLICATE_ID` | one raw ID declared in several sources (fuse) | error |
| `AMBIGUOUS_CROSS_REF` | a cross-source reference matches several allowed target kinds (fuse) | error |

Adapter-emitted rows give the severity shipped adapters use; the core has no default
for them, but overrides apply. Fuse codes come from `latt fuse` alone
([fuse.md](fuse.md#pipeline)). Hints cannot be promoted ([Severities](#severities)). An
override for a code outside this table acts only on another adapter's issues.

Configuration keys per code:

- `COVERAGE` - `target_kind`, `edge_kind`, `where`, `source_where`, `message`, `severity`
- `COVERAGE_DEEP` - `target_kind`, `via`, `evidence`, `where`, `source_where`, `message`, `severity`
- `CONSTRAINT` - `kind`, `when`, `expect`, `reject`, `message`, `severity`
- `EDGE_WHERE` - `edge_kind`, `when`, `expect`, `reject`, `message`, `severity`
- `EDGE_COUNT` - `edge_kind`, `node_kind`, `direction`, `group_by`, `min`, `max`, `where`, `message`, `severity`
- `SUMMARY` - `node_kind`, `status_attr`, `group_by_attr`, `message`, `severity`
- `SUPPRESS` - `code`, `node_ids` (no `severity`; see [SUPPRESS](#suppress))
- any other code (including adapter-emitted) - `message` and `severity`

All but `SUPPRESS` also take the paired `pathway` and `position_attr`
([Pathways](#pathways)). In the first five, a kind key that is missing, non-string or
undeclared is a load error naming code, key and value (exit 2, before the adapter runs).

### Messages

`message` replaces the text of each finding the entry produces; on a severity-only
entry, of every finding with that code, adapter issues included.

| Placeholder | Renders | Available on |
|---|---|---|
| `{message}` | the finding's original message | every entry |
| `{code}`, `{severity}` | code and reported severity | every entry |
| `{id}`, `{kind}` | the finding's node | codes whose findings carry a node (below) |
| `{attrs.<name>}` | a declared non-list attr; `<absent>` if missing | `CONSTRAINT` (`kind`), `EDGE_COUNT` (`node_kind`), `COVERAGE` and `COVERAGE_DEEP` (`target_kind`) |

```yaml
  - VACANCY:
      message: "see ADR-12: {message}"
  - COVERAGE:
      target_kind: req
      edge_kind: verifies
      message: "{id} ({attrs.status}) has no test"
```

`{{` and `}}` are literal braces. Load errors: unknown placeholders, unmatched braces,
node placeholders where findings carry no node, undeclared or list attrs, and
`{provenance.*}` (outputs already show the location).

The codes whose findings always carry a node are
`AMBIGUOUS_CROSS_REF`, `ATTR_ENUM`, `ATTR_LIST_ITEMS`, `ATTR_REQUIRED`, `ATTR_TYPE`,
`CONSTRAINT`, `COVERAGE`, `COVERAGE_DEEP`, `CROSS_SOURCE_DUPLICATE_ID`,
`EDGE_CONSTRAINT`, `EDGE_COUNT`, `EDGE_WHERE`, `ID_FORMAT`, `SUGGESTED_EDGE`,
`UNKNOWN_KIND`, `UNREFERENCED`, `UNTRACED` and `VACANCY`. For entries with their own
check, `{severity}` is the entry's; `--strict` promotes after rendering.

### Deep coverage

`COVERAGE` asks for a direct incoming edge. Under `COVERAGE_DEEP` a node is covered by a
direct `evidence` edge, **or** by incoming `via` edges whose sources are all covered,
repeated until nothing changes. Leaves without evidence and evidence-free cycles stay
uncovered. `where` (`CONSTRAINT` syntax) limits the targets checked:

```yaml
  - COVERAGE_DEEP:
      target_kind: req
      via: derives
      evidence: verifies
      where:
        status: {ne: "deferred"}
```

`source_where` limits the sources whose edges count. Excluded sources (such as retired
tests) cover nothing and leave the `COVERAGE_UNKNOWN` population, whose hint then names `source_where`.

```yaml
  - COVERAGE:
      target_kind: req
      edge_kind: verifies
      source_where:
        status: {ne: retired}
```

### SUMMARY

Configures `latt summary`'s table: one row per `group_by_attr` value, one column per
`status_attr` enum value, plus one for undeclared values found. Without it, summary
prints node, edge and finding counts by kind.

```yaml
  - SUMMARY:
      node_kind: spec-goal
      status_attr: status
      group_by_attr: file
```

### CONSTRAINT

Per-node rules: on nodes of `kind` passing the guard `when`, every `expect` condition
must hold and no `reject` condition may.

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

`expect` and `reject` together need at least one condition. `message` is a string
([Messages](#messages)).

| Operator | Holds when the attr | Trustfall |
|---|---|---|
| `eq: <value>` | equals value | `=` |
| `ne: <value>` | is absent or differs | `!=`, except `ne` also holds for absent |
| `in: [v1, v2]` | is a listed value | `one_of` |
| `lt`, `gt`, `lte`, `gte` | is less, greater, ≤, ≥ | `<`, `>`, `<=`, `>=` |
| `matches: <regex>` | matches unanchored as text; numbers and booleans convert, lists, maps and null fail | `regex` |
| `present: true` / `false` | exists (even empty or null) / does not | `is_null` cannot tell absent from null |

- Comparison is type-aware: `"42"` does not match `42`. A missing attr or type
  mismatch fails the condition.
- Ordering thresholds must be integers or strings; others are load errors. Strings compare lexically, so
  `YYYY-MM-DD` dates order chronologically.
- `matches` is a Rust [`regex`](https://docs.rs/regex); anchor with `^` and `$`.
- Unknown operators, and the retired `not` (use `ne`), are load errors.
- Future operators follow their Trustfall counterpart.

### EDGE_WHERE

Per-edge rules: `CONSTRAINT`'s blocks and operators, for an `edge_kind`, with conditions
keyed by `source`, `target` or `edge` attrs.

```yaml
  - EDGE_WHERE:
      edge_kind: picks
      when:
        source: { status: { eq: decided } }
      reject:
        target: { status: { eq: rejected } }
      message: "A pick must not name a rejected option"
      severity: error
```

- Other keys under a block are load errors. `expect` or `reject` is required.
- One finding per failing edge per entry, at the source node with the edge's provenance.
- Dangling edges and edges of or touching undeclared kinds are skipped. Edges outside
  the allowed pairs are still checked.

### EDGE_COUNT

Counts `edge_kind` edges at each `node_kind` node: outgoing (`direction: out`, default)
or incoming (`in`). The node at the other end is the far end.

```yaml
  - EDGE_COUNT:
      edge_kind: picks
      node_kind: req
      group_by: axis        # optional: one count per distinct far-end value
      min: 1
      max: 1
      where:                # optional: limits the counted nodes
        status: { eq: done }
```

- `min` and/or `max`: non-negative integers, `min` ≤ `max`. No edges counts zero.
- `node_kind` must sit at the counted end of some allowed pair of `edge_kind`.
- `group_by` counts per far-end attr value, type-aware (`1` ≠ `"1"`). A far end lacking
  the attr, or holding null, is reported at the edge and counted nowhere. Only values
  some edge reaches are checked.
- Dangling edges and edges touching undeclared kinds are not counted.
- The finding sits on the counted node and lists each far end with its provenance.

`CONSTRAINT`, `EDGE_WHERE` and `EDGE_COUNT` all honour repeated entries, pathway
demotion and `--strict`.

### Severities

`error`, `warning`, `info`, `hint`. `--strict` promotes `warning` to `error`; the exit
code follows ([cli.md](cli.md#exit-codes)). Overrides may demote any code to any tier.
**Hint cannot be promoted**: an attempt gives `CONFIG_ERROR` and the finding stays at `hint`.

Other words are load errors. Lint spellings get a pointer: `warn` → `warning`,
`deny` → `error`, `allow` → [`SUPPRESS`](#suppress).

### SUPPRESS

Marks a finding as expected.

```yaml
  - SUPPRESS:
      code: VACANCY
      node_ids: [UN-1, UN-2]   # optional; omit to suppress all findings of the code
  - SUPPRESS:
      code: UNTRACED
```

- A suppressed finding skips exit code 1 and `plain`/`rich`, and stays in `json` with
  `"suppressed": true`.
- Entries for one code merge `node_ids`; one without `node_ids` covers all.
- Runs **after** `--strict`. `CONFIG_ERROR` cannot be suppressed.
- Unmatched entries give `SUPPRESS_UNUSED`, itself suppressible; override it to `warning` to gate on them.

## Finding notes

`annotations: lattice-notes.yaml` names a file of the owners' decisions about findings,
resolved beside the declaring profile (the nearest under `extends`).

```yaml
- finding: {code: VACANCY, node: UN-3}
  note: The target edge lands with the M2 spec.
  until: 2026-12-31          # optional: a date, or {pathway: stage, position: M2}
  by: jone                   # optional
  at: 2026-10-01             # optional
```

- A note attaches to every finding with that code and node, and changes nothing else.
  Lattice never writes the file.
- `json` adds a `note` object; `plain` appends `| note: …`; `rich` and `browse` add a
  line. In browse, `A` opens the entry in `$EDITOR`, or shows the entry to add and a
  second `A` opens the file's end.
- `until` expires the note after a date (UTC), or once the register reaches the position
  on the named pathway. Expired notes still attach.
- `NOTE_INVALID` covers a missing `finding` or `note`, unknown keys, impossible dates,
  unknown pathways or positions, and repeats; the entry is skipped.
- A notes file other than a YAML list is a load error (exit 2). Fuse profiles cannot
  declare `annotations`; fuse keeps the notes each source's profile attached.

## Pathways

A finding bound to a pathway drops to `info` when the node's position is after the
current one or off the pathway. Findings already at `info` or `hint` keep theirs.

```yaml
pathways:
  - stage

validations:
  - VACANCY:
      pathway: stage
      position_attr: stage
```

- The adapter reads positions and the current position from the register.
- `pathway` and `position_attr` go together (a partial binding is a load error), and `pathway` must be listed in `pathways`.
- One binding per code; a second is a load error. Adapter-emitted codes can be bound.

## Inheritance

`extends` names a parent by relative path. It is merged before validation and stripped from the result.

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

Scalars: child wins. Lists: child replaces. Mappings: deep merge. `profile_version` is
the child's. Parent node kinds come first. Chains work; cycles are load errors.

## Versioning

`profile_version` is X.Y.Z without leading zeros. The major is the profile
format: the core rejects one it does not support, so it moves only with the core.

| Bump | When |
|---|---|
| minor | added vocabulary (a kind, an attr, an edge kind), or a changed ID shape |
| patch | everything formerly valid stays valid |

**Node IDs are public.** Narrowing or renaming a released ID shape needs a Breaking
changelog entry and an old-to-new ID mapping; widening, or a shape only release candidates carried, needs neither.

`--profile-version 1.2.0` makes any command that reads profile YAML refuse another
declared version (exit 2, before the adapter). The pin is X.Y.Z, with no ranges. Under
`extends` it checks the child, which must declare one. It compares the declared version
only; checking that edits bump it is a separate CI step. Without the pin, any minor and
patch of a supported major loads.

The notes file follows the profile major. Other documents carry their own versions:
`manifest_version` ([fuse.md](fuse.md)), `interface_version`
([adapters.md](adapters.md#writing-your-own-adapter)), `trace_version` and
`fuse_version` ([json.md](json.md)).

## Verification

```sh
latt show     --profile profiles/yours.yaml
latt validate --profile profiles/yours.yaml --adapter ./adapters/yours \
    --target /path/to/register-repo --format plain
```

`show` catches load errors; `validate` tests real data. Zero findings over zero edges
proves little, so compare `latt query counts` with what you expect.
