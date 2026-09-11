# program-composition Specification

## Purpose

Multi-source graph composition: combine independent single-source lattice runs
into a unified graph, qualify IDs and kinds by source, resolve cross-source
edges, and report findings. Implemented as `lattice fuse` (core subcommand).

## Requirements

### Requirement: Read a fuse manifest
`lattice fuse` SHALL accept a fuse manifest (YAML) declaring `manifest_version`,
`name`, `version`, `fuse_profile` (path to the fuse profile), and an ordered
`sources` list. Each source SHALL declare `name` (unique, must not contain `:`),
`profile`, `adapter`, and `target`. All paths SHALL be relative to the manifest
file's directory.

The command SHALL exit 2 if the manifest is missing, malformed, declares no
sources, contains duplicate source names, or has a source name containing `:`.

Verified by: `cargo test -p lattice-core --test fuse -- manifest`

#### Scenario: Valid manifest loads
- **WHEN** the command reads a manifest with two sources, each with name,
  profile, adapter, and target
- **THEN** it loads both sources in declared order

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
profiles.

Verified by: `cargo test -p lattice-core --test fuse -- rejects_bad_profiles`

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

### Requirement: Run each source through lattice trace
The command SHALL run `lattice trace --format json --profile <profile> --adapter
<adapter> --target <target>` for each source in manifest order. It SHALL parse
the JSON trace payload from each run.

A source whose trace run exits 2, fails to execute, or emits unparseable output
SHALL be recorded as `SOURCE_FAILURE`. The command SHALL continue running
remaining sources. After all sources run, if any source failed, the command SHALL
report the healthy sources' findings alongside SOURCE_FAILURE findings and exit 2
without assembling the graph.

A source whose trace run exits 0 or 1 SHALL contribute its trace payload to the
merge.

Verified by: `cargo test -p lattice-core --test fuse -- failed_trace`

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

Verified by: `cargo test -p lattice-core --test fuse -- assembly_qualifies`

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

Verified by: `cargo test -p lattice-core --test fuse -- duplicates_ambiguity`

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

### Requirement: Standard validators on the composed graph
The command SHALL construct a temporary profile from the composed graph's node
kinds and the fuse profile's edge kinds and validations, then run the standard
`validate` pass on the composed graph. Findings from standard validators (such as
UNREFERENCED, UNTRACED) SHALL carry source attribution derived from the node's provenance.
For node kinds emitted by source traces, the temporary profile SHALL qualify the
source profile's `id_pattern` so that it validates composed IDs while preserving
the source profile's validation of the raw ID. Node kinds added only from fuse
edge endpoint declarations SHALL accept any ID.

COVERAGE validations declared in the fuse profile SHALL route through the
standard validator on the composed profile. Before inclusion, the command SHALL
validate each COVERAGE entry's `edge_kind` against the fuse profile's declared
`edge_kinds`; an undeclared kind produces a CONFIG_ERROR finding and the entry
is excluded. Unresolved edges do not count as covering their target. COVERAGE
entries MAY include `where:` conditions, consistent with standard profiles.

`--strict` SHALL promote warnings to errors after all findings are collected,
consistent with single-source behavior.

Verified by: `cargo test -p lattice-core --test fuse -- standard_constraints duplicates_ambiguity coverage_undeclared coverage_repeated coverage_unknown coverage_where composed_ids`

#### Scenario: Standard validators produce findings on composed graph
- **WHEN** the fuse profile declares a CONSTRAINT validation and a composed
  node violates it
- **THEN** the command emits the CONSTRAINT finding with source attribution

#### Scenario: Composed IDs retain source pattern validation
- **WHEN** source `product` declares ID pattern `REQ-[0-9]+` and emits a node
  whose composed ID is `product:malformed`
- **THEN** the command emits an ID_FORMAT finding for that composed node
- **AND** it does not emit ID_FORMAT for a composed ID such as `product:REQ-001`

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

#### Scenario: Repeated COVERAGE entries share per-code severity
- **WHEN** two COVERAGE entries target the same kind with different severities
- **THEN** both checks run and the last-declared severity applies to all
  COVERAGE findings, per the standard per-code severity rule

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
from the fuse profile.

The fuse profile's `SUPPRESS` entries apply to the findings the command itself
collects — cross-source resolution findings (`CROSS_SOURCE_DUPLICATE_ID`,
`AMBIGUOUS_CROSS_REF`, `VACANCY`) and the standard-validator findings on the
composed graph. A fuse-level `node_ids` entry SHALL match against composed IDs
(`source:raw_id`), because that is the `node_id` the findings carry.
Suppression runs after `--strict` promotion and before the exit-code decision,
consistent with single-source behavior. `SUPPRESS_UNUSED` for a fuse-profile
entry is emitted against the fuse profile and names it as provenance.

A suppressed finding SHALL NOT count toward the command's exit code 1, and
SHALL render per the `output` capability: present in `json` with
`suppressed: true`, omitted from `plain` and `rich`.

Verified by: `cargo test -p lattice-core --test fuse -- suppress`

#### Scenario: Source-level suppression survives the merge
- **WHEN** source `product`'s profile suppresses `UNTRACED` and its trace run
  contributes an `UNTRACED` finding marked `suppressed: true`
- **THEN** the fuse output carries that finding with `suppressed: true` and it
  does not affect the fuse exit code

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
- **WHEN** a source profile suppresses `UNREFERENCED` and the fuse profile
  declares no `SUPPRESS` entries
- **THEN** the source's `UNREFERENCED` findings remain suppressed in the fuse
  output

### Requirement: Pathway preservation
The command SHALL preserve pathways from each source's trace payload, qualified
as `source:pathway_name`. Pathways SHALL appear in the fuse output.

Verified by: `cargo test -p lattice-core --test fuse -- assembly_qualifies`

#### Scenario: Pathways are source-qualified
- **WHEN** source `toml` has a pathway named `stage`
- **THEN** the fuse output contains pathway `toml:stage`

### Requirement: Tri-format output
The command SHALL output the fuse report through `output_result` in all three
formats (plain, json, rich). The JSON output SHALL include `header` (with `name`,
`version`, `manifest_version`), `nodes`, `edges`, `pathways`, and `findings`.
Each finding SHALL carry source attribution when applicable.

Verified by: `cargo test -p lattice-core --test fuse -- cli_clean_warning`

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
  or unparseable trace output.

Exit 2 SHALL never be collapsed into 1.

Verified by: `cargo test -p lattice-core --test fuse -- cli_clean_warning`

#### Scenario: Clean run
- **WHEN** all sources pass and no cross-source findings at error severity
- **THEN** the command exits 0

#### Scenario: Cross-source finding at error severity
- **WHEN** the composed graph has a `CROSS_SOURCE_DUPLICATE_ID`
- **THEN** the command exits 1

#### Scenario: Source failure
- **WHEN** one source's adapter exits 2
- **THEN** the command exits 2
