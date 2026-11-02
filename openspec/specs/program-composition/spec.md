# program-composition Specification

## Purpose

Multi-source graph composition: combine independent single-source lattice runs
into a unified graph, qualify IDs and kinds by source, resolve cross-source
edges, and report findings. Implemented as `latt fuse` (core subcommand).
## Requirements
### Requirement: Read a fuse manifest
`latt fuse` SHALL accept a fuse manifest (YAML) declaring `manifest_version`,
`name`, `version`, `fuse_profile` (path to the fuse profile), and an ordered
`sources` list. Each source SHALL declare `name` (unique, must not contain `:`),
`profile`, `adapter`, and `target`. All paths SHALL be relative to the manifest
file's directory. An `adapter` value spelled exactly as a compiled-in native
adapter name (`md`, `toml`, `prose-xref`) SHALL select that native adapter; any
other `adapter` value is a program path.

`manifest_version` SHALL be an X.Y.Z version whose major the core supports (the
versioning capability's wire version axes); a minor or patch above the core's is read. The manifest and each source
SHALL declare no key beyond those named here.

The command SHALL exit 2 if the manifest is missing, malformed, declares no
sources, contains duplicate source names, has a source name containing `:`,
declares an unknown key, or has a `manifest_version` that is not X.Y.Z or whose
major is unsupported.

Verified by: `cargo test -p latt --test fuse -- manifest`

#### Scenario: Valid manifest loads
- **WHEN** the command reads a manifest with two sources, each with name,
  profile, adapter, and target
- **THEN** it loads both sources in declared order

#### Scenario: Native adapter named in a manifest
- **WHEN** a source declares `adapter: md` in a build with native adapters
- **THEN** the source runs the md native adapter, and `adapter: ./md` runs the
  program `md` beside the manifest

#### Scenario: Unsupported or malformed manifest version
- **WHEN** a manifest declares `manifest_version: "2.0.0"` or `"1"`
- **THEN** the command exits 2 naming the version; `"1.4.2"` loads

#### Scenario: Unknown manifest or source key
- **WHEN** the manifest, or one of its sources, declares a key the format does not have
- **THEN** the command exits 2 naming the key

#### Scenario: Duplicate source names
- **WHEN** a manifest declares two sources with the same name
- **THEN** the command exits 2 with an error naming the duplicate

#### Scenario: Source name with colon
- **WHEN** a source name contains `:`
- **THEN** the command exits 2 — colons are reserved for source qualification

### Requirement: Read a fuse profile
The command SHALL load the fuse profile referenced by the manifest. The fuse
profile SHALL declare `edge_kinds` with allowed endpoint pairs using
source-qualified kind names (`source:kind` with colon separator), and optionally
`validations` for cross-source checks. The `validations` list MAY include
`SUPPRESS` entries on the same terms as a source profile (see the
profile-schema capability).

The fuse profile SHALL NOT declare `node_kinds` — those are owned by source
profiles. It SHALL NOT declare `annotations` either: it is not read from a file of its
own, so there is no directory to resolve the notes path against. Notes belong to source
profiles.

A `validations` entry naming a retired finding code SHALL be refused naming the
replacement, as in a source profile, and an entry carrying `message` SHALL be refused:
the composed run renders no declared message, and a key that did nothing would be
silence. A fuse profile with several such faults SHALL be refused once, naming every
fault on its own line.

Verified by: `cargo test -p latt --test fuse -- rejects_bad_profiles` and
`cargo test -p latt --test fuse -- notes_a_fuse_profile_cannot_declare_annotations`

#### Scenario: Fuse profile with cross-source edge kinds
- **WHEN** the fuse profile declares `derives` with allowed pair
  `[compliance:clause, product:requirement]`
- **THEN** the command uses this for cross-source edge resolution

#### Scenario: Fuse profile declares node_kinds
- **WHEN** the fuse profile contains a `node_kinds` section
- **THEN** the command exits 2 — node kinds belong to source profiles

#### Scenario: Fuse profile with a SUPPRESS entry loads
- **WHEN** the fuse profile declares `- SUPPRESS: {code: VACANCY}` in `validations`
- **THEN** the fuse profile loads and the entry applies to the composed run

#### Scenario: Fuse profile declares annotations
- **WHEN** the fuse profile contains `annotations: notes.yaml`
- **THEN** the command exits 2 naming `annotations` — notes belong to source profiles

#### Scenario: Fuse profile names a retired code
- **WHEN** the fuse profile declares `- DANGLING_REF: {severity: error}`
- **THEN** the command exits 2 naming `VACANCY` as the replacement

#### Scenario: Fuse profile declares a message
- **WHEN** the fuse profile declares `- VACANCY: {message: "see {message}"}`
- **THEN** the command exits 2 saying `message` is not supported in a fuse profile

### Requirement: Fuse profile endpoints name declared kinds
Every `source:kind` endpoint of a fuse profile's `edge_kinds` SHALL name a source the
manifest lists and a kind that source's profile declares. A fuse profile naming any other
SHALL be refused before a source runs, as a `FUSE_ERROR` with exit 2, naming each such
endpoint with its edge kind and listing the manifest's sources, or that source's kinds.
Such a pair matches nothing; composing it would give its kind an empty declaration that
accepts any ID, and a misspelt kind would read as a join with nothing to join. A source
whose profile does not load is reported as that source's failure instead.

Verified by: `cargo test --test fuse a_fuse_endpoint_naming_no_source_or_kind`

#### Scenario: An endpoint names a source the manifest lacks
- **WHEN** the manifest lists only `reqs` and an `allowed` pair names `ghost:issue`
- **THEN** `latt fuse` exits 2 with `fuse profile edge kind 'cites' names
  'ghost:issue', but the manifest has no source 'ghost' (sources: reqs)`

#### Scenario: An endpoint names a kind its source does not declare
- **WHEN** source `z` declares only `item` and an `allowed` pair names `z:itme`
- **THEN** `latt fuse` exits 2 naming `'z:itme'` and `(kinds: item)`

### Requirement: Run each source through latt trace
The command SHALL run `latt trace --format json --profile <profile> --adapter
<adapter> --target <target>` for each source in manifest order. It SHALL parse
the JSON trace payload from each run.

A source whose trace run exits 2, fails to execute, or emits unparseable output
SHALL be recorded as `SOURCE_FAILURE`. The command SHALL continue running
remaining sources. After all sources run, if any source failed, the command SHALL
report the healthy sources' findings alongside SOURCE_FAILURE findings and exit 2
without assembling the graph.

A source whose trace run exits 0 or 1 SHALL contribute its trace payload to the
merge.

Verified by: `cargo test -p latt --test fuse -- failed_trace`

#### Scenario: All sources succeed
- **WHEN** all sources exit 0 or 1
- **THEN** the command proceeds to merge

#### Scenario: One source fails
- **WHEN** one of two sources exits 2 and the other exits 0
- **THEN** the command reports the healthy source's findings and
  SOURCE_FAILURE for the failed source, and exits 2

### Requirement: Source-qualified merge with colon separator
The command SHALL merge trace payloads from all successful sources in declared
order. Node IDs SHALL be qualified as `source:raw_id` and kinds as
`source:raw_kind`, using the colon separator. Pathways SHALL be qualified as
`source:pathway_name`. Edge targets SHALL be initially qualified within the same
source.

If a raw node ID appears in multiple sources, the command SHALL emit a
`CROSS_SOURCE_DUPLICATE_ID` finding at error severity (profile-overridable)
naming the conflicting sources.

Merge order SHALL affect output ordering only, not which declaration wins.

Verified by: `cargo test -p latt --test fuse -- assembly_qualifies`

#### Scenario: IDs and kinds are source-qualified with colon
- **WHEN** source `product` emits node `REQ-1` of kind `requirement`
- **THEN** the composed graph contains node `product:REQ-1` of kind
  `product:requirement`

#### Scenario: Duplicate raw ID across sources
- **WHEN** sources `a` and `b` both emit a node with raw ID `T-11`
- **THEN** the command emits `CROSS_SOURCE_DUPLICATE_ID` naming both sources

### Requirement: Cross-source edge resolution via allowed pairings
For each edge whose kind is declared in the fuse profile's `edge_kinds`, the
command SHALL resolve the target by finding a node whose source-qualified kind
matches an allowed target kind and whose raw ID matches the edge's raw target.

If the target matches nodes in multiple allowed kinds, the command SHALL emit
`AMBIGUOUS_CROSS_REF` at error severity (profile-overridable).

If the target matches no node in any allowed target kind, the command SHALL emit
`VACANCY` at the fuse profile's declared severity for that code.

Resolved edges SHALL carry `target_kind` and `target_source` in the output.

A source's own trace run cannot see the other sources, so it reports an edge into
another source as a `VACANCY` for a target that does not exist — at hint severity
when the source profile marks the edge kind `cross_source`, at its declared
severity otherwise. Once the command has resolved such an edge, or found no allowed
target for it, the source's `VACANCY`
for it is the command's to withdraw: the finding with that source and the message
the validator gives for that edge's raw source, raw target and kind. When the
command resolves the edge, it SHALL withdraw the source's `VACANCY` and emit nothing.
When the edge's kind is declared in the fuse profile and no allowed target matches,
the command SHALL withdraw the source's `VACANCY` and emit its own, so an unresolved
edge is reported once. A source `VACANCY` for an edge whose kind the fuse profile
does not declare SHALL remain: the command never judged it.

Verified by: `cargo test -p latt --test fuse -- duplicates_ambiguity`, and
`cargo test -p latt --test fuse -- a_source_vacancy_is_withdrawn`

#### Scenario: Unambiguous resolution
- **WHEN** `traces_to` allows `[markdown:requirement, toml:item]` and a
  markdown edge targets `alpha/one` which exists only in source `toml`
- **THEN** the edge resolves to `toml:alpha/one` of kind `toml:item`

#### Scenario: Ambiguous resolution
- **WHEN** an edge kind allows two target kinds and the raw target ID exists
  in both
- **THEN** the command emits `AMBIGUOUS_CROSS_REF`

#### Scenario: Unresolvable target
- **WHEN** an edge targets an ID that exists in no allowed target kind
- **THEN** the command emits `VACANCY`

#### Scenario: A resolved edge's source VACANCY is withdrawn
- **WHEN** source `z` reports `VACANCY` for its edge `Z`→`A` because `A` is not
  in `z`, and the command resolves that edge to `a:A`
- **THEN** the fuse output carries no `VACANCY` for that edge

#### Scenario: An unresolved declared edge is reported once
- **WHEN** source `z` reports `VACANCY` for its edge `Z`→`B`, the fuse profile
  declares the edge's kind, and `B` exists in no allowed target kind
- **THEN** the fuse output carries exactly one `VACANCY` for that edge, the
  command's own, attributed to source `z`

#### Scenario: An undeclared edge keeps its source VACANCY
- **WHEN** source `z` reports `VACANCY` for an edge whose kind the fuse profile
  does not declare and whose target is not in `z`
- **THEN** the source's `VACANCY` remains in the fuse output and the command
  emits none of its own

### Requirement: Standard validators on the composed graph
The command SHALL construct a temporary profile from the composed graph's node
kinds and the fuse profile's edge kinds and validations, then run the standard
`validate` pass on the composed graph. Findings from standard validators SHALL
carry source attribution derived from the node's provenance.
For node kinds emitted by source traces, the temporary profile SHALL accept any
composed ID (assembly wrote the qualifier), and the command SHALL check the raw ID
against the source profile's own `id_pattern`, emitting `ID_FORMAT` as the standard
validator would. The source pattern is matched as compiled, never spliced into a
qualified pattern as text: a nested anchor or an escaped `$` would change meaning.
Node kinds added only from fuse edge endpoint declarations SHALL accept any ID.

The composed run owns the orphan findings. For a node kind emitted by a source
trace, the temporary profile SHALL carry that source profile's `orphan_ok`, so the
standard `UNREFERENCED` and `UNTRACED` validators run over the composed graph, where
an edge the command resolved across sources counts as incoming on its target. An
edge the command did not resolve SHALL still count as the source's own run counted
it: outgoing from its source node, and incoming on the same-source node its raw
target names when one exists. A kind named only in a fuse edge declaration
SHALL be `orphan_ok`. Whether an edge could supply a direction (validation's "Orphan
checks follow the allowed pairs") SHALL be read from every source profile's own
`allowed` pairs, qualified, and the fuse profile's, never from the edges that arrived:
a kind that received no edge is the one the finding is for. Every `UNREFERENCED` and `UNTRACED` finding a source
contributed SHALL be withdrawn at composition: the composed graph's answer replaces
it. A composed orphan finding's severity SHALL be the fuse profile's override for
that code when it declares one, else the source profile's resolved severity for it;
a composed orphan finding SHALL be suppressed when its node's source profile would
suppress that code for that node in the source's own run (see the suppression
requirement), and when its source counterpart (same source, node and code) arrived
suppressed. On a manifest whose
cross-source edges resolve nothing and whose fuse pairs permit no direction a source's
own pairs do not, the composed orphan findings SHALL equal the source runs' in count,
node and severity.

COVERAGE validations declared in the fuse profile SHALL route through the
standard validator on the composed profile. Before inclusion, the command SHALL
validate each COVERAGE entry's `edge_kind` against the fuse profile's declared
`edge_kinds`; an undeclared kind produces a CONFIG_ERROR finding and the entry
is excluded. Unresolved edges do not count as covering their target. COVERAGE
entries MAY include `where:` and `source_where:` conditions, consistent with standard profiles.

`--strict` SHALL promote warnings to errors after all findings are collected,
consistent with single-source behavior.

Verified by: `cargo test -p latt --test fuse -- standard_constraints duplicates_ambiguity coverage_undeclared coverage_repeated coverage_unknown coverage_where composed_ids source_pattern orphan`

#### Scenario: Standard validators produce findings on composed graph
- **WHEN** the fuse profile declares a CONSTRAINT validation and a composed
  node violates it
- **THEN** the command emits the CONSTRAINT finding with source attribution

#### Scenario: Source findings composition makes false are withdrawn
- **WHEN** source `teams` lists every node of source `people` through an edge kind
  the fuse profile declares, and the people source's own run reported each person
  `UNREFERENCED`
- **THEN** the fuse output carries no `UNREFERENCED` for any person, and carries
  the people source's `UNTRACED` per person as the composed run's own finding,
  attributed to `people`

#### Scenario: A person no team lists stays unreferenced
- **WHEN** one node of source `people` is the target of no edge after resolution
  and the people profile does not mark its kind `orphan_ok`
- **THEN** the fuse output carries exactly one `UNREFERENCED` for it, attributed to
  `people`, at the fuse profile's severity for that code when declared, else the
  people profile's

#### Scenario: An unresolved edge still counts as the source counted it
- **WHEN** a source node's only outgoing edge is unresolved after composition, and
  its raw target names a node of the same source that the fuse profile's pairing
  refuses
- **THEN** the fuse output carries no `UNTRACED` for the source node and no
  `UNREFERENCED` for the same-source target, beside the command's `VACANCY` for
  the edge

#### Scenario: Composed IDs retain source pattern validation
- **WHEN** source `product` declares ID pattern `REQ-[0-9]+` and emits a node
  whose composed ID is `product:malformed`
- **THEN** the command emits an ID_FORMAT finding for that composed node
- **AND** it does not emit ID_FORMAT for a composed ID such as `product:REQ-001`

#### Scenario: A source pattern keeps its meaning under composition
- **WHEN** a source profile's `id_pattern` is `(?:^A$)` or `A\$`, and the source
  emits the node that pattern accepts standalone
- **THEN** the composed graph carries no `ID_FORMAT` for it and the run completes:
  the raw ID is checked against the pattern as the source compiled it

#### Scenario: Coverage checks resolved edges only
- **WHEN** a COVERAGE validation targets a kind and an edge to that kind is
  unresolved
- **THEN** the unresolved edge does not count as covering its target

#### Scenario: Undeclared COVERAGE edge_kind produces CONFIG_ERROR
- **WHEN** a COVERAGE validation names an edge_kind not declared in the fuse
  profile's `edge_kinds`
- **THEN** the command emits a CONFIG_ERROR finding and excludes that entry
  from the composed profile

#### Scenario: COVERAGE_UNKNOWN fires when sources lack attribution
- **WHEN** source nodes that could carry an edge have no outgoing edge of the
  configured kind in the composed graph (unresolved edges count neither toward
  coverage nor toward attribution)
- **THEN** the command emits a COVERAGE_UNKNOWN hint and COVERAGE findings
  carry `state: "unknown"`

#### Scenario: COVERAGE where condition filters target nodes
- **WHEN** a COVERAGE validation includes a `where:` condition
- **THEN** only target nodes matching the condition are checked for coverage

#### Scenario: COVERAGE source_where condition excludes evidence
- **WHEN** a COVERAGE validation's `source_where:` excludes the only node linking to a target
- **THEN** that target is reported as uncovered

#### Scenario: Repeated COVERAGE entries keep their own severity
- **WHEN** two COVERAGE entries target the same kind with different severities
- **THEN** both checks run and each entry's findings carry that entry's severity, per
  the standard rule for entries that declare their own check

#### Scenario: Strict promotes warnings after collection
- **WHEN** `--strict` is passed and the composed graph has warning-severity
  findings
- **THEN** those findings are promoted to error severity

### Requirement: Suppression in program composition
Suppression applies at two levels, each after its own finding collection.

A source profile's `SUPPRESS` entries apply within that source's own `lattice
trace` run, per the validation capability, so the findings a source
contributes to the merge arrive already marked. The command SHALL preserve a
source finding's `suppressed` flag through the merge; it SHALL NOT unsuppress
a finding a source profile suppressed, and SHALL NOT re-derive that decision
from the fuse profile. A source `UNREFERENCED` or `UNTRACED` is withdrawn at
composition and replaced by the composed run's own (see the standard-validators
requirement). A composed `UNREFERENCED` or `UNTRACED` on a node of source `S` SHALL be
suppressed when `S`'s profile declares a `SUPPRESS` entry for that code that would
suppress it in `S`'s own run: an entry without `node_ids` covers every node of `S`, an
entry with `node_ids` matches the node's raw ID (without the `S:` qualifier), and the
first entry for a code decides. This holds whether or not `S`'s run reported the node
as an orphan: composition can make an orphan the source run did not see. A replacement
SHALL also keep the suppression of a source finding that arrived suppressed; the
command never unsuppresses one.

The fuse profile's `SUPPRESS` entries apply to the findings the command itself
collects — cross-source resolution findings (`CROSS_SOURCE_DUPLICATE_ID`,
`AMBIGUOUS_CROSS_REF`, `VACANCY`) and the standard-validator findings on the
composed graph, which include `UNREFERENCED` and `UNTRACED`. A fuse-level
`node_ids` entry SHALL match against composed IDs
(`source:raw_id`), because that is the `node_id` the findings carry.
Suppression runs after `--strict` promotion and before the exit-code decision,
consistent with single-source behavior. `SUPPRESS_UNUSED` for a fuse-profile
entry is emitted against the fuse profile and names it as provenance.

A suppressed finding SHALL NOT count toward the command's exit code 1, and
SHALL render per the `output` capability: present in `json` with
`suppressed: true`, omitted from `plain` and `rich`.

Verified by: `cargo test -p latt --test fuse -- suppress`

#### Scenario: Source-level suppression survives the merge
- **WHEN** source `product`'s profile suppresses `PARSE_ERROR` and its trace run
  contributes a `PARSE_ERROR` finding marked `suppressed: true`
- **THEN** the fuse output carries that finding with `suppressed: true` and it
  does not affect the fuse exit code

#### Scenario: A source SUPPRESS reaches an orphan only composition made
- **WHEN** source `langs`'s profile declares `SUPPRESS: {code: UNREFERENCED}`, node
  `ada`'s only incoming edge in the `langs` run is its own edge to raw target `ada`, so
  that run reports no `UNREFERENCED` for it, and the fuse profile resolves that edge to
  `grammars:ada`
- **THEN** the fuse output carries one `UNREFERENCED` for `langs:ada`, marked
  `suppressed: true`, and it does not affect the fuse exit code

#### Scenario: A source SUPPRESS with node_ids matches raw IDs
- **WHEN** source `langs`'s profile declares
  `SUPPRESS: {code: UNREFERENCED, node_ids: [ada]}`, and the composed graph has
  `UNREFERENCED` for `langs:ada` and `langs:bash`, neither reported by the `langs` run
- **THEN** the `langs:ada` finding is suppressed and `langs:bash` is not

#### Scenario: A source-suppressed orphan finding stays suppressed when recomputed
- **WHEN** source `z`'s profile suppresses `UNTRACED`, its trace run contributes
  `UNTRACED` for `Z` marked `suppressed: true`, and `Z` has no outgoing edge in
  the composed graph
- **THEN** the fuse output carries one `UNTRACED` for `z:Z`, marked
  `suppressed: true`, and it does not affect the fuse exit code

#### Scenario: Fuse-level suppression of a cross-source finding
- **WHEN** the fuse profile declares `SUPPRESS: {code: VACANCY}` and
  cross-source resolution emits 59 `VACANCY` findings
- **THEN** all 59 are suppressed: absent from plain output, present in JSON
  with `suppressed: true`, excluded from the exit code

#### Scenario: Fuse-level node_ids match composed IDs
- **WHEN** the fuse profile declares
  `SUPPRESS: {code: VACANCY, node_ids: [compliance:C-7]}` and `VACANCY`
  findings exist for `compliance:C-7` and `compliance:C-8`
- **THEN** the `compliance:C-7` finding is suppressed and `compliance:C-8` is not

#### Scenario: Fuse-level unmatched suppress reports against the fuse profile
- **WHEN** the fuse profile declares `SUPPRESS: {code: AMBIGUOUS_CROSS_REF}`
  and no such finding is emitted
- **THEN** the command emits `SUPPRESS_UNUSED` at info naming the fuse profile
  as provenance

#### Scenario: Fuse profile does not unsuppress a source finding
- **WHEN** a source profile suppresses `PARSE_ERROR` and the fuse profile
  declares no `SUPPRESS` entries
- **THEN** the source's `PARSE_ERROR` findings remain suppressed in the fuse
  output

#### Scenario: Fuse-level suppress does not reach a source-owned finding
- **WHEN** a source contributes an unsuppressed `PARSE_ERROR` and the fuse profile
  declares `SUPPRESS: {code: PARSE_ERROR}`
- **THEN** the source finding keeps gating, and the command emits `SUPPRESS_UNUSED`
  for the fuse entry

### Requirement: Pathway preservation
The command SHALL preserve pathways from each source's trace payload, qualified
as `source:pathway_name`. Pathways SHALL appear in the fuse output.

Verified by: `cargo test -p latt --test fuse -- assembly_qualifies`

#### Scenario: Pathways are source-qualified
- **WHEN** source `toml` has a pathway named `stage`
- **THEN** the fuse output contains pathway `toml:stage`

### Requirement: Tri-format output
The command SHALL output the fuse report through `output_result` in all three
formats (plain, json, rich). The JSON output SHALL include `header` (with `name`,
`version`, `manifest_version`, `fuse_version`), `nodes`, `edges`, `pathways`, and
`findings`. `fuse_version` is the report envelope's version, an integer string
(current value: the versioning capability's wire version axes), present even when the composition could not start; it changes when the top-level keys or the shape of a node, edge,
pathway or finding change.
Each finding SHALL carry source attribution when applicable.

Verified by: `cargo test -p latt --test fuse -- cli_clean_warning`

#### Scenario: JSON output contains composed graph
- **WHEN** the command runs with `--format json` against two sources
- **THEN** stdout is valid JSON with `header`, `nodes`, `edges`, `pathways`,
  and `findings` fields

#### Scenario: Plain output lists nodes and findings
- **WHEN** the command runs with `--format plain`
- **THEN** the output contains a header line, node listings, edge listings,
  and finding lines

### Requirement: Fuse exit codes
The command SHALL use the same three-valued exit codes as lattice:
- **0** — no error-severity findings in any phase.
- **1** — error-severity findings in source or cross-source validation.
- **2** — could not run: bad manifest, bad fuse profile, source adapter failure,
  unparseable trace output, or a source profile that cannot be loaded.

Exit 2 SHALL never be collapsed into 1.

Verified by: `cargo test -p latt --test fuse -- cli_clean_warning source_profile`

#### Scenario: Clean run
- **WHEN** all sources pass and no cross-source findings at error severity
- **THEN** the command exits 0

#### Scenario: Cross-source finding at error severity
- **WHEN** the composed graph has a `CROSS_SOURCE_DUPLICATE_ID`
- **THEN** the command exits 1

#### Scenario: Source failure
- **WHEN** one source's adapter exits 2
- **THEN** the command exits 2

#### Scenario: Source profile failure
- **WHEN** a source's profile cannot be loaded at composition
- **THEN** the report's exit code is 2

### Requirement: Member findings keep their notes
A finding read from a source's trace SHALL keep the `note` that source's own profile
attached to it, unchanged, in the fuse report's JSON, plain and rich output, on the same
terms as the output capability's noted findings. Fuse SHALL NOT attach notes of its own.

Verified by: `cargo test -p latt --locked --test fuse notes`.

#### Scenario: A source's note survives composition
- **WHEN** source `product` carries a `VACANCY` finding on `REQ-2` with a note
- **THEN** the fuse JSON finding for `product:REQ-2` carries the same `note` object

### Requirement: A source profile that cannot load is a source failure
Composition reads each source's profile for the ID patterns it qualifies onto composed
node kinds. A source profile that cannot be loaded SHALL be recorded as a
`SOURCE_FAILURE` finding at error severity naming the source and the profile path, on
the same terms as a source whose trace run failed: the report SHALL carry the other
sources' own findings and every `SOURCE_FAILURE`, the composition SHALL NOT run, and
the command SHALL exit 2. Composing without the pattern set would validate that
source's composed IDs against nothing and report the result as clean.

A trace handed to composition SHALL have passed the trace checks; the type that
carries it SHALL NOT be constructible by deserializing it directly, so an unchecked
severity or note reaches composition as an error, never as a panic.

Verified by: `cargo test -p latt --test fuse -- source_profile`

#### Scenario: Unloadable source profile
- **WHEN** two sources' traces parse and the first source's profile path is unreadable
- **THEN** the report could not run, carries a `SOURCE_FAILURE` naming the first source
  and its profile path and the second source's own findings, and its exit code is 2

#### Scenario: Unchecked trace is refused
- **WHEN** a trace carries a finding severity `fatal`
- **THEN** it is refused when it is parsed, naming the source, and composition is never reached

### Requirement: A source trace line outside 32 bits is a source failure
A source trace whose finding `line` or location `line` is negative or
above 4294967295 SHALL be unparseable output: `SOURCE_FAILURE` for that source, naming
the source and the line as written, and exit 2 under "Run each source through latt
trace". The fuse reader SHALL hold the line as the core's unsigned 32-bit provenance
line, never accept a negative one, and never clamp. `docs/fuse.md` SHALL state the
bound.

Verified by: `cargo test -p latt --locked --test fuse trace_line`

#### Scenario: A negative source trace line fails the source
- **WHEN** a source's trace JSON carries a finding whose `line` is -1
- **THEN** fuse reports `SOURCE_FAILURE` for that source naming the line, keeps the healthy sources' findings, and exits 2

#### Scenario: A source trace line beyond 32 bits fails the source
- **WHEN** a source's trace JSON carries a location whose `line` is 4294967296
- **THEN** fuse reports `SOURCE_FAILURE` for that source naming the line, and exits 2

#### Scenario: The largest 32-bit source trace line is kept
- **WHEN** a source's trace JSON carries a finding whose `line` is 4294967295
- **THEN** the composed finding carries line 4294967295

