# profile-schema Specification

## Purpose
Defines the YAML profile format that declares a domain's node kinds, edge kinds, typed
attributes, and validation configuration — the contract between adapters and core.
## Requirements
### Requirement: Profile YAML structure
A profile SHALL be a single YAML file containing: `name` (string), `profile_version`
(string, three-part numeric version X.Y.Z), `node_kinds` (map), `edge_kinds` (map), and
optionally `validations` (list), `pathways` (list), `extends` (string, resolved before the
value is loaded), `annotations` (string, the notes file; see the finding-notes capability)
and `adapter` (map). The loader SHALL reject any other top-level key with
an error naming the key and listing the accepted keys, so a mistyped section such as
`validation:` is a load error rather than a silently absent one. `adapter` is the one
section the core admits without interpreting: it is preserved verbatim so a profile can
carry adapter-specific settings core has no opinion about.

The `adapter` top-level key MAY contain a `native` field (string) naming a native
adapter to use when `--adapter` is not supplied on the command line, and a
`timeout_seconds` field (positive integer) bounding how long the core waits for an
external adapter program. The core reads these two fields during adapter resolution;
all other `adapter` sub-keys remain opaque to the core and are passed through to the
adapter unchanged. The `adapter` block MAY also carry `command_timeout_seconds`
(positive integer), which the core passes through but validates: adapters read it to
bound each subprocess they run. When either timeout key is present and is not a
positive integer, the loader SHALL reject the profile with a `CONFIG_ERROR` naming
the key, so a zero or negative bound never silently means "no bound". A positive
bound larger than the clock can represent SHALL wait as if unbounded rather than
panic outside the exit-code contract.

Verified by: `cargo test --test profile_schema`,
`cargo test -p latt --locked --test finding_notes location`, `cargo test --test native`,
`cargo test --test profile_schema timeout`, and
`cargo test -p latt-adapter a_bound_beyond_the_clock_does_not_panic`.

#### Scenario: Valid profile loads
- **WHEN** a profile YAML with all required top-level keys is loaded
- **THEN** the loader returns a parsed profile object with no errors

#### Scenario: Missing required key
- **WHEN** a profile YAML omits `node_kinds`
- **THEN** the loader raises a validation error naming the missing key

#### Scenario: Unknown top-level key rejected
- **WHEN** a profile declares `validation:` (mistyped singular) beside the required keys
- **THEN** the loader rejects the profile naming `validation` and listing the accepted
  top-level keys

#### Scenario: Adapter block accepted uninterpreted
- **WHEN** a profile declares a top-level `adapter` key whose contents core does not interpret
- **THEN** the profile loads and the block is retained for the adapter to read

#### Scenario: Profile with adapter.native
- **WHEN** a profile declares `adapter: { native: md, paths: { files: ["*.md"] } }`
- **THEN** the loader preserves both `native` and `paths` in the adapter block,
  and the core reads `native` during adapter resolution

#### Scenario: Timeout keys are validated
- **WHEN** a profile declares `adapter: { timeout_seconds: 0 }`,
  `adapter: { command_timeout_seconds: -5 }` or `adapter: { command_timeout_seconds: 0.5 }`
- **THEN** the loader rejects the profile naming the key, and the command exits 2

#### Scenario: Timeout keys pass through to the adapter
- **WHEN** a profile declares `adapter: { command_timeout_seconds: 30 }`
- **THEN** the resolved profile document the adapter receives carries
  `command_timeout_seconds: 30` unchanged

#### Scenario: Profile without adapter.native
- **WHEN** a profile's `adapter` block has no `native` key
- **THEN** the profile loads normally and `--adapter` is required on the command line

#### Scenario: Annotations key accepted
- **WHEN** a profile declares `annotations: lattice-notes.yaml` beside the required keys
- **THEN** the profile loads, and an unknown-key error for any other key lists
  `annotations` among the accepted top-level keys

### Requirement: Node kind declaration
Each entry in `node_kinds` SHALL declare an `id_pattern` (regex string) and MAY declare
an `attrs` map. The `id_pattern` constrains which node IDs are valid for that kind. A
node kind with no `attrs` key SHALL load with an empty attr map, so a kind that carries
no typed attributes needs no placeholder.

A node kind MAY declare `summary_attr` (string), naming which of its declared attrs the
trace output uses as the human-readable summary column. The named attr SHALL be present
in the kind's `attrs` map and SHALL NOT be a `list` type — the summary column is a short
scalar label. The loader SHALL reject a `summary_attr` naming an undeclared attr or one
whose type is `list`. A kind with no `summary_attr` renders an empty cell in the trace's
"Key Attr" column.

A node kind MAY declare `text_attrs` (list of strings), naming which of its declared
attrs a text-ranking consumer reads, in the order given. Each name SHALL be present in
the kind's `attrs` map and SHALL be a textual type — `string`, or `enum`, whose value
arrives as a string at runtime. The loader SHALL reject a `text_attrs` entry that is
undeclared, whose type is not `string` or `enum`, or that repeats a name already in the
list. An `int`, `bool` or `list` value is never ranked text. A `date` also remains
non-textual structured data even though it arrives as a string; admitting it would turn a
wire representation into prose by accident. `text_attrs` MAY name the same attr as
`summary_attr`.

A kind with no `text_attrs` is not an error; the consumer falls back to `summary_attr`.

The loader SHALL NOT require `text_attrs` to be present, and SHALL accept an empty list
as an explicit declaration that the kind offers no text to rank. An absent `text_attrs`
and an empty `text_attrs` SHALL remain distinguishable in the resolved profile document:
they carry different meanings for a ranking consumer — absent falls back to
`summary_attr`, empty declares that there is nothing to rank — so a representation that
renders them identically SHALL NOT be used.

A node kind MAY declare `text_chunk_line_prefix` (string), a **literal** line prefix at
which a ranking consumer subdivides the text that kind offers. It is a literal compared with a string prefix test, never a regex.

The loader SHALL reject a `text_chunk_line_prefix` that is not a string, that is empty or
contains only whitespace, or that contains a carriage return or line feed. A prefix that
matches every line, or that spans a line boundary, cannot name a cut point.

The loader SHALL reject `text_chunk_line_prefix` on a node kind that offers no rankable
text — one declaring `text_attrs: []`, or declaring neither `text_attrs` nor
`summary_attr`. The loader SHALL NOT require `text_chunk_line_prefix` to be present. A kind that does not
declare one SHALL be ranked as one undivided text, so every profile written before this key
produces identical output.

#### Scenario: Node kind with pattern and attrs
- **WHEN** a node kind declares `id_pattern: "^REQ-\\d{4}$"` and attrs `{text: {type: string, required: true}}`
- **THEN** the profile accepts the declaration and the pattern is compiled to a regex

#### Scenario: Node kind with summary_attr
- **WHEN** a node kind declares `summary_attr: text` and `attrs` includes `text`
- **THEN** the profile records the summary attr for that kind

#### Scenario: summary_attr naming an undeclared attr
- **WHEN** a node kind declares `summary_attr: title` but `attrs` does not include `title`
- **THEN** the loader raises an error naming the undeclared attr

#### Scenario: summary_attr naming a list attr
- **WHEN** a node kind declares `summary_attr: refs` and `refs` has type `list`
- **THEN** the loader raises an error: the summary column is a scalar label

#### Scenario: Node kind with text_attrs
- **WHEN** a node kind declares `text_attrs: [title, body]` and `attrs` includes both
- **THEN** the resolved profile records those two names in that order

#### Scenario: text_attrs naming an undeclared attr
- **WHEN** a node kind declares `text_attrs: [title, body]` but `attrs` omits `body`
- **THEN** the loader fails with an error naming `text_attrs` and the missing attr

#### Scenario: text_attrs naming a list attr
- **WHEN** a node kind declares `text_attrs: [refs]` and `refs` has type `list`
- **THEN** the loader fails with an error naming `text_attrs`

#### Scenario: text_attrs naming a non-textual scalar attr
- **WHEN** a node kind declares `text_attrs: [count]` and `count` has type `int`
- **THEN** the loader fails with an error naming `text_attrs`, the attr, and its
  declared type

#### Scenario: text_attrs naming an enum attr
- **WHEN** a node kind declares `text_attrs: [stage]` and `stage` is an enum
- **THEN** the resolved profile records the name — an enum value is a string at runtime

#### Scenario: text_attrs repeating a name
- **WHEN** a node kind declares `text_attrs: [title, title]`
- **THEN** the loader fails with an error naming `text_attrs`

#### Scenario: Node kind with no text_attrs
- **WHEN** a node kind declares `summary_attr` and no `text_attrs`
- **THEN** the resolved profile records `text_attrs` as absent, distinct from empty

#### Scenario: Explicitly empty text_attrs is distinct from absent
- **WHEN** one node kind declares `text_attrs: []` and another omits the key entirely
- **THEN** the resolved document distinguishes the two, and a reader can tell which kind
  declared emptiness

#### Scenario: Invalid regex in id_pattern
- **WHEN** a node kind declares `id_pattern: "[invalid"`
- **THEN** the loader raises an error identifying the node kind and the malformed pattern

#### Scenario: Node kind with a chunk line prefix
- **WHEN** a node kind declaring `text_attrs: [title, body]` also declares
  `text_chunk_line_prefix: "#### Scenario:"`
- **THEN** the resolved profile records that prefix for the kind

#### Scenario: Node kind with no chunk line prefix
- **WHEN** a node kind declares `text_attrs` and no `text_chunk_line_prefix`
- **THEN** the resolved profile records the prefix as absent, and the kind's text is ranked
  undivided

#### Scenario: A non-string chunk line prefix is rejected
- **WHEN** a node kind declares `text_chunk_line_prefix: 4`
- **THEN** the loader fails with an error naming `text_chunk_line_prefix`

#### Scenario: An empty or whitespace-only chunk line prefix is rejected
- **WHEN** a node kind declares `text_chunk_line_prefix: "   "`
- **THEN** the loader fails with an error naming `text_chunk_line_prefix`, because a prefix
  matching every line names no cut point

#### Scenario: A chunk line prefix spanning a line boundary is rejected
- **WHEN** a node kind declares a `text_chunk_line_prefix` containing a line feed
- **THEN** the loader fails with an error naming `text_chunk_line_prefix`

#### Scenario: A chunk line prefix on a kind with no rankable text is rejected
- **WHEN** a node kind declares `text_attrs: []` and a `text_chunk_line_prefix`
- **THEN** the loader fails with an error explaining the kind offers no text to chunk

#### Scenario: A chunk line prefix on a kind with neither text_attrs nor summary_attr is rejected
- **WHEN** a node kind declares a `text_chunk_line_prefix` but neither `text_attrs` nor
  `summary_attr`
- **THEN** the loader fails with an error explaining the kind offers no text to chunk

Verified by: `cargo test --test profile_schema text_attrs && cargo test --test profile_schema chunk`

### Requirement: Typed attribute declarations
Attributes SHALL support six primitive types: `string`, `int`, `bool`, `date`, `enum`, `list`.
Each attr declares `type` (required), `required` (bool, default false), `values` (list,
enum only), and `items` (type name, list only).

A `date` value SHALL be a JSON string containing exactly an ISO 8601 calendar date in
ASCII `YYYY-MM-DD` form. Month/day combinations SHALL be calendar-valid, including
Gregorian leap-year rules; datetimes and non-zero-padded dates SHALL not match.

A list's `items` SHALL name a scalar type only — `string`, `int`, `bool`, or `date`. Lists of
lists and lists of enums are rejected, keeping list validation a single flat pass.

#### Scenario: Date attr
- **WHEN** an attr declares `{type: date}` and its value is `"2030-01-15"`
- **THEN** the attr passes type validation without any date-specific profile sub-field

#### Scenario: Invalid calendar date
- **WHEN** an attr declared as `date` has value `"2030-02-29"`
- **THEN** validation emits `ATTR_TYPE` identifying the required `YYYY-MM-DD` date form

#### Scenario: Enum attr with values
- **WHEN** an attr declares `{type: enum, values: [done, partial, todo, blocked]}`
- **THEN** the profile records the allowed enum values for validation

#### Scenario: List attr with items type
- **WHEN** an attr declares `{type: list, items: date}`
- **THEN** the profile records the list element type for validation

#### Scenario: Enum without values
- **WHEN** an attr declares `{type: enum}` with no `values` key
- **THEN** the loader raises an error: enum type requires `values`

#### Scenario: List without items
- **WHEN** an attr declares `{type: list}` with no `items` key
- **THEN** the loader raises an error: list type requires `items`

#### Scenario: List of a non-scalar type
- **WHEN** an attr declares `{type: list, items: enum}`
- **THEN** the loader raises an error naming the valid item types

### Requirement: Edge kind declaration
Each entry in `edge_kinds` MAY declare `allowed`: a list of `[source_kind, target_kind]`
pairs constraining which node kinds may participate in edges of that kind. An edge kind
with no `allowed` key SHALL load with an empty pair list, which permits no pairing — any
edge of that kind whose endpoints both exist and have declared kinds then produces an
`EDGE_CONSTRAINT` finding.

#### Scenario: Valid edge kind
- **WHEN** an edge kind declares `allowed: [[test, req]]`
- **THEN** edges of that kind are only valid from nodes of kind `test` to nodes of kind `req`

#### Scenario: Edge kind references undefined node kind
- **WHEN** an edge kind's `allowed` pair names a node kind not in `node_kinds`
- **THEN** the loader raises an error identifying the undefined kind

### Requirement: Unknown keys in declarations
A node kind SHALL admit only `id_pattern`, `attrs`, `summary_attr`, `text_attrs`,
`text_chunk_line_prefix`, `orphan_ok`, `actions` and `id_names`; an attr declared in mapping form SHALL admit
only `type`, `required`, `values` and `items`; an edge kind SHALL admit only `allowed`
and `cross_source`. The loader SHALL reject any other key with an error naming the
declaration, the offending key and the accepted keys. As with validation entries, a typo
becomes a load error instead of a setting that silently does nothing.

Verified by: `cargo test --test profile_schema unknown`.

#### Scenario: Unknown node kind key rejected
- **WHEN** a node kind declares `id_patern` (mistyped) in place of `id_pattern`
- **THEN** the loader rejects the profile naming the kind, `id_patern`, and the accepted keys

#### Scenario: Unknown attr key rejected
- **WHEN** an attr declares `{ type: string, require: true }`
- **THEN** the loader rejects the profile naming the kind, the attr, `require`, and the
  accepted keys

#### Scenario: Unknown edge kind key rejected
- **WHEN** an edge kind declares `allow:` in place of `allowed:`
- **THEN** the loader rejects the profile naming the edge kind, `allow`, and the accepted keys

### Requirement: Profile version
Every profile SHALL carry a `profile_version` key (three-part numeric version string,
X.Y.Z where each component is a non-negative integer without leading zeros). Core SHALL reject a
profile whose `profile_version` major exceeds the supported range.

The major is the profile format: a profile of a major the core does not support
cannot load, so it changes only with the core's supported range. Within it, a
profile's author bumps `profile_version` by what the change does to the register's
consumers: **minor** for added vocabulary (a kind, an attr, an edge kind) or a changed
ID shape, **patch** for a change under which all formerly valid data stays valid.
Narrowing or renaming an ID shape that shipped in a release SHALL carry a Breaking
changelog entry and a deterministic old-to-new ID mapping in the same minor bump; a
shape that only release candidates carried needs neither. Widening an `id_pattern`
keeps every old ID valid.

A caller MAY additionally pin an exact `profile_version`: `Profile::require_version`
SHALL accept the profile only when its `profile_version` equals the pin, and SHALL refuse
a pin that is not itself X.Y.Z without leading zeros. Under `extends:` the compared
version is the child's. The pin is opt-in; the loader alone checks only the major. The CLI
exposes it as `--profile-version` (the cli capability, Requirement: Profile version pin).

#### Scenario: Supported version
- **WHEN** a profile declares `profile_version: "1.0.0"` and core supports major 1
- **THEN** the profile loads normally

#### Scenario: Unsupported major version
- **WHEN** a profile declares `profile_version: "2.0.0"` and core supports only major 1
- **THEN** core raises an error: unsupported profile version

#### Scenario: Exact pin
- **WHEN** a profile declares `profile_version: "1.4.2"` and a caller pins `1.4.2`, then
  `1.4.3`
- **THEN** the first pin accepts the profile and the second refuses it naming both versions

#### Scenario: Malformed pin
- **WHEN** a caller pins `1.0`, `01.0.0` or `1.0.0-rc.1`
- **THEN** the pin is refused as not a valid version, whatever the profile declares

#### Scenario: Pin under extends
- **WHEN** a child declaring `profile_version: "1.2.0"` extends a parent declaring `"1.1.0"`
- **THEN** a pin of `1.2.0` accepts the profile and a pin of `1.1.0` refuses it

### Requirement: Suppress configuration in profiles
The `validations` list SHALL accept entries keyed by `SUPPRESS`. Each entry
SHALL declare:

- `code` (string, required): the finding code to suppress.
- `node_ids` (list of strings, optional): specific node IDs to suppress. When
  absent, all findings of the code are suppressed.

Multiple `SUPPRESS` entries SHALL be accepted. Entries for the same code merge
their `node_ids` lists (union). A `SUPPRESS` entry with no `node_ids`
supersedes any ID-specific entry for the same code.

The loader SHALL reject a `SUPPRESS` entry missing `code`, or with `code` not
a string, or with `node_ids` not a list of strings. The loader SHALL also
reject a `SUPPRESS` entry with `code: "CONFIG_ERROR"` — a profile that
silences configuration errors would defeat the invariant that unreadable
input is reported, never dropped.

`SUPPRESS` is a selector over findings, not a validator's configuration, so
severity overrides and pathway bindings for a code attach to that code's own
`validations` entry, never to a `SUPPRESS` entry (see the validation
configuration schema requirement).

Verified by: `cargo test --test profile_schema suppress`

#### Scenario: Valid suppress entry loads
- **WHEN** the profile declares `- SUPPRESS: {code: VACANCY, node_ids: [UN-1]}`
- **THEN** the profile loads with one suppress entry

#### Scenario: Suppress-all loads
- **WHEN** the profile declares `- SUPPRESS: {code: UNREFERENCED}`
- **THEN** the profile loads with a suppress-all entry for that code

#### Scenario: Missing code is rejected
- **WHEN** the profile declares `- SUPPRESS: {node_ids: [X]}`
- **THEN** the loader rejects the profile

#### Scenario: node_ids of the wrong shape is rejected
- **WHEN** the profile declares `- SUPPRESS: {code: VACANCY, node_ids: "UN-1"}`
- **THEN** the loader rejects the profile naming `node_ids` and `got string`

#### Scenario: CONFIG_ERROR suppression is rejected
- **WHEN** the profile declares `- SUPPRESS: {code: CONFIG_ERROR}`
- **THEN** the loader rejects the profile

### Requirement: Validation configuration schema
Each entry in `validations` maps a validator code to its configuration. Core SHALL accept
only the keys that code defines — `COVERAGE` takes `severity`, `target_kind`, `edge_kind`, `where`, `source_where`;
`COVERAGE_DEEP` takes `severity`, `target_kind`, `evidence`, `via`, `where`, `source_where`;
`SUMMARY` takes `severity`, `node_kind`, `status_attr`, `group_by_attr`;
`CONSTRAINT` takes `severity`, `kind`, `when`, `expect`, `reject`, `message`;
`EDGE_WHERE` takes `severity`, `edge_kind`, `when`, `expect`, `reject`, `message`;
`EDGE_COUNT` takes `severity`, `edge_kind`, `node_kind`, `direction`, `group_by`, `min`, `max`, `where`, `message`;
`SUPPRESS` takes `code`, `node_ids` — and SHALL
reject an unknown key rather than ignoring it. A code core does not know SHALL accept
`severity` alone, so a profile can set the severity of a third-party validator. Every
entry except `SUPPRESS` SHALL also accept `message` (see the validation spec's
declared-message requirement).

A retired finding code (`ORPHAN_NODE`, `DANGLING_REF`, `AXIS_UNRESOLVED`,
`AXIS_INVALID`) SHALL be a load error naming the code and its replacement, in a profile
and in a fuse profile alike. An override naming a retired code would otherwise apply to
nothing in silence. The retired list SHALL have one definition, and the contracts gate
SHALL pin it.

In addition, any entry — for a code core implements or one it does not — SHALL accept the
pathway-binding keys `pathway` and `position_attr`, subject to the pathway-binding requirements
above. The binding is orthogonal to what the code means, so it is available to adapter
codes on the same terms as built-in ones.

`SUPPRESS` is the one entry that is not a code's configuration: it selects findings
by the `code` it names. It SHALL NOT accept `severity`, `pathway`, or
`position_attr` — a severity or binding on a selector would be a setting with
nothing to act on, and the loader SHALL reject it as an unknown key like any other.

An invalid severity string SHALL produce a diagnostic listing
all accepted severity values: error, hint, info, warning. For the lint-level words of
cargo-semver-checks and Rust, the diagnostic SHALL also name lattice's spelling: `warn`
→ `warning`, `deny` → `error`, and `allow` → a `SUPPRESS` entry. The `allow` diagnostic
SHALL say that a suppressed finding is still computed and kept in JSON, where an `allow`
lint does not run.

A `message` value that is not a string SHALL be a load error naming
the code. The declared message is never silently replaced by the generated one. A
`CONSTRAINT` or `EDGE_WHERE` entry whose `expect` and `reject` blocks together hold no
condition SHALL be a load error, because it would check nothing and always read clean.

Verified by: `cargo test --test profile_schema`.

#### Scenario: Non-string message rejected
- **WHEN** a profile declares `EDGE_COUNT` with `message: {a: 1}`
- **THEN** the loader rejects the profile: `EDGE_COUNT` `message` must be a string

#### Scenario: Entry with empty blocks rejected
- **WHEN** a profile declares `CONSTRAINT` with `kind: req` and `expect: {}`
- **THEN** the loader rejects the profile: the entry holds no conditions

#### Scenario: Unknown key rejected
- **WHEN** a profile declares `COVERAGE` with a `target_kinds` key (mistyped plural)
- **THEN** the loader raises an error naming the offending key

#### Scenario: Unknown validator code takes severity
- **WHEN** a profile sets `severity` on a validator code core does not implement
- **THEN** the profile loads and the override applies to issues carrying that code
  (see the validation spec's severity-override requirement)

#### Scenario: Adapter code takes a pathway binding
- **WHEN** a profile binds a pathway to `OBLIGATION_UNBACKED`, a code core does not implement
- **THEN** the profile loads and the binding applies to adapter issues carrying that code

#### Scenario: SUPPRESS with an unknown key rejected
- **WHEN** a profile declares `SUPPRESS` with `code: VACANCY` and a `node_id` key
  (mistyped singular)
- **THEN** the loader raises an error naming the offending key

#### Scenario: SUPPRESS does not take severity or a pathway binding
- **WHEN** a profile declares `SUPPRESS` with `code: VACANCY` and `severity: info`
- **THEN** the loader raises an error naming `severity` as an unknown key for `SUPPRESS`

#### Scenario: CONSTRAINT config with all keys loads
- **WHEN** a profile declares `CONSTRAINT` with `kind`, `when`, `expect`, `reject`, `message`, and `severity`
- **THEN** the profile loads and all six keys are available to the validator

#### Scenario: CONSTRAINT with only expect loads
- **WHEN** a profile declares `CONSTRAINT` with `kind: req` and `expect: { status: { present: true } }` and no `reject` or `when`
- **THEN** the profile loads

#### Scenario: CONSTRAINT with only reject loads
- **WHEN** a profile declares `CONSTRAINT` with `kind: req` and `reject: { priority: { eq: "" } }` and no `expect`
- **THEN** the profile loads

#### Scenario: CONSTRAINT with neither expect nor reject rejected
- **WHEN** a profile declares `CONSTRAINT` with `kind: req` and no `expect` or `reject`
- **THEN** the loader emits a `CONFIG_ERROR`: CONSTRAINT requires at least one of `expect` or `reject`

#### Scenario: CONSTRAINT with unknown condition operator rejected
- **WHEN** a profile declares `CONSTRAINT` with `expect: { status: { between: [1, 5] } }`
- **THEN** the loader emits a `CONFIG_ERROR` naming the unknown operator

#### Scenario: Ordering operator rejects non-comparable value
- **WHEN** a profile declares `CONSTRAINT` with `expect: { count: { lt: [1, 2] } }`
- **THEN** the loader emits a `CONFIG_ERROR`: ordering operators (`lt`, `gt`, `lte`, `gte`) require an integer or string value

#### Scenario: Ordering operator rejects float value
- **WHEN** a profile declares `CONSTRAINT` with `expect: { count: { gte: 2.5 } }`
- **THEN** the loader emits a `CONFIG_ERROR`: ordering operators require an integer or string value

#### Scenario: CONSTRAINT missing kind rejected
- **WHEN** a profile declares `CONSTRAINT` with `expect` but no `kind`
- **THEN** the loader emits a `CONFIG_ERROR`: CONSTRAINT requires `kind`

#### Scenario: CONSTRAINT kind names undeclared node kind
- **WHEN** a profile declares `CONSTRAINT` with `kind: widget` and `widget` is not in `node_kinds`
- **THEN** the loader emits a `CONFIG_ERROR` naming the undeclared kind

#### Scenario: A retired code is refused
- **WHEN** a profile declares `ORPHAN_NODE: { severity: info }`
- **THEN** the loader rejects the profile, exit 2, naming `UNREFERENCED` and `UNTRACED`
  as the replacement

#### Scenario: A retired code in a fuse profile is refused
- **WHEN** a fuse profile declares `DANGLING_REF: { severity: error }`
- **THEN** fuse rejects the fuse profile, exit 2, naming `VACANCY` as the replacement

#### Scenario: A lint-level word names lattice's spelling
- **WHEN** a validation override declares `severity: warn`
- **THEN** the loader rejects it, lists the accepted values, and names `warning`

#### Scenario: allow points at SUPPRESS and states the difference
- **WHEN** a validation override declares `severity: allow`
- **THEN** the loader rejects it, names a `SUPPRESS` entry, and says the suppressed
  finding is still kept in JSON

#### Scenario: message is accepted on a severity-only code
- **WHEN** a profile declares `VACANCY: { severity: error, message: "see ADR-12: {message}" }`
- **THEN** the profile loads

#### Scenario: Invalid severity lists hint among accepted values
- **WHEN** a validation override declares severity loud
- **THEN** the loader rejects it and lists error, hint, info, warning as valid values

### Requirement: Ordering pathway declaration
A profile MAY declare a `pathways` list naming the pathways it binds to. Core SHALL accept a list
of names and SHALL reject any other shape rather than ignoring it.

The list declares that a pathway exists and names it. It SHALL NOT carry the pathway values —
neither the ordered positions nor the current position. Those are target state, supplied
by the adapter at ingest; a copy held in the profile would advance without lattice
noticing, which is the hand-maintained derived state the tool exists to eliminate.

Verified by: `cargo test --test profile_schema pathway_list_loads`

#### Scenario: Pathway declared
- **WHEN** a profile declares `pathways: [phase]`
- **THEN** the profile loads and exposes a pathway named `phase`

#### Scenario: Pathway carrying values rejected
- **WHEN** a profile declares `pathways` as a mapping carrying `order` or `current`
- **THEN** the loader raises an error naming the offending shape

#### Scenario: No pathways list
- **WHEN** a profile declares no `pathways` list
- **THEN** the profile loads with no pathways, and no severity resolution is bound

### Requirement: Validation entry pathway binding
A `validations` entry MAY carry `pathway` and `position_attr`, binding that finding code's
severity to a node attribute's position on a declared pathway. Both SHALL be present
together: core SHALL reject an entry carrying one without the other, because a partial
binding is a setting that silently does nothing.

`pathway` SHALL name a pathway declared in the same profile's `pathways` list. Core SHALL reject a
binding naming an undeclared pathway at load time.

A demoted finding becomes `info`, always; there is no configurable demotion target.

Verified by: `cargo test --test profile_schema binding`

#### Scenario: Complete binding loads
- **WHEN** a profile declares `pathways: [phase]` and binds `OBLIGATION_UNBACKED` with
  `pathway: phase`, `position_attr: trigger`
- **THEN** the profile loads and the binding is exposed for that code

#### Scenario: Partial binding rejected
- **WHEN** a validation entry carries `pathway` but no `position_attr`
- **THEN** the loader raises an error naming the missing key

#### Scenario: Binding names an undeclared pathway
- **WHEN** a validation entry binds to pathway `phase` and no `pathways` list declares `phase`
- **THEN** the loader raises an error naming the undeclared pathway

### Requirement: One pathway binding per finding code
Core SHALL reject a profile that binds a pathway to a code already bound, naming the
duplicated code. This is a load error, not a silent discard of either binding.

A profile may legitimately configure the same validation code more than once, and the
validation spec requires every such entry to be honoured independently. A pathway binding is
the exception: two bindings for one code would give the resolution pass two answers for
the same finding with no rule to choose between them. Rejecting at load is the only
outcome that neither drops a declared setting nor resolves arbitrarily.

Verified by: `cargo test --test profile_schema second_binding_for_one_code_is_rejected`

#### Scenario: Second binding for the same code rejected
- **WHEN** a profile carries two `OBLIGATION_UNBACKED` entries, each with a `pathway` and
  `position_attr`
- **THEN** the loader raises an error naming the duplicated code

#### Scenario: Repeated non-binding configuration still honoured
- **WHEN** a profile carries two `COVERAGE` entries, neither carrying a `pathway`
- **THEN** the profile loads and both configurations are honoured

#### Scenario: One binding alongside a repeated plain configuration
- **WHEN** a profile carries two `COVERAGE` entries and exactly one of them binds a pathway
- **THEN** the profile loads, both configurations are honoured, and the single binding
  applies

### Requirement: Profile inheritance via extends
A profile MAY declare `extends: <path>` (string) naming a single parent profile.
The path SHALL be resolved relative to the directory of the file that declares it.
The core SHALL load the parent, merge parent into child on the raw parsed value
before any structural validation, and strip `extends` from the merged result so
the resolved document never carries it.

Merge rules:
- Scalars: child value wins.
- Lists (YAML sequences): child replaces the parent's list whole — no element
  merging. A child restating `allowed` or `validations` owns that list entirely.
- Mappings: recursive deep merge. A child mapping key that also exists in the
  parent is merged (if both sides are mappings) or replaced (if the child's
  value is a scalar or list). A parent key not present in the child is inherited.

`profile_version` SHALL come from the child file alone. A child that omits
`profile_version` SHALL be rejected even when the parent declares one, because
the child is the document of record for version compatibility.

Node-kind declaration order in the merged result SHALL be parent-first (in the
parent's declared order), then child-only additions (in the child's declared
order). A child key that overrides a parent kind keeps the parent's position.

A parent that is not a YAML mapping SHALL be rejected at load time, naming the
parent file.

Verified by: `cargo test --test profile_schema child_inherits scalar_child_wins list_replaces mapping_deep_merge child_missing resolved_document chain direct_cycle self_extends non_string missing_parent parent_first non_mapping`

#### Scenario: Child inherits parent's node kinds and edge kinds
- **WHEN** a parent profile declares node kind `req` and edge kind `derives`, and a
  child declares `extends: parent.yaml` with no `node_kinds` or `edge_kinds`
- **THEN** the loaded profile contains `req` and `derives` from the parent

#### Scenario: Scalar child-wins
- **WHEN** a parent declares `name: parent` and a child declares `name: child`
  with `extends: parent.yaml`
- **THEN** the loaded profile's name is `child`

#### Scenario: List replaces whole
- **WHEN** a parent declares a `validations` list with two entries and a child
  declares `validations` with one entry
- **THEN** the loaded profile's validations contain only the child's entry

#### Scenario: Mapping deep-merge inherits parent attrs
- **WHEN** a parent declares node kind `req` with `id_pattern` and `attrs`, and
  a child declares `req` with only a new `id_pattern`
- **THEN** the loaded profile's `req` has the child's pattern and the parent's attrs

#### Scenario: Child missing profile_version is rejected
- **WHEN** a child profile omits `profile_version` and the parent declares one
- **THEN** the loader raises an error: the child must declare its own version

#### Scenario: Resolved document contains merged content without extends
- **WHEN** a child extends a parent and `resolved_document` is serialized
- **THEN** the JSON contains merged node kinds from both profiles and no
  `extends` key

#### Scenario: Non-mapping parent is rejected
- **WHEN** a child extends a parent file that contains a YAML list, not a mapping
- **THEN** the loader raises an error naming the parent file

### Requirement: Inheritance chain support
Chains of inheritance (A extends B extends C) SHALL be supported. Each link in
the chain follows the same merge rules.

Verified by: `cargo test --test profile_schema chain_three_levels`

#### Scenario: Three-level chain
- **WHEN** profile C extends B which extends A, and each adds a node kind
- **THEN** the loaded profile contains all three node kinds in order: A's, B's, C's

### Requirement: Inheritance cycle detection
The loader SHALL reject a profile whose `extends` chain forms a cycle, including
a profile that extends itself. The error message SHALL name the file that closes
the cycle. Cycle detection SHALL use canonicalized file paths so that different
relative paths to the same file are recognized.

Verified by: `cargo test --test profile_schema direct_cycle self_extends`

#### Scenario: Direct cycle
- **WHEN** profile A extends B and profile B extends A
- **THEN** the loader raises an error naming the file that closes the cycle

#### Scenario: Self-extends
- **WHEN** a profile declares `extends` pointing to itself
- **THEN** the loader raises an error naming the file

### Requirement: Extends key validation
The `extends` key, when present, SHALL be a string. A non-string value (list,
mapping, number, boolean) SHALL be rejected at load time. A path that does not
resolve to an existing file SHALL be rejected with an error naming the path.

Verified by: `cargo test --test profile_schema non_string_extends missing_parent`

#### Scenario: Non-string extends is rejected
- **WHEN** a profile declares `extends: [a.yaml, b.yaml]`
- **THEN** the loader raises an error: extends must be a string

#### Scenario: Missing parent file is rejected
- **WHEN** a profile declares `extends: nonexistent.yaml`
- **THEN** the loader raises an error naming the missing file

### Requirement: Node-kind declaration order across merge
Node kinds in the merged profile SHALL be ordered: parent's kinds first (in the
parent's declaration order), then child-only additions (in the child's
declaration order). A child kind that overrides a parent kind retains the
parent's position, not the child's. This order determines `declared_index`,
which controls trace grouping.

Verified by: `cargo test --test profile_schema parent_first_declared_index`

#### Scenario: Parent-first ordering
- **WHEN** a parent declares kinds `[alpha, beta, delta]` and a child declares
  `[beta, gamma]` (overriding beta, adding gamma)
- **THEN** `declared_index` order is alpha=0, beta=1, delta=2, gamma=3

### Requirement: Profile errors use the native type vocabulary
Profile loading error messages that name a YAML value's runtime type SHALL use the
native vocabulary (`string`, `int`, `float`, `bool`, `list`, `object`, `null`),
not Python's (`str`, `dict`, `NoneType`). The declared attr-type vocabulary additionally
includes `date`; because a date's runtime representation is a string, `date` SHALL be used
only where an expected schema type is known.

Verified by: `cargo test --test native_messages`

#### Scenario: Null pathways reported with a native type name
- **WHEN** a profile declares `pathways: null`
- **THEN** the loading error reads `'pathways' must be a list of pathway names, got
  null`, followed by the standing guidance that pathway values are target state

### Requirement: Node kind orphan policy flag
A node kind MAY declare `orphan_ok: true`, exempting nodes of that kind from the
`UNREFERENCED`/`UNTRACED` validators. The value SHALL be a boolean; any other type SHALL be rejected
at load time naming the kind and the value's type in the native vocabulary. Absence
means `false` — the exemption is opt-in, and an existing profile's behaviour does not
change by omission.

The flag is validation policy only. It SHALL NOT affect ingest, queries, or any other
validator: an `orphan_ok` node still appears in `query orphans`, still counts, and still
raises every other finding it earns.

Verified by: `cargo test --test profile_schema orphan_ok`

#### Scenario: Flag loads
- **WHEN** a node kind declares `orphan_ok: true`
- **THEN** the profile loads and exposes the exemption for that kind

#### Scenario: Non-boolean flag rejected
- **WHEN** a node kind declares `orphan_ok: "yes"`
- **THEN** the loader raises an error naming the kind and `got string`

#### Scenario: Absent flag defaults to false
- **WHEN** a node kind declares no `orphan_ok` key
- **THEN** nodes of that kind are subject to `UNREFERENCED`/`UNTRACED` as before

### Requirement: Edge kinds support cross_source flag
An edge kind declaration in a profile SHALL accept an optional `cross_source: true`
field. When present, the validation layer SHALL demote VACANCY findings for
edges of that kind whose target does not resolve to hint severity instead of the
default error severity. This applies in standalone (single-source) runs only — in a
fuse context, cross-source edges resolve normally after merge.

The flag is profile data, not core vocabulary. The core reads it as a severity
modifier during VACANCY collection, not as a behavioral switch.

A profile that does not declare `cross_source` on any edge kind has no change in
behavior.

The demotion applies after the profile's `VACANCY` override, as a pathway demotion
does. Hint is the lowest severity, so the override cannot raise a cross-source
`VACANCY` above it: that would promote a hint, which the validation spec forbids.
The override still applies to `VACANCY` on other edge kinds, and it is not a
refused promotion, so it produces no `CONFIG_ERROR`.

Verified by: `cargo test -p latt-core --lib dangling_target`,
`cargo test -p latt-core --lib vacancy_override_applies_to_ordinary_edges_and_cross_source_stays_hint`,
and `.venv/bin/python -m pytest tests/test_adapter_md.py` (profile with cross_source edge)

#### Scenario: Cross-source edge kind demotes dangling ref
- **WHEN** a profile declares edge kind `derives` with `cross_source: true` and an
  edge of that kind targets an ID that does not exist in the graph
- **THEN** the VACANCY finding for that edge has hint severity, not error

#### Scenario: Non-cross-source edge kind unchanged
- **WHEN** a profile declares edge kind `contains` without `cross_source` and an edge
  of that kind targets a missing ID
- **THEN** the VACANCY finding has its default severity (error)

#### Scenario: Flag absent means no change
- **WHEN** no edge kind in the profile declares `cross_source`
- **THEN** all VACANCY findings use their default or profile-overridden severity

#### Scenario: A VACANCY override does not reach a cross-source edge
- **WHEN** a profile declares edge kind `derives` with `cross_source: true` and edge kind
  `contains` without it, overrides `VACANCY` to `warning`, and an edge of each kind
  targets a missing ID
- **THEN** the `contains` VACANCY has severity `warning`, the `derives` VACANCY stays
  `hint`, and no `CONFIG_ERROR` is emitted

### Requirement: EDGE_WHERE configuration
An `EDGE_WHERE` entry SHALL name a declared edge kind in `edge_kind` and SHALL declare
at least one of `expect` or `reject`. Each of `when`, `expect` and `reject` SHALL be a
mapping whose keys are drawn from `source`, `target` and `edge`. Each of those keys
SHALL hold a condition block with the same syntax and operators as `CONSTRAINT`. Any
other key in a block, or a block that is not a mapping, SHALL be a load error naming the
block. `severity` SHALL be a valid severity string.

Verified by: `cargo test --test validation edge_where_config`

#### Scenario: EDGE_WHERE config with all keys loads
- **WHEN** a profile declares `EDGE_WHERE` with `edge_kind`, `when`, `expect`, `reject`, `message` and `severity`
- **THEN** the profile loads

#### Scenario: EDGE_WHERE missing edge_kind rejected
- **WHEN** a profile declares `EDGE_WHERE` with `reject` but no `edge_kind`
- **THEN** the loader rejects the profile: `EDGE_WHERE` requires `edge_kind`

#### Scenario: EDGE_WHERE edge_kind names an undeclared edge kind
- **WHEN** a profile declares `EDGE_WHERE` with `edge_kind: picks` and `picks` is not in `edge_kinds`
- **THEN** the loader rejects the profile naming `picks`

#### Scenario: EDGE_WHERE with neither expect nor reject rejected
- **WHEN** a profile declares `EDGE_WHERE` with only `edge_kind`
- **THEN** the loader rejects the profile: `EDGE_WHERE` requires at least one of `expect` or `reject`

#### Scenario: EDGE_WHERE block key outside the selectors rejected
- **WHEN** a profile declares `EDGE_WHERE` with `reject: { status: { eq: blocked } }`, an attr where a selector belongs
- **THEN** the loader rejects the profile naming `reject.status` and the selectors `source`, `target`, `edge`

#### Scenario: EDGE_WHERE unknown operator rejected
- **WHEN** a profile declares `EDGE_WHERE` with `expect: { target: { count: { between: [1, 5] } } }`
- **THEN** the loader rejects the profile naming the unknown operator `between`

### Requirement: EDGE_COUNT configuration
An `EDGE_COUNT` entry SHALL name a declared edge kind in `edge_kind` and a declared
node kind in `node_kind`. `direction` SHALL be `out` or `in` and defaults to `out`. At
least one allowed pair of the edge kind SHALL place `node_kind` at the counted end: the
source for `out`, the target for `in`. Otherwise every node of the kind would count
zero, which is a misconfiguration and not a finding. At least one of `min` and `max`
SHALL be declared. Each SHALL be a non-negative integer, and `min` SHALL NOT exceed
`max`. `group_by` SHALL be a string. `where` SHALL be a `CONSTRAINT`-syntax condition
block on the counted node.

Verified by: `cargo test --test validation edge_count_config`

#### Scenario: EDGE_COUNT config with all keys loads
- **WHEN** a profile declares `EDGE_COUNT` with every key
- **THEN** the profile loads

#### Scenario: EDGE_COUNT node kind never at the counted end
- **WHEN** an entry has `edge_kind: fulfills`, `node_kind: need`, `direction: out`, and no allowed `fulfills` pair has `need` as source
- **THEN** the loader rejects the profile naming `need`, `fulfills` and the direction

#### Scenario: EDGE_COUNT without bounds rejected
- **WHEN** an entry declares neither `min` nor `max`
- **THEN** the loader rejects the profile: `EDGE_COUNT` requires at least one of `min` or `max`

#### Scenario: EDGE_COUNT min above max rejected
- **WHEN** an entry declares `min: 2` and `max: 1`
- **THEN** the loader rejects the profile naming both bounds

#### Scenario: EDGE_COUNT invalid direction rejected
- **WHEN** an entry declares `direction: both`
- **THEN** the loader rejects the profile naming `both` and the accepted values

### Requirement: Node kind actions
A node kind MAY declare `actions`: a mapping from verb name to an action. The loader
SHALL keep the verbs in declared order. A verb name SHALL be a non-empty string; the
core gives no verb a meaning. An action SHALL be a mapping that admits only `run` and
`mode`, and both are required:

- `run`: a non-empty list of strings, never a single string. The first element names
  the program and SHALL contain no placeholder.
- `mode`: one of `interactive`, `capture`, `detached`.

Inside a `run` element, `{` opens a placeholder and `}` closes it. `{{` and `}}` are
literal braces. The loader SHALL accept only `{id}`, `{kind}`, `{provenance.file}`,
`{provenance.line}` and `{attrs.<name>}` where `<name>` is an attr the kind declares.
The loader SHALL reject an unknown placeholder, an `{attrs.…}` naming an undeclared attr
or a `list` attr, an unclosed `{` and a lone `}`. Each error names the kind, the verb
and the offending text. A kind with no `actions` key has none. The resolved profile
document carries `actions` as written.

Verified by: `cargo test -p latt --locked --test profile_schema actions`.

#### Scenario: Node kind with actions
- **WHEN** a node kind declares
  `actions: {show: {run: ["ent", "show", "{id}"], mode: capture}}`
- **THEN** the profile loads and the kind carries one action, `show`

#### Scenario: Shell string rejected
- **WHEN** an action declares `run: "ent show {id}"`
- **THEN** the loader rejects the profile naming the kind, the verb and `run`

#### Scenario: Unknown mode rejected
- **WHEN** an action declares `mode: split`
- **THEN** the loader rejects the profile naming the kind, the verb, `split` and the
  accepted modes

#### Scenario: Placeholder naming an undeclared attr
- **WHEN** a kind that declares no `number` attr has an action using `{attrs.number}`
- **THEN** the loader rejects the profile naming the kind, the verb and `{attrs.number}`

#### Scenario: Placeholder in the program
- **WHEN** an action declares `run: ["{attrs.tool}", "show"]`
- **THEN** the loader rejects the profile naming the kind and the verb

#### Scenario: Unknown key in an action
- **WHEN** an action declares `cmd:` in place of `run:`
- **THEN** the loader rejects the profile naming the kind, the verb, `cmd` and the
  accepted keys

### Requirement: Coverage entries name declared kinds
A standard profile's `COVERAGE` and `COVERAGE_DEEP` entries SHALL be checked when the
profile loads, as `CONSTRAINT`, `EDGE_WHERE` and `EDGE_COUNT` entries are. A missing
kind key, a kind key whose value is not a string, a `target_kind` naming no declared
node kind, and an `edge_kind`, `via` or `evidence` naming no declared edge kind SHALL
each be a load error naming the code, the key and the value, so the command exits 2
before any adapter runs. A coverage entry that cannot run is a broken setup, not a
finding about the register: reported as a finding, it exits 1 beside real findings and
reads as one.

The profile `fuse` composes from its sources is exempt. Its kinds are the kinds the
sources' traces carried, known only after they have run, so on that profile the
validation spec's `CONFIG_ERROR` finding stands (program-composition spec, standard
validators on the composed graph).

Verified by: `cargo test --test profile_schema coverage_kind`

#### Scenario: COVERAGE target_kind names an undeclared node kind
- **WHEN** a profile declares `COVERAGE` with `target_kind: widget` and `widget` is not in `node_kinds`
- **THEN** the loader raises an error naming `COVERAGE`, `target_kind` and `widget`

#### Scenario: COVERAGE_DEEP evidence names an undeclared edge kind
- **WHEN** a profile declares `COVERAGE_DEEP` with `evidence: nosuch` and `nosuch` is not in `edge_kinds`
- **THEN** the loader raises an error naming `COVERAGE_DEEP`, `evidence` and `nosuch`

#### Scenario: COVERAGE missing a kind key
- **WHEN** a profile declares `COVERAGE` with `target_kind: req` and no `edge_kind`
- **THEN** the loader raises an error naming the missing key

#### Scenario: Validate exits 2 on an undeclared coverage kind
- **WHEN** `latt validate` runs with a profile whose `COVERAGE` names an undeclared kind
- **THEN** it exits 2 with the load error and does not run the adapter

### Requirement: Source address declaration
A node kind MAY declare `id_names`, stating that its IDs are addresses in the target
(their resolution is the source-existence capability's). The value SHALL be either a
preset name or a structured block:

- `file` — the ID is a path.
- `cargo` — shorthand for `{ language: rust, separator: "::" }`.
- `pytest` — shorthand for `{ language: python, separator: "::", params: "[]" }`.
- `gotest` — shorthand for `{ language: go, separator: "::" }`.
- A mapping with `language` (`rust`, `python` or `go`) and `separator` (a non-empty string), both
  required, and optional `params` (exactly two characters, opener then closer). It admits
  no other key.

The loader SHALL reject an unknown preset, an unknown `language`, a missing or empty
`separator`, a `params` that is not two characters, an unknown key, and a value of any
other type. Each error SHALL name the kind, `id_names`, the offending value or key, and
what is accepted. The accepted forms are the source-existence resolver's own, so the
loader and the resolver cannot disagree. A language with no resolver is rejected, not reduced to a file
check. The key SHALL reach adapters unchanged in the resolved profile document: a
preset stays a preset, and is expanded by the resolver. A kind without the key declares
nothing about its IDs, so every profile written before it validates identically.
`id_names: null` SHALL read as absent, as a null `orphan_ok` or `actions` does.

Verified by: `cargo test -p latt --test profile_schema id_names`

#### Scenario: Presets load
- **WHEN** four kinds declare `id_names: file`, `id_names: cargo`, `id_names: pytest` and `id_names: gotest`
- **THEN** the profile loads, and the resolved document carries each value as written

#### Scenario: Structured block loads
- **WHEN** a kind declares `id_names: { language: rust, separator: "::" }`
- **THEN** the profile loads, and the resolved document carries the mapping as written

#### Scenario: Unknown preset rejected
- **WHEN** a kind declares `id_names: jest`
- **THEN** the loader rejects the profile naming the kind, `id_names`, `jest`, and the
  accepted presets `file`, `cargo`, `pytest` and `gotest`

#### Scenario: Unknown language rejected
- **WHEN** a kind declares `id_names: { language: ruby, separator: "::" }`
- **THEN** the loader rejects the profile naming the kind, `ruby`, and the accepted
  languages `rust`, `python` and `go`

#### Scenario: Structured block without a separator rejected
- **WHEN** a kind declares `id_names: { language: python }`
- **THEN** the loader rejects the profile naming the kind and `separator`

#### Scenario: Null id_names reads as absent
- **WHEN** a kind declares `id_names: null`
- **THEN** the profile loads, and none of the kind's nodes is resolved

#### Scenario: Non-string, non-mapping id_names rejected
- **WHEN** a kind declares `id_names: [file]`
- **THEN** the loader rejects the profile naming the kind and `id_names`

### Requirement: A refused profile names every independent fault
When the loader refuses a profile, the refusal SHALL name every fault it can detect
independently, not only the first, so a profile with three mistakes is fixed in one
edit. Each fault SHALL be on its own line, naming where it is. A fault that stops further
checking (unreadable file, invalid YAML, an `extends` cycle, a top level that is not a
mapping) SHALL be reported alone. A fault that depends on another (an entry naming a kind
that a broken `node_kinds` failed to declare) MAY be left out. Within one validation
entry, a fault in its `message` or pathway binding SHALL NOT keep its body from being
checked: a `CONSTRAINT` with an unknown placeholder and a retired operator names both. The exit code SHALL stay 2.
The fuse-profile loader SHALL behave the same way.

Verified by: `cargo test -p latt --locked --test rejected_profiles`

#### Scenario: Three independent faults are reported together
- **WHEN** a profile declares `VACANCY` with `severity: loud`, `COVERAGE` with a mistyped
  `target_kinds` key, and `UNTRACED` with `severity: nope`
- **THEN** the loader refuses the profile, exit 2, and the refusal names all three entries

#### Scenario: Two faults in one entry are reported together
- **WHEN** a `CONSTRAINT` entry's `message` names an unknown placeholder and its `when`
  block uses the retired operator `not`
- **THEN** the loader refuses the profile, exit 2, and the refusal names both

#### Scenario: A blocking fault is reported alone
- **WHEN** a profile is not valid YAML
- **THEN** the loader refuses it with the YAML error only

