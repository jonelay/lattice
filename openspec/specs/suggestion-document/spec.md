# suggestion-document Specification

## Purpose
The interchange between a suggestion producer and the core: a program proposes edges the
register does not yet declare, as a ranked, schema-versioned document that is scratch by
contract. It keeps ranking — and anything that would need a model, a network or a corpus —
outside the core, which only renders what it is handed.

## Requirements

### Requirement: Suggestion document schema
A suggestion document SHALL be a JSON object carrying `suggestion_version`, `producer` and
`suggestions`. A suggestion SHALL carry `src`, `tgt`, `kind`, `score` and `basis`. `src` and
`tgt` SHALL be public node IDs; `kind` SHALL be an edge kind; `score` SHALL be a number;
`basis` SHALL be a string.

`basis` is free-form on purpose: a producer records how it reached the candidate — a
similarity figure, a model identifier, a prompt version, the ranks it fused — and the core
SHALL carry that text through without interpreting it. Constraining `basis` to a shape the
core understands would make the core know how its producers work, which is the coupling this
capability exists to avoid.

`producer` names the program and version that emitted the document, so a reviewer reading a
rendered suggestion can tell which ranker proposed it when several ran.

Verified by: `cargo test --test suggestions schema`

#### Scenario: A well-formed document is accepted
- **WHEN** the core reads a document declaring `suggestion_version`, a `producer` string, and
  one suggestion carrying all five fields
- **THEN** the core accepts it and renders that suggestion

#### Scenario: A missing required field is a contract failure
- **WHEN** a document declares a suggestion with no `basis` field
- **THEN** the core reports the document as failing the contract schema and exits 2, rather
  than rendering a partial entry

#### Scenario: Basis text survives unchanged
- **WHEN** a suggestion carries a `basis` of `cosine 0.83; model nomic-embed-text-v1.5`
- **THEN** the rendered finding carries that text verbatim

### Requirement: Suggestion document is versioned and negotiated
A suggestion document SHALL declare `suggestion_version`, and the core SHALL reject a version
it does not support rather than reading the document on a guess. An unsupported version is a
setup fault, not a finding: the core SHALL exit 2, the same way it treats an adapter
declaring an unsupported contract version.

Verified by: `cargo test --test suggestions version`

#### Scenario: Unsupported version is refused
- **WHEN** a document declares `suggestion_version` the core does not support
- **THEN** the core reports the unsupported version and exits 2

#### Scenario: Version mismatch is not a finding
- **WHEN** a document declares an unsupported version and the register itself is clean
- **THEN** the core exits 2, never 0 and never 1 — a broken overlay is distinguishable from a
  clean run and from a real finding

### Requirement: Suggestions are ordered deterministically in the document
A producer SHALL order `suggestions` by descending `score`, breaking ties on `src`, then
`tgt`, then `kind`, so a document is byte-reproducible for a given producer and input and two
producers that agree on scores agree on order. The document is where rank lives.

The core SHALL NOT preserve that order when rendering. A rendered suggestion is a finding, and
findings carry one total order across the whole report — by provenance, then code, then node
ID, then message — so that repeated runs are byte-identical whatever produced them. Giving the
overlay an ordering of its own would fork that guarantee for the sake of a ranking the reviewer
can already read: `score` travels in every rendered message, and the document itself is the
ranked artifact.

Verified by: `cargo test --test suggestions ordering`

#### Scenario: Rank is legible in the report
- **WHEN** a document carries suggestions scored 0.4, 0.9 and 0.7
- **THEN** every rendered line carries its score, so the ranking is recoverable however the
  report is ordered

#### Scenario: Rendering is deterministic
- **WHEN** the same register and the same document are rendered twice
- **THEN** both outputs are byte-identical, and suggestions sort into the report by the same
  key as every other finding

### Requirement: A suggestion document is ephemeral
A suggestion document SHALL be scratch output. Nothing in lattice SHALL write one into the
register, read one back as evidence of a relationship, or treat its absence as a finding. The
durable artifact of an accepted suggestion is a human's edit to the register, landing in a
reviewed diff and re-ingested through the ordinary adapter path.

A suggestion is therefore advice in flight, never derived state at rest — the distinction the
tool exists to hold.

Verified by inspection: the core has no write path for a suggestion document, and `--suggestions`
is read-only. Enforced by `cargo test --test suggestions readonly`, which asserts a run with
`--suggestions` creates and modifies no file.

#### Scenario: Running with suggestions writes nothing
- **WHEN** `lattice validate --suggestions <file>` runs to completion
- **THEN** no file in the target repo and no file in the lattice tree has been created or
  modified, including the suggestion document itself

#### Scenario: A suggestion is not evidence
- **WHEN** a suggestion proposes a `verifies` edge between a test and a requirement, and the
  register declares no such edge
- **THEN** coverage and trace answers for that requirement are unchanged — the register still
  says the requirement is unverified
