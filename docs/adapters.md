# Adapters

An adapter turns one register format into the document the core reads: nodes, edges,
pathways, and a finding for each thing it could not read. The core builds the graph and
checks it against the profile. The adapter never sees the rules; the core never opens
your files.

- **Built in:** `md`, `toml` and `prose-xref` run inside `latt`.
- **External:** a program run as `<program> --profile <file> --target <dir>` that writes
  the document to stdout: `latt-adapter-entomologist`, the Python `openspec` adapter,
  and any adapter you write.

The profile's `adapter:` block selects and configures one; its other keys belong to the
adapter. For the rest of a profile, see [profiles.md](profiles.md).

On this page: [Native adapters](#native-adapters) · [Shared rules](#shared-rules) ·
[Markdown table](#markdown-table-adapter) · [TOML](#toml-adapter) ·
[Entomologist](#entomologist-adapter) · [Prose cross-reference](#prose-cross-reference-adapter) ·
[OpenSpec](#openspec-adapter) · [Suggestion sidecar](#suggestion-sidecar) ·
[Writing your own adapter](#writing-your-own-adapter)

## Native adapters

`md`, `toml` and `prose-xref` exist only inside the `latt` binary; `adapters/md` runs
`latt adapter md`. A build without the `native-adapters` feature has none of them.

```sh
latt validate --profile profiles/md.yaml --adapter md --target /path/to/repo
latt validate --profile profiles/toml.yaml --adapter toml --target /path/to/repo
latt validate --profile profiles/prose-xref.yaml --adapter prose-xref --target /path/to/repo
```

Or set `adapter.native` in the profile and drop `--adapter`:

```yaml
adapter:
  native: md
  paths:
    files: ["*.md"]
```

```sh
latt validate --profile profiles/md.yaml --target /path/to/repo
```

Resolution:

- Precedence: `--adapter` > `adapter.native` > exit 2.
- An `--adapter` value matching a native name runs in-process; any other is an external
  program name or path.
- A bare name is looked up on PATH, then retried as `latt-adapter-<name>` (the name a
  release install uses), so `--adapter entomologist` runs `latt-adapter-entomologist`. A
  failure names both spellings.
- A path such as `./md` runs that program even if its name matches a native adapter, and
  gets no retry.
- `adapter.native` is native-only and does not fall back to PATH.
- A spawn error names the program, the OS error and the native adapters.
- A fuse manifest's `adapter:` takes the same native names ([Manifest](fuse.md#manifest)).

`entomologist` runs as a subprocess because it is a bounded run of `git`. In a checkout,
the shim `./adapters/<name>` looks on PATH, then `$CARGO_TARGET_DIR/debug`, then
`target/debug`, and exits 127 naming all three if none has it.

### Running an adapter by hand

Adapters take the *resolved* profile, the JSON form of your YAML:

```sh
latt show --profile profiles/md.yaml > /tmp/p.json
latt adapter md --profile /tmp/p.json --target .
```

The same holds for `latt-adapter-entomologist` and `adapters/<name>`. Given the YAML, an
adapter exits 2 with `could not load profile p.yaml: expected ident at line 1 column 2`;
the profile itself is fine.

### Timeouts

A wall-clock bound kills, on expiry, an external adapter run, the entomologist adapter's
`git` commands, the `git archive` behind `query diff`, and the `git log` behind a
`browse` provenance preview. Unbounded: programs started from `browse` (owner actions,
`$EDITOR`) and `fuse`'s per-source `latt trace` as a whole; an external adapter that
`fuse` runs uses its source profile's key.

Both keys take a positive integer of seconds; any other value refuses the profile (exit
2). Native adapters read neither.

| Key | Default | Read by | Covers | On expiry |
|---|---|---|---|---|
| `timeout_seconds` | 300 | the core | the adapter's whole run; also each `git archive \| tar` pair of `query diff` | exit 2 naming the program (or revision) and the bound; nothing ingested |
| `command_timeout_seconds` | 60 | the entomologist adapter | one `git` subprocess; all blobs come from one `cat-file --batch`, so the whole read | `PARSE_ERROR` naming the command and the bound; the adapter continues, exit 0 |

```yaml
adapter:
  timeout_seconds: 600
  command_timeout_seconds: 120
```

## Shared rules

These hold for md, TOML and prose-xref unless a section says otherwise.

- **Unknown keys:** an `adapter:` key the adapter does not read, at any depth, is exit 2
  naming the key. `native` and `timeout_seconds` belong to the core and pass.
- **Profile errors** (md, TOML) name your `--profile` path, the adapter, and serde's field
  and position: ``could not load profile profiles/x.yaml for adapter md: missing field `id_column` at line 1 column 166``.
- **Unreadable paths:** a matching symlink to nothing, or a matching path whose name is
  not UTF-8, is a `PARSE_ERROR` and is skipped. Symlinked directories are not followed.
- **No matching files:** a readable target where nothing matches `paths.files` gets a
  `NO_MATCHING_FILES` info finding naming the globs and the target.
- **Non-UTF-8 lines:** each is a `PARSE_ERROR` at that line naming the column of the
  first bad byte and showing the line with each such byte as `\xNN`. Each section says
  what happens to the rest.
- **`id_names`:** a node whose kind declares `id_names`
  ([profiles.md](profiles.md#node-kinds)) is resolved against the target; one naming
  nothing gets `SOURCE_MISSING`.

```text
ERROR PARSE_ERROR 5.0/en/0x14-V5-File-Handling.md:56 5.0/en/0x14-V5-File-Handling.md: line is not valid UTF-8 at column 18, so it was not read: | **5.9.9** | caf\xE9 | 1 |
```

Globs in `paths.files` (md, TOML) are relative to the target:

| Pattern | Matches |
|---|---|
| `*` | within one path segment |
| `**` | across segments; `**/*.md` also matches at the target root |
| `?` | one character |
| anything else, `[` included | itself (no classes or braces) |

## Markdown table adapter

Rows are the only nodes and `edge_columns` the only edges. Headings only dispatch tables;
prose is not read.

**Single-table form:** one `table` schema for every table in every matched file. The
profile must declare exactly one node kind, or it fails to load.

```yaml
adapter:
  paths:
    files: ["*.md"]
  table:
    id_column: "ID"
    column_map:
      "Description": summary
    edge_columns:
      "Traces To": traces_to
```

**Multi-table form:** `tables` dispatches each table by its nearest preceding heading (any
level, text without the `#`s); the first matching entry wins.

```yaml
adapter:
  paths:
    files: ["**/*.md"]
  tables:
    - heading: '^Domain [0-9]+$'
      kind: requirement
      id_column: "ID"
      column_map:
        "Description": summary
        "Status": status
      edge_columns:
        "Traces To": traces_to

    - heading: '^Stakeholders$'
      kind: stakeholder
      id_column: "ID"
      column_map:
        "Name": name
        "Role": role
      edge_columns: {}
```

| Key | Required | Default | Meaning |
|---|---|---|---|
| `heading` | in `tables` | | regex over the heading text |
| `kind` | in `tables` | | node kind of the rows; must be in `node_kinds` |
| `id_column` | yes | | header of the ID column |
| `column_map` | no | empty | header → string attr (`""` when empty) |
| `edge_columns` | no | empty | header → edge kind |
| `strip_emphasis` | no | `false` | strip wrapping emphasis from IDs and edge targets |

**IDs** are the trimmed cell text as written: `**1.1.1**` stays `**1.1.1**`, and edge
cells must match it. `strip_emphasis: true` removes wrapping `**`, `__`, `*`, `_` or
backtick runs, repeatedly (`` **`1.1.1`** `` → `1.1.1`). It keeps empty pairs (`****`),
a pair whose marker recurs inside at its own length (`**a** b **c**`), links, and attr
cells. Turning it on changes IDs, so bump `profile_version` minor.

**Attrs** are strings: declare them `type: string`, since `type: int` fails `ATTR_TYPE`
on every row (`expected type 'int', got string`).

**Edge cells** hold comma-separated targets, trimmed, empties skipped. Commas inside
`[...]` or `(...)` do not split, so `[R-2, R-3](http://x)` is one target, kept as written.
A cell naming its own row is a self-edge with no finding; that includes the ID column
listed in `edge_columns` to read its link.

**Not read:**

- A row with fewer cells than its header is a `PARSE_ERROR` ("table row has fewer
  columns than its header") and no node, and so is one with more. Lattice does not pad
  short rows as GitHub does.
- A table without the ID column, prose tables included, is a `PARSE_ERROR` ("configured
  ID column … was not found in table headers"); so is an unmatched table in multi-table
  form. Fence a prose table or narrow `paths.files` to keep it out.
- Tables and headings in fenced code (```` ``` ```` or `~~~`). As in CommonMark, a
  backtick run whose info string holds a backtick (```` ```a`b ````) opens no fence. A
  fence open at end of file is a `PARSE_ERROR` at its opening line.
- Blockquote markers (`>`, `>>`) are stripped first, so quoted tables count.
- A non-UTF-8 line: its row is skipped, the rest of the file is read. If it is a table's
  header or separator, or (multi-table) the heading above it, the table is skipped and a
  second `PARSE_ERROR` at the table gives the line and row count.

When a glob's leading directories repeat the target's (`--target docs` with
`docs/*.md`) and dropping them would match a file already walked beneath the target,
`NO_MATCHING_FILES` also names the directory above as the target to use.

`profiles/md-sources.yaml` shows `id_names` with `cargo`, `pytest`, `gotest` and `file`
tables.

## TOML adapter

`tables` maps array-of-tables names to node kinds:

```yaml
adapter:
  paths:
    files: ["registers/*.toml"]
  id_prefix: file_stem
  tables:
    item:
      kind: item
      id_key: id
      key_map:
        id: id
        stage: stage
      edge_keys:
        register: belongs_to
        refs: references
```

| Key | Required | Default | Meaning |
|---|---|---|---|
| `paths.files` | yes | | register file globs |
| `tables` | yes | | table name → entry (below) |
| `mode` | no | `tables` | `directory`: each file's root table is one record, described by the single `tables` entry (more is exit 2) |
| `id_prefix` | no | none | `file_stem`: IDs become `<file-stem>/<id>` |
| `header` | no | none | one singleton table per file as a node: `table`, `kind`, `id` (`file_stem` uses the filename), `key_map` (default empty) |
| `pathway` | no | none | ordering pathway from a file: `source_file`, `name`, `order_key`, `current_key` as dotted paths |

`tables` entry:

| Key | Required | Default | Meaning |
|---|---|---|---|
| `kind` | yes | | node kind |
| `id_key` | yes | | key holding the ID |
| `id_prefix` | no | none | `kind`: IDs become `<kind>/<id>`, or `<file-stem>/<kind>/<id>` with `file_stem` |
| `key_map` | no | empty | key → attr ([key_map values](#key_map-values)) |
| `edge_keys` | no | empty | key → edge kind or table ([edge keys](#edge-keys)) |

Node IDs share one namespace across tables and kinds: a `language` and a `grammar` both
named `ada` give a `PARSE_ERROR` "duplicate node", and the second and its edges are not
read. `id_prefix: kind` keeps them apart. Two directory-mode registers are two profiles,
composed with `latt fuse` ([fuse.md](fuse.md)).

`edge_keys` and `key_map` keys (header included) are looked up as written, then as a
dotted path: `people.leads` reads `leads` in `[people]`. A path blocked by a non-table is
a `PARSE_ERROR`; one with a missing part is an absent key.

### Edge keys

An entry is an edge kind, or a table with `kind` and optionally:

| Key | Meaning |
|---|---|
| `table` | `tables` entry the targets belong to; the target ID is built as that table builds IDs in the same file, prefixes included. Without it the value is the ID as written. An unknown name is exit 2 |
| `fallback` | key to read when the edge key is absent |
| `entry_key` | field holding the target in a table entry: `entry_key: name` reads `["a", { name = "b" }]` as two edges |

Helix's `languages.toml`, where a language without `grammar` uses the grammar of its own
name and file types and servers appear bare and as tables:

```yaml
adapter:
  native: toml
  paths:
    files: ["languages.toml"]
  tables:
    language:
      kind: language
      id_key: name
      id_prefix: kind
      key_map:
        file-types: { scalars: file_types, tables: { glob: file_globs } }
        language-servers: { scalars: language_servers, tables: { name: language_servers } }
      edge_keys:
        grammar: { kind: uses_grammar, table: grammar, fallback: name }
    grammar:
      kind: grammar
      id_key: name
      id_prefix: kind
```

The value is a string (one edge) or an array (one per string). Each other entry is a
`PARSE_ERROR` at its line, shown as written; the strings beside it are still read:

```
ERROR PARSE_ERROR records/a.toml:3 records/a.toml: item 'DEV-001' has edge key 'people.members' entry 2 of 4: { github = "x", roles = [] } is a table, not a target string; it was not read
```

An edge key reads a value, so targets written as subtables (`[people.alice]`) cannot be
declared.

### key_map values

A scalar (string, integer, float, boolean, date-time) or an array of scalars, which
becomes a `list` attr. A table value is a `PARSE_ERROR` naming the key; the row is still
read. An array with an array or table entry sets no attr, and each such entry is an
`UNREAD_ENTRIES` warning at its line, as written (cut at 120 characters, joined onto one
line):

```
WARNING UNREAD_ENTRIES languages.toml:421 languages.toml: item 'language/toml' has mapped key 'file-types' entry 4 of 13: { glob = "Cargo.lock" } is a table, not read; the attr is not set
```

Edge arrays differ: their strings become edges and other entries are errors, since a
meant edge is missing.

**Shapes** split a mixed array: `scalars` names the attr for scalar entries, `tables`
maps a table-entry field to an attr. Each attr is always set (`[]` if nothing matches);
an unclaimed entry is an `UNREAD_ENTRIES` warning. In the Helix example,
`file-types = ["toml", { glob = "Cargo.lock" }]` sets `file_types: ["toml"]` and
`file_globs: ["Cargo.lock"]`. Shapes naming one attr fill it in order:
`language-servers = ["biome", { name = "ts-ls", except-features = ["format"] }]` sets
`language_servers: ["biome", "ts-ls"]`. Other fields of a table entry are not read, as
with any key `key_map` does not name.

- A claimed entry that cannot be read (non-scalar field, float JSON cannot hold):
  `PARSE_ERROR`, attr unset.
- A non-array value: `PARSE_ERROR`.
- A shapes table with neither key, or another key: exit 2.

### Parsing and locations

Invalid TOML is a `PARSE_ERROR` naming the line; other files are still read. A file with
a non-UTF-8 line is read as nothing.

Findings sit where things were written: a node at its `[[table]]` header, an edge at its
target (array entries each at their line), a key finding at the key's value, including
the core's attr type and enum checks. A directory-mode node is at line 0 (`file.toml:0`).

`id_names` applies to table, directory-mode and header nodes. Example:
`profiles/toml.yaml`.

## Entomologist adapter

Run as `--adapter ./adapters/entomologist` from a checkout or `--adapter entomologist`
from an install. It reads the `entomologist-data` orphan branch of a git-backed issue
tracker through `git ls-tree` and `cat-file --batch`. The format is fixed; its only key
is `command_timeout_seconds` ([Timeouts](#timeouts)).

| Item | Source |
|---|---|
| issue | `entomologist-data:<id>`, line 0 (whole file) |
| dependency edge | `entomologist-data:<id>/dependencies/<dep-id>`, line 0 |
| attr | its file, line 1, e.g. `entomologist-data:<id>/state:1` |

## Prose cross-reference adapter

Run as `--adapter prose-xref` (native) or `--adapter ./adapters/prose-xref`. It parses
`§` cross-references: `` `file.md` § `Heading` ``.

```yaml
adapter:
  paths:
    files: ["**/*.md"]
    content_dirs: ["characters", "outline", "ref", "worldbuilding"]
```

| Key | Required | Meaning |
|---|---|---|
| `paths.files` | yes | file globs |
| `paths.content_dirs` | yes | directories to scan; only files under them are read |

Output: `document` and `section` nodes and `xref` edges. A dangling reference is an edge
whose target the core reports as `VACANCY`. Section IDs are `file.md#Parent::Child` (from
profile version `1.1.0`).

| Finding | When |
|---|---|
| `AMBIGUOUS_REF` | two or more headings match; no edge |
| `WRAPPED_REF` (info) | a bare `§` finds no heading and the previous line ends with a path; join the lines |
| `PARSE_ERROR` | `§` without backticks or with nothing after it; a path leaving the target with `..` ("target outside the register"); an unclosed fence (at its opening line); a missing `content_dirs` directory; an unreadable file |
| `NO_MATCHING_FILES` | nothing under the directories matches; names globs and directories |

**Syntax:**

- A link may name the file: `` [text](file.md) § `Heading` `` (fragment and title ignored).
- The space before `§` may be non-breaking.
- A bare `§` after a file-qualified one on the same line uses that file; on a new line it
  means the current file.
- `` §§ `A`, `B` and `C` `` reads several headings in one file, separated by commas,
  `and` or `or`.
- A `§` in inline code (`` `§` ``) is plain text.

**Matching:** text matches a heading exactly after stripping wrapping `**`, `__`, `*`,
`_`, or matches the part before a qualifier (`: `, ` (`, ` [`, ` — `), so `` § `Stage 8` ``
matches `## Stage 8: The Mixing Frontier [working]`. An exact match wins over qualified
ones. A heading with inline code cannot be cited: `` § `Use `x` here` `` ends at the
inner backtick and is `VACANCY`.

**Unread input:** only fenced code hides references; indented lines are read (fence any
example). Fences open at any indentation. Other directories are still read when one is
missing, a reference into an unreadable file reports its target as unreadable, and a
non-UTF-8 line is read as blank.

To tolerate optional directories, downgrade the finding; this affects every
`PARSE_ERROR`, so prefer listing only existing directories:

```yaml
validations:
  - PARSE_ERROR: { severity: warning }
```

A register with no references validates at exit 0, with no edges (`latt query counts`:
`edge xref 0`).

## OpenSpec adapter

`adapters/openspec` (Python) reads `openspec/specs/*/spec.md` and `Requirement:`
comments in tests.

```yaml
adapter:
  paths:
    spec_dir: "openspec/specs"
    tests:
      - glob: "crates/*/tests/*.rs"
        dialect: rust
      - glob: "tests/*.py"
        dialect: python
```

| Key | Required | Meaning |
|---|---|---|
| `paths.spec_dir` | yes (else exit 2) | the specs directory |
| `paths.tests` | no | list of `glob` + `dialect` (`rust`, `python`, `typescript`); absent or empty means specs only, with no `test` nodes or test findings |

An unknown dialect exits with one stderr line naming it and the supported ones, and no
traceback. For
another language, write an adapter emitting `test` nodes and `verifies` edges and
compose it with `latt fuse`.

Requirement IDs are `<capability>/<title>`, the capability being the spec directory, so
one title in two capabilities is two requirements. A
citation is a whole-line comment (`//` in Rust and TypeScript, `#` in Python) and binds
every test after it until the next:

```rust
// Requirement: cli/Exit codes
#[test]
fn a_clean_register_exits_zero() { /* … */ }
```

```python
# Requirement: cli/Exit codes
def test_a_clean_register_exits_zero():
    ...
```

```ts
// Requirement: cli/Exit codes
describe("exit codes", () => {
  it("is zero on a clean register", () => { /* … */ });
});
```

**TypeScript:** a test is an `it(...)` or `test(...)` call opening a line, modifiers
allowed (`.skip`, `.skipIf(cond)`, `.each(table)`). Its ID joins the path, enclosing
`describe` names and its name with `::` (`<path>::exit codes::is zero on a clean
register`). Template literals and block comments are text. A call with a non-literal name
is a warning `PARSE_ERROR` and is skipped. `profiles/openspec.yaml` admits these IDs: a
path, `::` and any name without surrounding whitespace.

**Checks:** the dialect applies to every matched file, whatever its extension.

- A glob matching files but no tests is one warning `PARSE_ERROR` (one file with no
  test is not reported):

  ```
  WARNING PARSE_ERROR .:0 230 files matched test glob 'test/**/*.ts' and none held a rust test item
  ```

- A missing `spec_dir` is an error `PARSE_ERROR`; one with no `<capability>/spec.md` is a
  warning.
- A non-UTF-8 line drops the requirement holding it, with a second `PARSE_ERROR` at its
  heading; other requirements are kept. In a test file, it drops the whole file.

## Suggestion sidecar

`adapters/latt-suggest` (Python, stdlib only) ranks edges the register does not declare
and writes a suggestion document to stdout for `latt validate --suggestions <file>`. It
reads an adapter's output and writes only stdout and stderr.

| Option | Default | Meaning |
|---|---|---|
| `--profile` | | resolved profile JSON from `latt show`; YAML is refused (`failed to read profile ...`, exit 2) |
| `--interface` | `-` (stdin) | an interface document; only `nodes` and `edges` are read, so `latt fuse --format json` output works |
| `--edge-kind` | | edge kind to rank; must have `allowed` pairs |
| `--backend` | `none` | `none` or `ollama` |
| `--model` | `nomic-embed-text` | the Ollama embedding model |
| `--host` | `http://localhost:11434` | the Ollama server |
| `--top-k` | 5 | targets kept per source |

Sources are nodes of an allowed source kind without an outgoing edge of that kind;
targets are all nodes of an allowed target kind; only allowed pairs are proposed. Ranked
text is `text_attrs`, else `summary_attr` ([profiles](profiles.md)); nodes with neither
are excluded and counted on stderr.

`unranked` counts candidate sources no suggestion names. With `--backend none`,
`suggestions` is empty, `unranked` counts every candidate, and `--top-k` does nothing.
With a backend it counts textless sources and those with no legal target. A positive
count renders as one `SUGGESTION_UNRANKED` hint.

Exit 0 with a document; 2 when the profile, interface document or edge kind cannot be
read, or the backend is unreachable.

### Suggestion document

What `validate --suggestions` reads, from any producer:

| Key | Required | Value |
|---|---|---|
| `suggestion_version` | yes | `"1.0"` or `"1.1"`; others refused |
| `producer` | yes | program name and version |
| `suggestions` | no | entries in render order; absent is none, `null` refused |
| `unranked` | no | non-negative integer; absent is 0 |

Entries hold `src`, `tgt`, `kind`, `basis` (strings) and `score` (number). An unknown ID
renders as `SUGGESTION_UNRESOLVED`. A missing required key, a missing entry field or a
non-numeric `score` refuses the whole document (exit 2).

```json
{
  "suggestion_version": "1.1",
  "producer": "latt-suggest 0.3.0",
  "suggestions": [
    {"src": "REQ-2", "tgt": "SPEC-4", "kind": "satisfies", "score": 0.81,
     "basis": "cosine 0.8100; model nomic-embed-text"}
  ],
  "unranked": 0
}
```

### Ranking over a fused graph

`latt show` refuses a fuse profile (`missing required key 'node_kinds'`). Write a profile
naming the source-qualified kinds (`<source>:<kind>`, [fuse](fuse.md)), their text attrs
and the fuse edge kind, resolve it, and pass the fused JSON as `--interface`.
Suggestions then name IDs like `markdown:REQ-1`.

```yaml
name: fused
profile_version: "1.0.0"
node_kinds:
  "markdown:requirement":
    id_pattern: "^markdown:"
    summary_attr: summary
    attrs:
      summary: {type: string}
  "toml:item":
    id_pattern: "^toml:"
    summary_attr: title
    attrs:
      title: {type: string}
edge_kinds:
  traces_to:
    allowed:
      - ["markdown:requirement", "toml:item"]
```

## Writing your own adapter

Any program that takes `--profile <resolved-profile.json>` and `--target <dir>` and
prints an interface document is an adapter:

```json
{
  "interface_version": "1.4",
  "nodes": [
    {"id": "REQ-1", "kind": "req", "attrs": {"text": "The pump stops on overpressure"},
     "provenance": {"file": "requirements.md", "line": 4}},
    {"id": "T-1", "kind": "test", "attrs": {},
     "provenance": {"file": "tests.md", "line": 2}}
  ],
  "edges": [
    {"src": "T-1", "tgt": "REQ-1", "kind": "verifies",
     "provenance": {"file": "tests.md", "line": 2}, "declared_by": 1}
  ],
  "pathways": [],
  "findings": [
    {"severity": "error", "code": "PARSE_ERROR",
     "message": "row 3: expected 3 cells, found 2",
     "provenance": {"file": "requirements.md", "line": 6}, "node_id": null}
  ]
}
```

The core checks kinds and IDs against the profile. Your findings are kept as written,
even with a code the profile does not list.

### The contract

**Input.** The `--profile` file is the core's resolved profile as JSON: the whole profile
as loaded (raw `id_pattern`s, edge kinds, validations, pathways, the `adapter:` block
unchanged) plus `resolved_schema`, now `"1"`. Exit non-zero if it is missing or a value
you do not read. `latt show --profile <file>` prints it.

**Output.** One UTF-8 JSON object on stdout with exactly these keys; any other key, at
any level except inside `attrs`, is exit 2:

| Object | Keys |
|---|---|
| document | `interface_version`, `nodes`, `edges`, `pathways`, `findings` (absent arrays read as empty) |
| node | `id`, `kind`, `attrs`, `provenance`, `attr_provenance` (optional) |
| edge | `src`, `tgt`, `kind`, `attrs` (optional), `provenance`, `declared_by` (optional) |
| pathway | `name`, `order`, `current` |
| finding | `severity`, `code`, `message`, `provenance`, `node_id`, `state` (optional) |
| provenance | `file`, `line` |

- **`interface_version`:** `1.4`; `1.0` to `1.3` still ingest. The legacy
  `contract_version` is read only when `interface_version` is absent; a present one
  always governs, even if malformed or unsupported.
- **`findings`** was `issues` before 1.2; either is read, both is refused.
- **`attrs`:** any JSON object, any depth. Encode values JSON has no type for.
- **`attr_provenance`:** attr name → provenance of its value when it differs from the
  node's (`{"scope": {"file": "languages.toml", "line": 1173}}`). `ATTR_TYPE`,
  `ATTR_ENUM` and `ATTR_LIST_ITEMS` findings land there. Each name must be in `attrs`.
- **`line`:** one-based, or 0 for no exact line; 0 to 4294967295, else exit 2 (nodes,
  edges, findings, `attr_provenance`).
- **`file`:** relative to the target when inside it, else as written (`<profile>`).
- **`severity`:** `error`, `warning`, `info` or `hint`; profile overrides apply after.
- **Repeated node IDs:** the first wins; each later one becomes a finding naming both
  places. Emit every occurrence.
- **`declared_by`:** index in `nodes` of the entry whose row declared the edge, which may
  differ from `src`'s entry; record it when appending, since an ID lookup finds the first
  occurrence. If that entry is a repeat, the core drops it and its edges, and its finding
  counts them. An edge without `declared_by` is always read, from whichever node holds
  `src`, and duplicate findings for that `src` count it as read. A value that is not a
  valid index is exit 2.
- **Pathways:** positions unique, `current` among them; otherwise emit `PATHWAY_INVALID`
  and omit the pathway.
- Strip one leading UTF-8 BOM before parsing a file.

**Exit.** Exit 0 whenever you wrote a document; a file you cannot read, decode or parse
is a finding, and the rest is still read. Exit non-zero only when you cannot produce a
document. The core exits 2 and ingests nothing when the program cannot start, exits
non-zero, writes non-UTF-8 or an invalid document, or exceeds `timeout_seconds`
([Timeouts](#timeouts)).
