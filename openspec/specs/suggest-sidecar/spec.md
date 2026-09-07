# suggest-sidecar Specification

## Purpose
`lattice-suggest`, the program that proposes candidate edges the register does not yet
declare. It ranks; it never decides, never writes, and never runs as part of a gating check.
This capability covers its inputs, its offline default, and the embedding baseline every
later ranker is measured against.
## Requirements
### Requirement: The sidecar is a program emitting a suggestion document
`lattice-suggest` SHALL be an executable program. Given the resolved profile and a contract
document, it SHALL write a single suggestion document to stdout and exit 0. A malformed or
unreadable *input* SHALL become an issue reported on stderr with a still-valid document on
stdout, or an explicit non-zero exit; it SHALL NOT be dropped in silence.

A non-zero exit means the sidecar itself broke, not that a suggestion was rejected — the same
division the adapter contract draws.

It takes the resolved profile because the contract document alone does not carry
`summary_attr` or the profile's allowed endpoint pairs, and both are needed to know what text
to compare and which pairings are legal.

Verified by: `.venv/bin/python -m pytest tests/test_suggest.py -k program`

#### Scenario: Emits a document on stdout
- **WHEN** the sidecar is run with a resolved profile and a contract document
- **THEN** it writes one suggestion document to stdout and exits 0

#### Scenario: Unreadable input is reported, not dropped
- **WHEN** the contract document contains a node the sidecar cannot read
- **THEN** the sidecar reports it and still emits a document for the nodes it could read

### Requirement: Candidates are constrained to profile-legal pairings
The sidecar SHALL only propose an edge whose `(src kind, tgt kind, edge kind)` triple is
declared `allowed` by the profile. A pairing the profile forbids SHALL NOT appear in the
document at any score, because the core would reject it as an `EDGE_CONSTRAINT` violation if
a human acted on it — proposing it wastes the reviewer's attention on an edit that cannot land.

The profile is the only place the sidecar learns what kinds exist. It SHALL NOT name a node
kind, an edge kind or an ID shape of its own.

Verified by: `.venv/bin/python -m pytest tests/test_suggest.py -k constrained`

#### Scenario: A forbidden pairing is never proposed
- **WHEN** the profile allows `verifies` only from `test` to `req`, and the two most similar
  texts in the register belong to a `test` and a `spec` node
- **THEN** that pair appears nowhere in the document

#### Scenario: No vocabulary of its own
- **WHEN** the sidecar runs against a profile whose kinds are named differently
- **THEN** it proposes edges in that profile's vocabulary, with no change to the sidecar

### Requirement: Ranking baseline is embedding similarity over declared text
For this capability's baseline, the sidecar SHALL rank candidate edges by cosine similarity
between embeddings of the text each node declares. The text a node offers SHALL be the values
of the attrs the profile names in that node kind's `text_attrs`, joined in the declared order.
A node kind declaring no `text_attrs` SHALL fall back to its `summary_attr` value, so a profile
written before `text_attrs` existed ranks exactly as it did. The `basis` SHALL record the
similarity figure and the embedding model's identifier so a reviewer can tell what produced the
ranking.

Where the profile declares a `text_chunk_line_prefix` for a **target's** kind, the sidecar SHALL
subdivide that target's joined text into chunks, embed each chunk, and take the pairing's `score`
to be the **maximum** cosine similarity over the target's chunks. Where no prefix is declared the
target offers one undivided text and the score is that single similarity, unchanged.

Pooling SHALL be maximum, and SHALL NOT be configurable. Averaging the per-chunk scores measured
*below* the unchunked baseline on this register (hit@1 0.416 against 0.507), because it puts every
irrelevant block back into the number — the dilution chunking exists to escape. The gain is
specifically max-pooling, not chunking as such.

Subdivision SHALL apply to targets only. A node kind may be a target of one edge kind and a source
of another, so chunking is a property of the role, not of the kind; sources SHALL be ranked as one
undivided text whether or not their kind declares a prefix. This is also the arrangement the
measurement held fixed.

The sidecar SHALL NOT name an attr of its own, and SHALL NOT name a split marker of its own.
Which attrs carry rankable text, and where a target's text divides, are the profile's
declarations, because either one chosen by the sidecar is domain vocabulary inside a program
that must work against any register — the same reason node kinds and edge kinds are profile
data. The marker is compared as a literal string prefix of a line, never as a regex.

The population this baseline targets is the set of nodes carrying no edge of the proposed
kind — the tests with no requirement marker. The output is a ranked review list, not an oracle.

Chunking a target changes its ranking against **every** edge kind for which it is a legal target,
not only the one being ranked.

Measured against this register's 219 citation labels, one vector per requirement put the true
requirement first for 51% of unmarked tests and never surfaced 8% of them in a 20-deep list;
chunking with max-pooling moves those to 63% and 2% (`notes/chunking-s40/RESULTS.md`). The gain is
granularity, not semantic alignment — a control cutting the same documents into the same number of
chunks at arbitrary boundaries scores within one label — but a fully generic fixed-window splitter
measured 0.04–0.09 hit@1 worse, which is why the cut points are the register's own and the profile
names them.

Verified by: `.venv/bin/python -m pytest tests/test_suggest.py -k "rank or chunk"`

#### Scenario: More similar text ranks higher
- **WHEN** one candidate requirement's text closely paraphrases a test's docstring and another
  is unrelated
- **THEN** the paraphrasing candidate carries the higher score and sorts first

#### Scenario: Basis records the model
- **WHEN** the sidecar ranks with a named embedding model
- **THEN** every suggestion's `basis` carries that model's identifier

#### Scenario: Declared text attrs are the ranked text
- **WHEN** a node kind declares `text_attrs: [title, body]` and a node carries both
- **THEN** both values are ranked, and a profile naming only `title` ranks the title alone

#### Scenario: A kind with no text_attrs falls back to its summary attr
- **WHEN** a node kind declares `summary_attr: text` and no `text_attrs`
- **THEN** the sidecar ranks that kind's `text` values, unchanged from before `text_attrs` existed

#### Scenario: An explicitly empty text_attrs does not fall back
- **WHEN** a node kind declares `summary_attr: text` and `text_attrs: []`
- **THEN** the sidecar ranks no text for that kind and its nodes are excluded as textless,
  rather than falling back to `text`

#### Scenario: A declared prefix splits a target into chunks
- **WHEN** a target kind declares `text_chunk_line_prefix: "#### Scenario:"` and a target's
  joined text holds a preamble followed by three lines starting with that prefix
- **THEN** the backend receives four separate texts for that target — the preamble and one per
  marked block — instead of one

#### Scenario: A pairing scores as the target's best chunk
- **WHEN** a target is chunked and one of its chunks is far more similar to a source than the
  target's text as a whole
- **THEN** the pairing's score is that chunk's similarity, not the average over chunks

#### Scenario: A kind declaring no prefix is ranked undivided
- **WHEN** no node kind in the profile declares `text_chunk_line_prefix`
- **THEN** the texts sent to the backend and the resulting suggestions are identical to those
  produced before this key existed

#### Scenario: Sources are never chunked
- **WHEN** a node kind declaring a `text_chunk_line_prefix` supplies the sources of the ranked
  edge kind
- **THEN** each source is sent to the backend as one undivided text

#### Scenario: Splitting drops no text
- **WHEN** a target's joined text is subdivided at a declared prefix
- **THEN** concatenating the chunks in order reproduces the joined text, save for any part that
  is entirely whitespace — each chunk but the last keeps its trailing newline, because the cut
  falls after the separator — so no region carrying text goes unranked

#### Scenario: A target whose text holds no marker stays whole
- **WHEN** a target's kind declares a prefix but that target's text contains no line starting
  with it
- **THEN** that target is ranked as one undivided text rather than as zero chunks

#### Scenario: A declared prefix that never splits anything is reported
- **WHEN** a kind declares a `text_chunk_line_prefix` and no target of that kind yields more than
  one chunk
- **THEN** the sidecar reports that on stderr, because a marker absent from every node is a
  profile or adapter fault rather than a register with no blocks

### Requirement: The sidecar runs offline by default
With no embedding backend configured, the sidecar SHALL run without a network, an API key or
a GPU, and SHALL report plainly that no backend is available rather than failing obscurely or
silently emitting an empty document. Any backend requiring a network or an accelerator SHALL
be opt-in on the command line.

This keeps CI hermetic and keeps the real-data gate runnable on a host that cannot reach the
embedding host. The suggestion path is advisory, so it must never be the reason a check cannot
run.

Verified by: `.venv/bin/python -m pytest tests/test_suggest.py -k offline`

#### Scenario: No backend configured
- **WHEN** the sidecar runs with no embedding backend named
- **THEN** it reports that no backend is configured and emits a document with zero
  suggestions, distinguishable from a backend that ran and found nothing

#### Scenario: No network on the default path
- **WHEN** the sidecar runs with no backend named and no network is reachable
- **THEN** it exits 0

### Requirement: A node with no text is reported and excluded, never silently ranked
A candidate node offering no text to rank on — no non-empty value for any attr the profile
names as rankable text for its kind — SHALL be excluded from ranking before any embedding
call is made, and the exclusion SHALL be reported on stderr with a count per side alongside
the existing ranked-population report. When exclusion leaves either side of the pairing
empty, the sidecar SHALL emit a valid document with zero suggestions, report why, and
exit 0.

A textless node has no defined input to the ranking baseline, so any score computed for
it is fabricated — the S35 live run produced 4,520 suggestions all at cosine 1.0000 from
a register whose profile declared no rankable text, indistinguishable on stdout from a
strong ranking. Absent text is not malformed input, so the unreadable-input requirement
does not cover it; it needs its own report. Exclusion happens before the embed call so
an all-textless register never needs a backend at all.

Verified by: `.venv/bin/python -m pytest tests/test_suggest.py -k text`

#### Scenario: A textless node is excluded and reported
- **WHEN** one candidate source node carries no non-empty value for any of its kind's
  rankable attrs, and others carry text
- **THEN** no suggestion names the textless node, the textless nodes' texts are never
  sent to the backend, and stderr reports how many sources and targets were excluded

#### Scenario: An all-textless population makes no backend call
- **WHEN** every candidate node on one side carries no text
- **THEN** the sidecar emits a document with zero suggestions, reports the empty ranked
  population on stderr, exits 0, and never invokes the embedding backend

### Requirement: Duplicate node IDs rank as one candidate

When the contract document carries several nodes under one ID, the sidecar SHALL rank
that ID once, over the union of the occurrences' text: source occurrence texts are
concatenated in occurrence order, and target occurrences' chunk lists are concatenated,
so the best-block score covers every occurrence. Exactly one suggestion SHALL be
emitted per (src, tgt, kind) triple. The merge SHALL be reported on stderr with a count
of merged occurrences; the node identity carried in suggestions is the first
occurrence's, matching the core's ingest rule for which occurrence becomes the node.

The contract retains every occurrence by design and the core reports the duplicate as
a finding; the sidecar neither repeats that finding nor resolves it. It unions rather
than keeping one occurrence because a suggestion list is a recall surface: an
occurrence never scored is a candidate no reviewer can rescue, which is the one failure
a ranked review list must not have. An ID whose union of text is empty is excluded and
reported under the textless-node requirement as before.

Verified by: `.venv/bin/python -m pytest tests/test_suggest.py -k duplicat`

#### Scenario: A duplicated target scores its first occurrence's text

- **WHEN** a target ID occurs twice and only the first occurrence's text matches a
  source
- **THEN** the pair scores on that matching text, and the target appears at most once
  in the source's suggestion list

#### Scenario: A duplicated source ranks once over its union of text

- **WHEN** a source ID occurs twice, one occurrence carrying text and one carrying none
- **THEN** the source is ranked on the union rather than excluded as textless, and each
  of its suggested targets appears once

#### Scenario: Merged duplicates are reported

- **WHEN** any ID occurs more than once among the candidates
- **THEN** stderr reports how many occurrences were merged

