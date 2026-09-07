# program-composition Specification

## Purpose

External shim for composing multiple single-source lattice runs into a unified
program graph, validating cross-source edges, and reporting findings — the
consumer-first step before native core support.

## Requirements

### Requirement: Read a program manifest
The shim SHALL accept a program manifest (YAML) declaring `manifest_version`,
`program` (name), `program_version`, `program_profile` (path to the program
profile), and an ordered `sources` list. Each source SHALL declare `name`
(unique), `profile`, `adapter`, and `target`. All paths SHALL be relative to the
manifest file's directory.

The shim SHALL exit 2 if the manifest is missing, malformed, declares no sources,
or contains duplicate source names.

Verified by: `.venv/bin/python -m pytest tests/test_lattice_compose.py -k manifest`

#### Scenario: Valid manifest loads
- **WHEN** the shim reads a manifest with two sources, each with name, profile,
  adapter, and target
- **THEN** it loads both sources in declared order

#### Scenario: Duplicate source names
- **WHEN** a manifest declares two sources with the same name
- **THEN** the shim exits 2 with an error naming the duplicate

#### Scenario: Manifest missing
- **WHEN** the manifest path does not exist
- **THEN** the shim exits 2

### Requirement: Read a program profile
The shim SHALL load the program profile referenced by the manifest. The program
profile SHALL declare `edge_kinds` with allowed endpoint pairs using
source-qualified kind names (`source/kind`), and optionally `validations` for
cross-source checks.

The program profile SHALL NOT declare `node_kinds` — those are owned by source
profiles.

Verified by: `.venv/bin/python -m pytest tests/test_lattice_compose.py -k program_profile`

#### Scenario: Program profile with cross-source edge kinds
- **WHEN** the program profile declares `derives` with allowed pair
  `[compliance/clause, product/requirement]`
- **THEN** the shim uses this for cross-source edge resolution

#### Scenario: Program profile declares node_kinds
- **WHEN** the program profile contains a `node_kinds` section
- **THEN** the shim exits 2 — node kinds belong to source profiles

### Requirement: Run each source through lattice
The shim SHALL run `lattice trace --format json --profile <profile> --adapter <adapter> --target <target>` for each source in manifest order. It SHALL parse the JSON trace
payload from each run.

A source whose lattice run exits 2 SHALL be recorded as failed. The shim SHALL
continue running remaining sources. After all sources run, if any source exited 2,
the shim SHALL report the healthy sources' findings and exit 2 without attempting
merge.

A source whose lattice run exits 0 or 1 SHALL contribute its trace payload to the
merge.

Verified by: `.venv/bin/python -m pytest tests/test_lattice_compose.py -k source_run`

#### Scenario: All sources succeed
- **WHEN** all sources exit 0 or 1
- **THEN** the shim proceeds to merge

#### Scenario: One source exits 2
- **WHEN** one of three sources exits 2 and the other two exit 0
- **THEN** the shim reports the two healthy sources' findings and exits 2

### Requirement: Merge trace payloads with source qualification
The shim SHALL merge trace payloads from all successful sources in declared order.
Node kinds SHALL be qualified by source name: a node of kind `requirement` from
source `product` becomes kind `product/requirement` in the merged graph. Node IDs
SHALL remain unqualified (flat).

If a node ID appears in multiple sources, the shim SHALL emit a
`CROSS_SOURCE_DUPLICATE_ID` finding at error severity naming both sources and the
conflicting ID. All conflicting occurrences SHALL be reported, not just the second.

Merge order SHALL affect output ordering only, not which declaration wins.

Verified by: `.venv/bin/python -m pytest tests/test_lattice_compose.py -k merge`

#### Scenario: Kinds are source-qualified
- **WHEN** source `product` emits a node of kind `requirement` and source
  `compliance` emits a node of kind `clause`
- **THEN** the merged graph contains `product/requirement` and `compliance/clause`

#### Scenario: Duplicate ID across sources
- **WHEN** source `product` and source `compliance` both emit a node with ID `T-11`
- **THEN** the shim emits `CROSS_SOURCE_DUPLICATE_ID` naming both sources

### Requirement: Resolve cross-source edges via allowed pairings
For each edge whose kind is declared in the program profile's `edge_kinds`, the
shim SHALL resolve the target by finding a node whose source-qualified kind matches
the allowed target kind and whose ID matches the edge's target. The source of the
edge is determined by which source's adapter emitted it.

If the target ID matches nodes in multiple sources and the allowed pairings do not
disambiguate, the shim SHALL emit `AMBIGUOUS_CROSS_REF` at error severity.

If the target ID matches no node in any allowed target kind, the shim SHALL emit
`DANGLING_REF` at the program profile's declared severity for that edge kind.

Verified by: `.venv/bin/python -m pytest tests/test_lattice_compose.py -k resolution`

#### Scenario: Unambiguous resolution
- **WHEN** `derives` allows `[compliance/clause, product/requirement]` and a
  compliance edge targets `E1.01` which exists only in source `product`
- **THEN** the edge resolves to `product/requirement` node `E1.01`

#### Scenario: Ambiguous resolution
- **WHEN** `derives` allows two target kinds and the target ID exists in both
- **THEN** the shim emits `AMBIGUOUS_CROSS_REF`

#### Scenario: Unresolvable target
- **WHEN** an edge targets an ID that exists in no allowed target kind
- **THEN** the shim emits `DANGLING_REF`

### Requirement: Three-phase validation
The shim SHALL validate in three phases:

1. **Source validation.** Each source's lattice run validates independently. Only
   source-local findings are reported. Cross-source edge kinds (marked
   `cross_source: true` in the source profile) produce hint-severity DANGLING_REF
   for unresolvable targets rather than errors.

2. **Merge.** Payloads are merged with source qualification and duplicate detection.

3. **Cross-source validation.** The program profile's edge kinds and validations
   are checked against the merged graph.

Source profiles govern phase-1 finding severity. The program profile governs
phase-3 finding severity.

Verified by: `.venv/bin/python -m pytest tests/test_lattice_compose.py -k phase`

#### Scenario: Source findings reported before cross-source
- **WHEN** a source has an intra-source DANGLING_REF and the merged graph has a
  cross-source DANGLING_REF
- **THEN** both are reported, with the source finding attributed to its source

#### Scenario: Source-local coverage not contradicted by cross-source edges
- **WHEN** a source profile declares a COVERAGE validation and the program profile
  declares a COVERAGE validation over the same target kind but different edge kind
- **THEN** both run independently in their respective phases

### Requirement: JSON output
The shim SHALL write JSON to stdout containing the merged graph (nodes with
source-qualified kinds, edges) and all findings from all three phases. Each
finding SHALL carry source attribution when applicable. The `axes` field SHALL
be present but empty — axis merging is deferred to native core support.

Verified by: `.venv/bin/python -m pytest tests/test_lattice_compose.py -k output`

#### Scenario: Output contains merged nodes and findings
- **WHEN** the shim runs against a two-source program
- **THEN** stdout is valid JSON containing nodes from both sources with qualified
  kinds, edges, and any findings from all phases

### Requirement: Program exit codes
The shim SHALL use the same three-valued exit codes as lattice:
- **0** — no error-severity findings in any phase.
- **1** — error-severity findings in source or cross-source validation.
- **2** — shim could not run: bad manifest, bad program profile, or any source
  adapter exiting 2.

Verified by: `.venv/bin/python -m pytest tests/test_lattice_compose.py -k exit_code`

#### Scenario: Clean run
- **WHEN** all sources pass and no cross-source findings at error severity
- **THEN** the shim exits 0

#### Scenario: Cross-source finding at error severity
- **WHEN** the merged graph has a `CROSS_SOURCE_DUPLICATE_ID`
- **THEN** the shim exits 1
