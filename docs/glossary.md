# Glossary

The terms the other pages use, each defined once. Look a word up here when a page
uses it without explaining it.

## Running Lattice

**Register.** The typed records a repository maintains as plain text. Lattice Auditing & Tracing Tool reads
registers; it never writes them.

**Target.** The directory (usually a repo checkout) whose register Lattice reads.
`--target` on the command line.

**Profile.** YAML declaring node kinds, edge kinds, ID patterns, attr declarations and
validations. See [profiles.md](profiles.md).

**Adapter.** What reads a register's format and produces an interface document: an
external program writing it to stdout, or a [native adapter](adapters.md#native-adapters)
built into the core. See [adapters.md](adapters.md#writing-your-own-adapter).

## The graph

**Node.** One record in the register.

**Edge.** A typed, directed link between two nodes. `edge_kinds` declares allowed
pairings.

**Kind.** A named category for nodes or edges. Opaque to the core. Lattice does not say "type",
which names an attr's value type.

**Schema.** The declared kinds, their attrs and the allowed pairs: what the profile says a
register may hold. The Schema view in `browse` (`S` in the Nodes view, in a build with
the `tui` feature) shows its projection: the kinds with their counts and the allowed
edges. Where the word would mean a file format, the prose says "profile format" or
"interface document format".

**Provenance.** Source file path (relative to target root) and line number.

**Pathway.** An ordered lifecycle sequence (e.g. draft, review, approved). When bound
to a finding code, the core demotes findings for nodes past the current position.

## Validation

**Finding.** A single validation result carrying severity, code, message, and origin; a
finding about one file carries its provenance, one about a whole input does not.

**Severity.** `error` (exit 1 when unsuppressed, from the commands that audit; `query` never exits 1:
[exit codes](cli.md#exit-codes)), `warning` (promotable by `--strict`), `info` (not
promoted by `--strict`; a profile override may raise it), `hint` (never promoted by
anything).

**Issue.** The Rust type a finding is held in, `latt_core::types::Issue` (also
exported from `latt_core::service`), and what [embedding.md](embedding.md) calls it.
Not an issue in a tracker such as GitHub or GitLab.

**Finding code.** Stable machine-readable name for a class of finding: `VACANCY`,
`UNREFERENCED`, `UNTRACED`, `COVERAGE`, `CONSTRAINT`, etc. Default severity is
profile-overridable.

**Vacancy.** A dangling reference: an edge names a target that does not exist. Its
finding code is `VACANCY`, the one crystallography term among them.

## Adapter boundary

**Interface document.** The JSON an adapter produces: nodes, edges, pathways,
findings, version. The only thing crossing the adapter boundary.

**Interface version.** The version of the interface document format. Unsupported versions
exit 2.

**Resolved profile.** Canonical JSON the core produces from a profile YAML. Inheritance
resolved, fields validated, `adapter:` namespace preserved.

**Ingest.** Reading an interface document into the in-memory graph: parsing, duplicate
detection, pathway validation, adapter-finding collection.

## Multi-source composition

**Fuse.** Combining multiple source registers into one graph via `latt fuse`. Each
source is traced independently; IDs and kinds are qualified with `source:` (colon
separator). See [fuse.md](fuse.md).

**Source.** One input register in a fuse operation, with its own profile, adapter and
target.

**Fuse manifest.** YAML listing sources to fuse plus a fuse profile for cross-source
validation.

**Fuse profile.** Profile governing the composed graph. Declares cross-source edge kinds
and validations. Node kinds belong to source profiles.

## Other terms

**Sidecar.** External program proposing edges; `validate --suggestions FILE` renders its
output as hints.

**Trace report.** The machine-readable form (`--format json`) that fuse and other tools
consume. [cli.md](cli.md#commands) lists every command.
