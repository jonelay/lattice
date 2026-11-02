# adapter-prose-xref Specification

## Purpose
Parses Markdown section cross-references using the backtick-delimited `§`
convention and emits `document`/`section` nodes with `xref` edges into an
interface document.

## Requirements
### Requirement: Build document and section nodes from content directories

The adapter SHALL read `adapter.paths.content_dirs` from the resolved profile.
It SHALL reject an `adapter` or `adapter.paths` key it does not read, naming the
key, with exit 2 — a mistyped `content_dir` would otherwise scan nothing; the
core-owned `native` and `timeout_seconds` keys are admitted. It SHALL walk those directories under the target recursively, reading only regular
files, and symlinks that resolve to files, whose target-relative paths match a
configured glob. It SHALL NOT follow symlinked directories. For each matched
`.md` file it SHALL emit one `document` node and one `section` node per heading.
Headings inside fenced code blocks SHALL be excluded, and so SHALL references.
Indentation SHALL NOT mark code: a reference on a line indented four or more spaces, or
by a tab, SHALL be read like any other — such a line is list-item continuation as often
as it is code, and an author who means code can fence it. A line so indented SHALL NOT
be a heading.
A fence opens on a line of three or more backticks or tildes at any indentation, so a
fence continuing a list item hides what it holds — except that, as in CommonMark, a
backtick run whose info string holds a backtick is inline code and opens nothing — and closes only on a line of the same
character, at least as long, at any indentation, with nothing after it but whitespace.
A fence still open at the end of the file SHALL be a `PARSE_ERROR` at the line that
opened it, since every heading and reference after that line went unread.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k "document_nodes or section_nodes or fenced_code"`, and `cargo test -p latt-core --lib adapters::prose_xref headings_skip`, `cargo test -p latt-core --lib adapters::prose_xref fence`, `cargo test -p latt-adapter fence`, `cargo test -p latt-core --lib adapters::prose_xref indented_lines`, `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k unclosed_fence`, and `cargo test -p latt-core --lib adapters::prose_xref unknown_adapter_keys_are_named`

#### Scenario: Files produce document and section nodes

- **WHEN** the target contains `characters/alice.md` with headings
- **THEN** the document contains a `document` node `characters/alice.md` and
  `section` nodes for each heading

#### Scenario: Fenced code headings are excluded

- **WHEN** a heading appears inside a fenced code block
- **THEN** no section node is emitted for it

#### Scenario: A fence inside a list item hides its references

- **WHEN** a list item holds a fence indented four spaces whose body contains
  `` § `In` ``
- **THEN** no reference to `In` is read, and the lines after the closing fence
  are read

#### Scenario: Unclosed fence

- **WHEN** a fence opened on line 5 is never closed
- **THEN** the adapter emits a `PARSE_ERROR` at line 5 saying the fence is never
  closed, and emits no section node for a heading after it

#### Scenario: Indented lines carry references but not headings

- **WHEN** a list item continues on a line indented four spaces that contains
  `` § `Spaced` ``, and another line indented four spaces reads `# Not a heading`
- **THEN** a reference to `Spaced` is read, and no section node is emitted for
  `Not a heading`

#### Scenario: A fence line with trailing text does not close

- **WHEN** a fence opened by ` ``` ` is followed by the line ` ``` not a close `
- **THEN** the fence stays open and headings and references after that line are
  excluded until a bare ` ``` ` line

### Requirement: Report unreadable input as PARSE_ERROR

Every configured content directory is required. The adapter SHALL emit one
`PARSE_ERROR` at line 0 and continue, never exit non-zero, when:

- the target is not a directory — one finding naming the target, and no other
  output;
- a content directory does not exist or is not a directory — provenance is the
  configured directory name;
- a directory, a directory entry or an entry's type cannot be read;
- a matched file cannot be read — no `document` node is emitted for it;
- a line of a matched file is not valid UTF-8 — the finding is at that line, shown as
  written, as the adapter contract states; the line is read as blank, so no heading or
  reference comes from it, and the rest of the file is read;
- a matched symlink resolves to nothing — the file cannot be read, as above;
- a matched path is not valid UTF-8 — "path is not valid UTF-8", naming the path with
  the invalid bytes replaced by U+FFFD and carrying it with those bytes escaped in the
  message, in path order; the file is not read and is not discovered.

A reference whose target file was discovered but could not be read SHALL be a
`PARSE_ERROR` "target file unreadable: <file>" at the reference's line, not a
missing-file finding. A profile MAY downgrade these findings with a
`PARSE_ERROR` severity override. Exit 2 is reserved for a profile that cannot
be loaded and for failure to write the document.

When the target is a directory but no file under the content directories matches the
configured globs, the adapter SHALL emit one `NO_MATCHING_FILES` issue at `info`
severity, beside any `PARSE_ERROR` for a missing content directory, whose message names
the globs and the content directories, so a mistyped glob never reads as an empty
register.

A reference naming a path that, taken relative to the referring file, climbs out of
the target with `..` SHALL be a `PARSE_ERROR` "target outside the register: <path>" at
the reference's line, and no edge: the register is the target, and a file beyond it is
neither discovered nor read.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k "invalid_utf8 or missing_content_directory or non_directory_target or severity_follows or dangling_symlink or non_utf8_path or no_matching_files or outside_the_register or climbs_and_returns"`, and `cargo test -p latt-core --lib adapters::prose_xref decode`, and `cargo test -p latt-core --lib adapters::prose_xref walk_error`, and `cargo test -p latt-core --lib adapters::prose_xref normalize_path`

#### Scenario: Invalid UTF-8 file

- **WHEN** `ref/bad.md` reads `# Bad`, then two lines that are not valid UTF-8 (the
  second `## Gone caf\xE9`), and `ref/pointer.md` references `` `ref/bad.md` § `Bad` ``
- **THEN** the adapter exits 0 with a `PARSE_ERROR` at each of the two lines, shown as
  written; `ref/bad.md` and its `Bad` section are nodes, no section comes from the
  `Gone` line, and the reference resolves

#### Scenario: Dangling symlink

- **WHEN** `ref/gone.md` is a symlink to a missing file and `ref/pointer.md`
  references `` `ref/gone.md` § `Gone` ``
- **THEN** the adapter emits one `PARSE_ERROR` for `ref/gone.md` and a
  `PARSE_ERROR` "target file unreadable: ref/gone.md" on `ref/pointer.md`

#### Scenario: Missing content directory

- **WHEN** the profile lists `worldbuilding` and the target has no such directory
- **THEN** one `PARSE_ERROR` "content directory 'worldbuilding' does not exist or
  is not a directory" is emitted and the other directories are read

#### Scenario: No files match

- **WHEN** every content directory exists but holds no `.md` file
- **THEN** the adapter emits no nodes and one `NO_MATCHING_FILES` issue at `info`
  severity naming `**/*.md` and the content directories

#### Scenario: A reference climbs out of the register

- **WHEN** `ref/notes.md` references `` `../../outside.md` § `X` `` and a file
  `outside.md` exists beside the target
- **THEN** the adapter emits a `PARSE_ERROR` "target outside the register:
  ../../outside.md" and no edge

#### Scenario: Downgraded missing directory

- **WHEN** the profile sets `PARSE_ERROR` to `warning` and a content directory
  is missing
- **THEN** `latt validate` reports the finding as a warning and exits 0

### Requirement: Parse backtick-delimited cross-references

The adapter SHALL recognize `` `file.md` § `Heading` `` as a cross-file
reference and `` § `Heading` `` as a self-reference, emitting one `xref` edge
from the nearest preceding section to the matched target.

A Markdown inline link to a `.md` file directly before the `§` SHALL name the file as
a backtick path does: `` [text](path/file.md) § `Heading` ``, with an optional
`#fragment` or quoted title in the link, which are ignored. Its path SHALL resolve
exactly as a backtick path resolves. Read as prose, the link was ignored and the
reference resolved in the citing file instead — an edge to the wrong section, or a
`VACANCY` naming a file nobody cited.

Whitespace between the file and the `§` MAY include a non-breaking space (U+00A0,
U+202F), as typeset prose writes one.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k "cross_file_xref or self_ref_xref or markdown_link"`, and `cargo test -p latt-core --lib adapters::prose_xref backtick_delimited`, and `cargo test -p latt-core --lib adapters::prose_xref markdown_link_prefix`, and `cargo test -p latt-core --lib adapters::prose_xref non_breaking_space`

#### Scenario: Cross-file reference

- **WHEN** `alice.md` contains `` `bob.md` § `Personality` ``
- **THEN** one `xref` edge targets `characters/bob.md#Bob::Personality`

#### Scenario: Self-reference

- **WHEN** `alice.md` contains `` § `Background` `` and has a `Background` heading
- **THEN** one `xref` edge targets `characters/alice.md#Alice::Background`

#### Scenario: A non-breaking space keeps the file

- **WHEN** a line contains `` `a.md` `` then U+00A0 then `` § `X` ``
- **THEN** the reference resolves against `a.md`, not the citing file

#### Scenario: A Markdown link names the file

- **WHEN** `ref/notes.md`, which has its own `Alpha` heading, contains
  `` [a](a.md) § `Alpha` and § `Beta` `` and `` [the A file](./a.md#alpha) §`Alpha` ``
- **THEN** all three `xref` edges target `ref/a.md`, none `ref/notes.md`, and no
  finding is emitted

### Requirement: Read every heading listed after `§§`

`§§` followed by a backtick-delimited heading SHALL begin a list. Each further
backtick-delimited heading joined to the previous one by a comma, `and`, `or`, or
a comma and one of those words SHALL be a reference of its own, against the same
file as the first. Read as prose, every heading after the first was lost with no
finding.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k double_section_sign`, and `cargo test -p latt-core --lib adapters::prose_xref double_section_sign`

#### Scenario: A listed heading is a reference

- **WHEN** `ref/notes.md` contains `` [a](a.md) §§ `Alpha`, `Beta` and `Gamma` ``
- **THEN** three `xref` edges target `ref/a.md`'s `Alpha`, `Beta` and `Gamma`

### Requirement: Ignore `§` inside inline code

A `§` that lies inside an inline code span SHALL NOT be read as a reference.
Code spans are found per line as CommonMark does: a backtick run opens a span
that closes at the next backtick run of the same length, and a run with no
such partner is literal text. Every other `§` on the line, including one
written directly against its heading (`` §`Heading` ``), SHALL be parsed.

Verified by: `cargo test -p latt-core --lib adapters::prose_xref inline_code`, and `cargo test -p latt-core --lib adapters::prose_xref double_backtick`, and `cargo test -p latt-core --lib adapters::prose_xref unmatched_backtick`, and `cargo test -p latt-core --lib adapters::prose_xref without_space`

#### Scenario: Marker in code beside a real reference

- **WHEN** a line contains `` The marker is `§`; see § `Real` here. ``
- **THEN** one reference to `Real` is read and the marker is ignored

#### Scenario: Whole reference inside a code span

- **WHEN** a line contains ``` `` see § `Heading` `` ```
- **THEN** no reference is read

### Requirement: Carry forward file context on the same line

When a bare `§` follows a file-qualified `§` on the same line with only
connector text between them, the adapter SHALL resolve against the preceding
file target. When that preceding file did not resolve, the carried-forward
reference SHALL inherit the failure: one `PARSE_ERROR` naming the same missing
file, and no edge — never an edge into an earlier line's file or the current one.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k carry_forward`, and `cargo test -p latt-core --lib adapters::prose_xref carry_forward`

#### Scenario: Bare § inherits preceding file

- **WHEN** a line contains `` `structure.md` § `Act I` and § `Act II` ``
- **THEN** both edges target `structure.md`

#### Scenario: Preceding file does not resolve

- **WHEN** an earlier line resolved `a.md`, and a later line contains
  `` `gone.md` § `Y` / § `Z` `` where `gone.md` is not a discovered file
- **THEN** each of the two references is a `PARSE_ERROR` naming `gone.md`, and
  neither becomes an edge into `a.md`

#### Scenario: A local reference ends the chain

- **WHEN** a line contains `` See `a.md` § `X`. Locally see § `Y` and § `Z`. ``
- **THEN** `Y` and `Z` both target the citing file: the connector before `Z`
  carries `Y`'s local context, not `a.md`

### Requirement: Explain a reference a line break split from its file

A file applies only to a `§` on its own line, so a reflow that leaves the path at
the end of one line and the `§` at the start of the next makes a self-reference.
When such a `§` — nothing but whitespace before it on its line — matches no
heading in the citing file, and the previous line ends with a path the
file-prefix grammar accepts, the adapter SHALL emit one `WRAPPED_REF` info
finding at the reference naming that path, beside the edge. The path SHALL NOT
bind the reference: the edge still targets the citing file, so the core's
`VACANCY` still reports it. Without the finding, the `VACANCY` names the citing
file and points the reader at the heading text instead of the line break.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k wrapped_reference`, and `cargo test -p latt-core --lib adapters::prose_xref wrapped_file`

#### Scenario: Path at the end of the previous line

- **WHEN** `notes.md` has no `X` heading, one line ends with `` `a.md` `` and
  the next begins with `` § `X` ``
- **THEN** the edge targets `notes.md#X`, and one `WRAPPED_REF` info finding at
  the `§` line names `a.md`

#### Scenario: Prose between the path and the sign

- **WHEN** one line ends with `` `a.md` `` and the next reads `` and § `X` ``
- **THEN** no `WRAPPED_REF` is emitted

### Requirement: Path-qualified section IDs

Section node IDs SHALL include the heading ancestry, joined with `::`, to
distinguish headings with the same text under different parents. For example,
`# Top` → `## Parent A` → `### Child` produces node ID
`file.md#Top::Parent A::Child`.

The shipped profile `profiles/prose-xref.yaml` declares this ID shape as
`profile_version` `1.1.0`; the flat `file.md#Heading` shape of `1.0.0` names a
different node for any nested heading. Its `section` `id_pattern` constrains
the file half only (`^[^#]+\.md#.+$`): a heading may contain `#`.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k "duplicate_headings_under_different_parents or shipped_profile or sibling_duplicate"`, and `cargo test -p latt-core --lib adapters::prose_xref path_qualified`

#### Scenario: Duplicate headings under distinct parents produce distinct nodes

- **WHEN** `bob.md` has `### Motivation` under both `## Early Life` and `## Career`
- **THEN** two distinct section nodes exist: `bob.md#Bob::Early Life::Motivation`
  and `bob.md#Bob::Career::Motivation`
- **AND** no `PARSE_ERROR` is emitted for duplicate nodes

Sibling headings with identical text remain duplicate nodes; the core reports
them as `PARSE_ERROR`.

#### Scenario: Shipped profile accepts path-qualified IDs

- **WHEN** `latt validate` runs the shipped profile, version `1.1.0`, over
  `# Top`, `## C# Notes` and `### Issue #42`
- **THEN** no `ID_FORMAT` is reported for `Top`, `Top::C# Notes` or
  `Top::C# Notes::Issue #42`
- **AND** a section in a file named `ref/x#y.md` is reported as `ID_FORMAT`

### Requirement: Match reference text to a heading

Reference text SHALL match a heading when the two are equal, when they are
equal after wrapping emphasis (`**`, `__`, `*`, `_`) is removed from each, or
when the reference equals the heading's text, emphasis removed, before its
first qualifier separator: `: `, ` (`, ` [` or ` — `. The first two rules are
exact matches. When any heading matches exactly, the exact matches are the
candidates and qualifier-prefix matches are not considered; otherwise every
qualifier-prefix match is a candidate. One candidate is the target, and two or
more are `AMBIGUOUS_REF`.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k "without_emphasis or prefix_before_qualifier or qualified_candidates"`

#### Scenario: Emphasis is ignored

- **WHEN** a file has `## *Emphasized*` and a reference reads `` § `Emphasized` ``
- **THEN** the edge targets that section

#### Scenario: Qualified heading matches its prefix

- **WHEN** a file has `## Stage 8: The Mixing Frontier [working]` and a
  reference reads `` § `Stage 8` ``
- **THEN** the edge targets that section

#### Scenario: Two qualified candidates

- **WHEN** a file has `## Twin: One` and `## Twin (Two)` and a reference reads
  `` § `Twin` ``
- **THEN** one `AMBIGUOUS_REF` is emitted and no edge

#### Scenario: Exact heading beats qualified candidates

- **WHEN** a file has `## Arc`, `## Arc: One` and `## Arc (Two)`, and a
  reference reads `` § `Arc` ``
- **THEN** the edge targets the `Arc` section and no `AMBIGUOUS_REF` is emitted
- **AND** two sections both headed `Same` under different parents still make
  `` § `Same` `` an `AMBIGUOUS_REF`

### Requirement: AMBIGUOUS_REF for duplicate headings

When a referenced heading matches two or more sections in the target file, the
adapter SHALL emit one `AMBIGUOUS_REF` warning and no edge. References match
against leaf heading text only — authors cannot use path-qualified syntax
(e.g., `§ \`Parent::Child\``) to disambiguate. Arrow-aware resolution
(`§ \`Parent\` → \`Child\``) is a candidate follow-on.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k ambiguous_ref`

#### Scenario: Duplicate heading produces warning

- **WHEN** `bob.md` has two `## Duplicate Section` headings and a ref targets it
- **THEN** one `AMBIGUOUS_REF` finding is emitted and no `xref` edge

### Requirement: PARSE_ERROR for bare § without backtick delimiters

When `§` is followed by text without backtick delimiters, or by nothing before
the end of the line or the next `§`, the adapter SHALL emit one `PARSE_ERROR`.
Two adjacent signs, `§§`, are one plural marker and get at most one such finding.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k bare_undelimited`, and `cargo test -p latt-core --lib adapters::prose_xref empty_section_sign`

#### Scenario: Undelimited § is a parse error

- **WHEN** a line contains `§ Bare Reference`
- **THEN** one `PARSE_ERROR` is emitted

#### Scenario: § with no target is a parse error

- **WHEN** a `§` is followed only by the end of the line or by another `§` after
  whitespace
- **THEN** one `PARSE_ERROR` "§ with no target" is emitted for each such `§`

#### Scenario: An empty plural marker is one parse error

- **WHEN** a line ends with `§§`
- **THEN** one `PARSE_ERROR` "§ with no target" is emitted

### Requirement: Unresolved references produce edges for core VACANCY

When a heading does not exist in the target file, the adapter SHALL emit an
`xref` edge with the unresolved target ID so the core fires `VACANCY`.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_prose_xref.py -k unresolved_ref`

#### Scenario: Missing heading becomes a VACANCY edge

- **WHEN** a ref targets `alice.md` § `Nonexistent Heading`
- **THEN** an `xref` edge with target `characters/alice.md#Nonexistent Heading`
  is emitted
- **AND** the core validates it as `VACANCY`
