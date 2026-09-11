# Glossary

## Running lattice

**Register** — the typed records a repository maintains as plain text. Lattice reads
registers; it never writes them.

**Target** — the directory (usually a repo checkout) whose register lattice reads.
`--target` on the command line.

**Profile** — YAML declaring node kinds, edge kinds, ID patterns, attribute schemas,
validations. See [profiles.md](profiles.md).

**Adapter** — a standalone program that reads a register's format and emits an interface
document to stdout.

## The graph

**Node** — one record in the register.

**Edge** — a typed, directed link between two nodes. `edge_kinds` declares allowed
pairings.

**Kind** — a named category for nodes or edges. Opaque to the core.

**Provenance** — source file path (relative to target root) and line number.

**Pathway** — an ordered lifecycle sequence (e.g. draft → review → approved). When bound
to a finding code, the core demotes findings for nodes past the current position.

## Validation

**Finding** — a single validation result carrying severity, code, message, and provenance.

**Severity** — `error` (exit 1), `warning` (promotable by `--strict`), `info` (never
promoted), `hint` (never promoted by anything).

**Finding code** — stable machine-readable name for a class of finding: `VACANCY`,
`UNREFERENCED`, `UNTRACED`, `COVERAGE`, `CONSTRAINT`, etc. Default severity is
profile-overridable.

## Adapter boundary

**Interface document** — JSON an adapter writes to stdout: nodes, edges, pathways,
findings, version. The only thing crossing the adapter boundary.

**Interface version** — schema version of the interface document. Unsupported versions
exit 2.

**Resolved profile** — canonical JSON the core produces from a profile YAML. Inheritance
resolved, fields validated, `adapter:` namespace preserved.

**Ingest** — reading an interface document into the in-memory graph: parsing, duplicate
detection, pathway validation, adapter-finding collection.

## Multi-source composition

**Fuse** — combining multiple source registers into one graph via `lattice fuse`. Each
source is traced independently; IDs and kinds are qualified with `source:` (colon
separator). See [fuse.md](fuse.md).

**Source** — one input register in a fuse operation. Has its own profile, adapter, target.

**Fuse manifest** — YAML listing sources to fuse plus a fuse profile for cross-source
validation.

**Fuse profile** — profile governing the composed graph. Declares cross-source edge kinds
and validations. Node kinds belong to source profiles.

## Other terms

**Sidecar** — external program proposing edges via `--suggestions`. Renders as hints.

**Trace report** — the machine-readable form (`--format json`) that fuse and other tools
consume. See the README command table for the full command list.

## Crystal lattice concepts

Lattice borrows its name and several terms from crystallography.

**Unit cell** — the profile: the repeating structural pattern every register instance
tiles against.

**Polymorphism** — same register content through different profiles produces different
graphs.

**Vacancy** — a dangling reference: an edge names a target that does not exist.

**Dislocation** — a broken cross-reference chain propagating downstream.

**Grain boundary** — the interface where two independently grown sources meet in fuse.

**Epitaxy** — the suggestion sidecar: new edges proposed from existing structure.

**Diffraction** — trace: revealing the graph's structure from opaque plain-text registers.
