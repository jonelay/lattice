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

A manifest or fuse profile that cannot be read or is not valid YAML SHALL be one
`FUSE_ERROR` whose message is one line, naming the file and then each cause once. Its
provenance SHALL name that file, the manifest as given, and for a YAML error the line
the parser reports.

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

#### Scenario: Unreadable or malformed manifest
- **WHEN** `--manifest` names a directory, or a file holding `manifest_version: [`
- **THEN** the command exits 2 with one `FUSE_ERROR` on one line, carrying `Is a
  directory` or the parser's error once; the malformed file's finding is at line 2

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

Each source's trace run SHALL be bounded by its source profile's adapter timeout
(`adapter.timeout_seconds`, or its default) plus 30 seconds for the trace's own work; a
run still going at the bound SHALL be killed.

A source whose trace run exits 2, fails to execute, runs past its bound, or emits
unparseable output SHALL be recorded as `SOURCE_FAILURE`. The command SHALL continue running
remaining sources. After all sources run, if any source failed, the command SHALL
report the healthy sources' findings alongside SOURCE_FAILURE findings and exit 2
without assembling the graph.

A `SOURCE_FAILURE` message SHALL be one line: how the run ended (`trace exited 2`, or
the signal that stopped it), then the run's stderr with its non-blank lines joined by
`; `.

A source whose trace run exits 0 or 1 SHALL contribute its trace payload to the
merge.

Verified by: `cargo test -p latt --test fuse -- failed_trace`, and
`cargo test -p latt-core --lib bounded_source_runs`

#### Scenario: All sources succeed
- **WHEN** all sources exit 0 or 1
- **THEN** the command proceeds to merge

#### Scenario: One source fails
- **WHEN** one of two sources exits 2 and the other exits 0
- **THEN** the command reports the healthy source's findings and
  SOURCE_FAILURE for the failed source, and exits 2

#### Scenario: A source run does not finish
- **WHEN** a source profile declares `adapter.timeout_seconds: 1` and its trace run does
  not exit
- **THEN** the run is killed at its bound and the source's `SOURCE_FAILURE` reads
  `trace exceeded <N> s and was stopped`

#### Scenario: A failed run's stderr spans lines
- **WHEN** a source's adapter writes `first line` and `second line` to stderr and exits 2
- **THEN** its `SOURCE_FAILURE` message starts `trace exited 2: ` and holds
  `first line; second line`, on one line

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
A node whose `ID_FORMAT` its source already reported SHALL NOT be checked again: the
source's finding is the one report, unchanged but for qualification and attribution.
Node kinds added only from fuse edge endpoint declarations SHALL accept any ID.

The composed run owns `EMPTY_KIND`. Every `EMPTY_KIND` a source contributed SHALL be
withdrawn at composition, and the command SHALL emit one per kind with no instance
in the whole fused population: each node kind any source profile declares, qualified,
with no composed node, and each edge kind a source profile or the fuse profile
declares, by its shared name, with no composed edge. Every composed edge counts,
resolved or not. A node kind's finding SHALL take its severity and suppression as a
composed orphan finding does (below); an edge kind's SHALL take the fuse profile's
alone, since the sources declaring it may disagree.

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

Verified by: `cargo test -p latt --test fuse -- standard_constraints duplicates_ambiguity coverage_undeclared coverage_repeated coverage_unknown coverage_where composed_ids source_pattern orphan empty kinds id_format composed_id_check`

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

#### Scenario: A malformed ID is reported once
- **WHEN** source `product` reports `ID_FORMAT` on its node `malformed`
- **THEN** the fuse output carries exactly that finding for `product:malformed`, with
  its note, and no second `ID_FORMAT` for the node

#### Scenario: A kind whose every edge dangles is not empty
- **WHEN** every `verifies` edge of source `tests` names a target no source declares
- **THEN** the fuse output lists those edges and carries no `EMPTY_KIND` for
  `verifies`, beside the `VACANCY` per edge

#### Scenario: Each empty kind is reported once
- **WHEN** source `product` declares node kinds `requirement` and `hazard` and emits
  no hazard, and edge kind `controls` has no edge in any source
- **THEN** the fuse output carries one `EMPTY_KIND` for node kind `product:hazard`
  and one for edge kind `controls`, and none naming the raw kind `hazard`

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
command never unsuppresses one. A composed `EMPTY_KIND` for node kind `S:kind` SHALL be
suppressed when `S`'s profile declares a `SUPPRESS` entry for `EMPTY_KIND` without
`node_ids`.
A source's `SUPPRESS_UNUSED` for an `UNREFERENCED` or `UNTRACED` entry was judged
against the orphan findings composition withdrew, so the command SHALL judge that entry
again against the composed run's findings of its code on `S`'s nodes, matching raw IDs:
the source finding SHALL be withdrawn when the entry matches one, and otherwise SHALL
name only the `node_ids` still unmatched. An entry `S`'s run used that matches no
composed finding SHALL be reported as a `SUPPRESS_UNUSED` attributed to `S`, with the
severity and suppression `S`'s profile gives that code.

The fuse profile's `SUPPRESS` entries apply to the findings the command itself
collects — cross-source resolution findings (`CROSS_SOURCE_DUPLICATE_ID`,
`AMBIGUOUS_CROSS_REF`, `VACANCY`) and the standard-validator findings on the
composed graph, which include `UNREFERENCED` and `UNTRACED`. A fuse-level
`node_ids` entry SHALL match against composed IDs
(`source:raw_id`), because that is the `node_id` the findings carry. A fuse-profile
entry SHALL only add suppression: an entry for a finding's code that does not match it
(a `node_ids` entry, for a finding with no node) SHALL leave the suppression its source
decided.
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

#### Scenario: A source entry the composed graph uses is not reported unused
- **WHEN** source `z`'s profile declares `SUPPRESS: {code: UNREFERENCED}`, its run
  reports no orphan and so reports that entry as `SUPPRESS_UNUSED`, and the composed
  graph has `UNREFERENCED` for `z:X`
- **THEN** the `z:X` finding is suppressed and the fuse output carries no
  `SUPPRESS_UNUSED` for the entry

#### Scenario: A source entry the composed graph leaves unmatched is reported unused
- **WHEN** source `a`'s profile declares `SUPPRESS: {code: UNREFERENCED, node_ids: [A]}`,
  its run suppresses `UNREFERENCED` for `A`, and an edge from source `z` resolves to `a:A`
- **THEN** the fuse output carries no `UNREFERENCED` for `a:A` and one `SUPPRESS_UNUSED`
  attributed to `a` naming `node_ids ["A"]`

#### Scenario: A source's SUPPRESS_UNUSED message template survives reconciliation
- **WHEN** the source profile gives `SUPPRESS_UNUSED` a `message:` template, and the
  composed graph uses that source's entry wholly, in part, or not at all where the
  source run used it
- **THEN** the source's finding is withdrawn, rewritten or added exactly as without a
  template, and every one the fuse output carries is rendered through the template at
  its own severity

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
pathway or finding change. A finding may gain an optional key (as `stage`) without
changing `fuse_version`; removing a key or changing a key's type or meaning does.
Each finding SHALL carry source attribution when applicable.
Each `SOURCE_FAILURE` and `FUSE_ERROR` finding that fuse itself raises SHALL carry `stage`
in `json`, naming where the run failed: `manifest`, `fuse_profile` (the fuse profile, or
the composed profile refusing one of its entries), `source_run` (a source's run could
not start, exited with a code other than 0 or 1, or was killed by a signal, a source
profile that does not load inside that run included; with no `source`, fuse could not
find its own executable), `timeout` (stopped past its bound), `source_output` (the trace
did not parse), `source_profile` (fuse's own load of a source profile after the runs
failed) or `composition` (a composition check that the earlier stages had passed). No
other finding SHALL carry it, and its `message` SHALL be unchanged by it. A finding with
one of those codes that a source's adapter wrote is carried as the source wrote it, with
no `stage`.

Verified by: `cargo test -p latt --test fuse -- cli_clean_warning stage source_profile_that`
and `cargo test -p latt-core --lib bounded_source_runs`

#### Scenario: JSON output contains composed graph
- **WHEN** the command runs with `--format json` against two sources
- **THEN** stdout is valid JSON with `header`, `nodes`, `edges`, `pathways`,
  and `findings` fields

#### Scenario: Plain output lists nodes and findings
- **WHEN** the command runs with `--format plain`
- **THEN** the output contains a header line, node listings, edge listings,
  and finding lines

#### Scenario: Failure findings carry a stage
- **WHEN** the manifest is missing, or a source's `latt trace` exits 2, and the command
  runs with `--format json`
- **THEN** the `FUSE_ERROR` finding carries `"stage": "manifest"`, or the
  `SOURCE_FAILURE` finding carries `"stage": "source_run"`, and a run that could run
  has no finding with `stage`

#### Scenario: The stage tells the manifest, the fuse profile and a source profile apart
- **WHEN** the manifest is not valid YAML, the fuse profile is not valid YAML, the
  composed profile refuses a fuse profile entry, a source profile breaks between its
  trace run and fuse's own load of it, or a source's run passes its bound
- **THEN** the finding's `stage` is `manifest`, `fuse_profile`, `fuse_profile`,
  `source_profile` or `timeout` respectively

### Requirement: Fuse exit codes
The command SHALL use the same three-valued exit codes as lattice:
- **0** — no error-severity findings in any phase.
- **1** — error-severity findings in source or cross-source validation.
- **2** — could not run: bad manifest, bad fuse profile, source adapter failure,
  unparseable trace output, a source profile that cannot be loaded, or a panic in fuse itself.

Exit 2 SHALL never be collapsed into 1.

At exit 2 the command SHALL also name each `SOURCE_FAILURE` and `FUSE_ERROR` on stderr,
after the report on stdout, which it SHALL leave unchanged: in `plain` and `rich` one line
per failure, `fuse: source '<name>' failed at <stage>: <message>. Next: <action>` for a
source and `fuse: <stage> failed: <message>. Next: <action>` for the composition's own,
one trailing `.` of the message dropped; in `json` one object in the `validate` findings
shape, with no `stage` key, whose `message` holds the same text without the `fuse: `
prefix. The action SHALL follow the stage: for `timeout`, raise the source's
`timeout_seconds` or run its trace alone; for `source_run`, `source_output` and
`source_profile`, run the source's `latt trace` and read its stderr, except that a
`source_run` with no source (fuse could not find its own executable) SHALL say to check
or reinstall the `latt` binary; for `manifest` and `fuse_profile`, fix the file or entry
the message names; for `composition`, report a bug. A failure a source carried has no
stage and SHALL read `fuse: source '<name>' failed: <message>`, with no action. In that
text, the source name included, each control character but tab, each bidi control
(U+061C, U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069) and U+2028/U+2029 SHALL be
written as a visible escape. A run that exits 0 or 1 SHALL write no such cause.

Verified by: `cargo test -p latt --test fuse -- cli_clean_warning source_profile stderr staged`
and `cargo test -p latt --bin latt loading::tests`

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

#### Scenario: Failure cause on stderr
- **WHEN** a source named `specs` exits 2 and the command runs with `--format plain`
- **THEN** the command exits 2, stdout carries the report as before, and stderr carries
  one line starting `fuse: source 'specs' failed at source_run: trace exited 2` and
  ending `. Next: ` and the `source_run` action

### Requirement: Member findings keep their notes
A finding read from a source's trace SHALL keep the `note` that source's own profile
attached to it, unchanged, in the fuse report's JSON, plain and rich output, on the same
terms as the output capability's noted findings. A finding of the command's own with the
code and composed node of a source finding that carried a note SHALL carry that note,
unchanged: the composed orphan findings, a `VACANCY` the command judged in place of the
source's, and a fuse validation's finding on the node. `CROSS_SOURCE_DUPLICATE_ID` and
`AMBIGUOUS_CROSS_REF` judge across sources and SHALL carry no source's note; a
duplicate's node is a raw ID, which may spell another source's composed one. This
copies notes the sources
attached; fuse SHALL NOT read annotations of its own or match a source's notes file
against the composed graph, so an entry that matched nothing in its source's run
attaches to nothing in the fuse.

Verified by: `cargo test -p latt --locked --test fuse note`.

#### Scenario: A source's note survives composition
- **WHEN** source `product` carries a `VACANCY` finding on `REQ-2` with a note
- **THEN** the fuse JSON finding for `product:REQ-2` carries the same `note` object

#### Scenario: A note reaches the finding that replaces its own
- **WHEN** source `product`'s note is attached to its `UNTRACED` on `REQ-2`, which
  composition withdraws and the composed run reports again
- **THEN** the composed `UNTRACED` on `product:REQ-2` carries the same `note` object

#### Scenario: A note stays with its source
- **WHEN** sources `product` and `legacy` both declare raw ID `REQ-2`, and only
  `product`'s note matches it
- **THEN** findings on `legacy:REQ-2` carry no note

### Requirement: A source profile that cannot load is a source failure
Composition reads each source's profile for the ID patterns it qualifies onto composed
node kinds. A source profile that cannot be loaded SHALL be recorded as a
`SOURCE_FAILURE` finding at error severity naming the source and the profile path, on
the same terms as a source whose trace run failed: the report SHALL carry the other
sources' own findings and every `SOURCE_FAILURE`, the composition SHALL NOT run, and
the command SHALL exit 2. Composing without the pattern set would validate that
source's composed IDs against nothing and report the result as clean. The other
sources' findings SHALL arrive as each reported them, their `EMPTY_KIND` and notes
included, except that `--strict` still promotes their warnings.

A trace handed to composition SHALL have passed the trace checks; the type that
carries it SHALL NOT be constructible by deserializing it directly, so an unchecked
severity or note reaches composition as an error, never as a panic.

Verified by: `cargo test -p latt --test fuse -- source_profile failed_assembly`

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

