# Glossary

Terms as lattice uses them, in the order you are likely to meet them.

## Running lattice

**Register** — the collection of typed records a repository maintains as
plain text: requirements in a markdown table, specs in TOML files, test
declarations in source code. Lattice reads registers; it never writes them.

**Target** — the path to the directory (usually a repo checkout) whose
register lattice reads. Passed as `--target` on the command line.

**Profile** — the YAML file declaring what the register contains: node kinds,
edge kinds, ID patterns, attribute schemas, validations. The core ships no
domain vocabulary of its own; everything domain-specific arrives from a
profile. See [profiles.md](profiles.md) for how to write one.

**Adapter** — a standalone program that reads a register's own format and
emits an interface document. Each adapter bridges one format: markdown tables,
TOML files, GitHub issues, GitLab issues. Adapters are separate executables;
the core never imports or links adapter code.

## The graph

**Node** — one record in the register: a requirement, a spec, a test, a
device — whatever the profile's `node_kinds` declare.

**Edge** — a typed, directed link between two nodes. An edge says "this spec
satisfies that requirement" or "this test verifies that spec." The profile's
`edge_kinds` declare which pairings are allowed.

**Kind** — a named category for nodes or edges. A profile might declare node
kinds `requirement`, `spec`, and `test`, and edge kinds `satisfies` and
`verifies`. The core treats them as opaque labels.

**Provenance** — where a node, edge, or finding came from: a file path
relative to the target root and a line number. Provenance is carried through
every stage so that output always points back to the source text.

**Pathway** — an ordered lifecycle sequence declared in the register, such as
draft → review → approved. When a pathway is bound to a finding code, the
core demotes findings for nodes whose position is after the register's
current position on that pathway.

## Validation output

**Finding** — a single validation result: a gap, violation, or observation
the core or an adapter reports. Each finding carries a severity, a code, a
message, and a provenance.

**Severity** — the weight of a finding:

- `error` — sets exit code 1.
- `warning` — reported; promotable to error by `--strict`.
- `info` — reported; never promoted.
- `hint` — advice the tool cannot stand behind; never promoted by anything.

**Finding code** — the stable machine-readable name for a class of finding:
`VACANCY`, `ORPHAN_NODE`, `PARSE_ERROR`, `COVERAGE`, `CONSTRAINT`, and
others. A profile may override a code's default severity.

## The adapter boundary

**Interface document** — the JSON an adapter writes to stdout: nodes, edges,
pathways, findings, and a version field. This is the only thing that crosses
the adapter boundary; the core ingests it and builds its graph from it.

**Interface version** — the schema version of the interface document. The
core accepts documents declaring versions it supports and exits 2 for
anything else.

**Resolved profile** — the canonical JSON the core produces from a profile
YAML and hands to the adapter as `--profile`. All inheritance is resolved,
all fields are validated, and the adapter's own `adapter:` namespace is
reproduced unchanged. Adapters read this instead of the user's YAML file.

**Ingest** — the core's process of reading an interface document into its
in-memory graph: parsing the JSON, detecting duplicate node IDs, validating
pathway integrity, and collecting adapter-emitted findings.

## Multi-source composition

**Fuse** — combining multiple source registers into a single graph. Each
source is traced independently; `lattice fuse` ingests all traces, qualifies
node IDs and kinds with a `source:` prefix (colon separator), and runs the
standard validators on the composed graph. Implemented as `lattice fuse
--manifest <manifest.yaml>`.

**Source** — one input register in a fuse operation, identified by name. Each
source has its own profile, adapter, and target.

**Fuse manifest** — the YAML file listing the sources to fuse: their names,
profiles, adapters, and targets, plus a fuse profile for cross-source
validation. Manifest keys: `name`, `version`, `fuse_profile`, `manifest_version`,
and `sources`.

**Fuse profile** — the profile governing the composed graph. It declares
cross-source edge kinds (whose allowed pairings name source-qualified kinds)
and cross-source validations. Node kinds belong to source profiles, not the
fuse profile.

## Auxiliary

**Sidecar** — an external program that proposes edges (suggestions) for a
register, consumed via `--suggestions`. Suggestions render as hints and do
not affect the exit code.

**Trace** — the full per-node report: every node with its kind, attrs,
provenance, outgoing edges, and attached findings. `lattice trace` produces
it; `lattice trace --format json` emits the machine-readable form that fuse
and other tools consume.

## Crystal lattice concepts

Lattice borrows its name and several concepts from crystallography. These
terms appear in documentation, design discussions, and roadmap thinking.

### Structure

**Unit cell** — in crystallography, the smallest repeating structural unit
that tiles to fill a crystal. A profile is a unit cell: it declares the
pattern — kinds, edges, constraints — that every register instance must
tile against. The register is the crystal; the profile is the repeating
rule.

**Polymorphism** — the same chemical composition crystallizing into
different structures. The same register content read through different
profiles produces different graphs — different kinds, different edges,
different findings. The data is invariant; the structure depends on the
lens.

### Defects

**Defect** — any deviation from a crystal's ideal periodic structure.
Findings are defects: the profile declares the ideal structure, and
validation reports every deviation from it. The finding-code taxonomy
(`VACANCY`, `ORPHAN_NODE`, `ID_PATTERN`, `CONSTRAINT`) is a defect
classification.

**Vacancy** — a missing atom at a site the crystal structure expects to be
occupied. A dangling reference is a vacancy: an edge names a target node
that does not exist. The graph has a hole where a record should be.

**Dislocation** — a line defect where a break propagates through the
crystal structure, distorting everything downstream. A broken
cross-reference chain — A→B→C where B was renamed — is a dislocation:
one break distorts the entire reachability closure beyond it.

### Composition

**Grain boundary** — the interface where two independently grown crystal
regions meet, with mismatched orientations. This is what fuse produces: two
source registers that grew independently, joined at cross-source edges
where their ID namespaces meet. The `:` qualifier that distinguishes
`toml:REQ-001` from `md:REQ-001` is the orientation marker.

**Epitaxy** — growing a new crystal layer on an existing substrate, with
the substrate's structure guiding the new growth. The suggestion sidecar is
epitaxial: it proposes new edges guided by the existing graph's structure
and text similarity, growing the traceability layer on top of what the
register already declares.

### Analysis

**Diffraction** — bouncing waves off a crystal to reveal its internal
structure from the interference pattern. Trace is diffraction: you cannot
see the graph directly in the source files, but the trace report reveals
the full structure — every node, edge, and defect — from a single pass
over opaque plain-text registers.
