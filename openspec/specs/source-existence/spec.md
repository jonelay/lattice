# source-existence Specification

## Purpose
Reports a register row whose ID names a file, or a definition in a source file, that
does not exist in the target, so a renamed or deleted source surfaces as a finding
instead of a stale row.
## Requirements
### Requirement: Declared source addresses resolve against the target
An adapter that honours `id_names` SHALL resolve every node it declares under a kind
carrying `id_names`, after building the rest of its document. Nodes named only as edge
targets SHALL NOT be resolved; a dangling one stays a `VACANCY`. Resolution SHALL be
relative to the `--target` directory and SHALL read files only — nothing is imported,
compiled, executed or collected.

A node that does not resolve SHALL produce one `SOURCE_MISSING` finding at warning
severity, carrying the node's ID as `node_id` and the node's provenance, with message
`'<id>' does not resolve: <reason>`. The node SHALL still be emitted. These findings
SHALL follow the adapter's other findings, in the order the nodes were declared. A
profile severity override and `SUPPRESS` SHALL apply to `SOURCE_MISSING` by code string,
as to any adapter code.

Verified by: `cargo test -p latt-adapter --locked source`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py -k sources`

#### Scenario: Missing file is reported against its row
- **WHEN** a `file` kind declares node `docs/gone.md` at `REGISTER.md:7` and the target
  has no such file
- **THEN** the document carries a warning `SOURCE_MISSING` with `node_id`
  `docs/gone.md`, provenance `REGISTER.md:7`, and message
  `'docs/gone.md' does not resolve: no file at 'docs/gone.md'`, and the node is still present

#### Scenario: Present file is silent
- **WHEN** a `file` kind declares a node naming a regular file in the target
- **THEN** no finding is emitted for it

#### Scenario: Edge-only reference is not resolved
- **WHEN** a requirement row's edge cell names `tests/test_x.py::test_gone` and no row
  declares that node
- **THEN** no `SOURCE_MISSING` is emitted for it, and validation reports `VACANCY`

#### Scenario: Kind without id_names is not checked
- **WHEN** a kind declares no `id_names`
- **THEN** none of its nodes is resolved, whatever its IDs look like

### Requirement: An address is a path and an optional symbol chain
A file address SHALL be the whole ID. A symbol address SHALL be read with the declared
`separator` and, when declared, `params`:

1. When `params` is declared and the ID ends with its closer, everything from the first
   opener after the first `separator` SHALL be dropped. A parameter value may contain
   the separator, so this happens before splitting.
2. The rest SHALL split on `separator` into a path and a symbol chain of zero or more
   names. A path with no chain names only its file.
3. Shape checks SHALL apply before any filesystem access: an empty segment, a chain
   longer than the language reaches, and a path without the language's file extension
   each fail with their reason (`empty name in node ID`,
   `<language> addresses reach at most <n> names`,
   `a <language> address names a '<ext>' file`).
4. The path SHALL then resolve as a file address, and the chain against the language's
   definitions in that file.

Verified by: `cargo test -p latt-adapter --locked source`

#### Scenario: Parameter value containing the separator
- **WHEN** a pytest kind declares node `tests/test_a.py::test_new[a::b]` and
  `test_new` is defined at top level
- **THEN** it resolves

#### Scenario: Empty segment
- **WHEN** a cargo kind declares node `tests/foo.rs::::t`
- **THEN** `SOURCE_MISSING` reads `'tests/foo.rs::::t' does not resolve: empty name in node ID`

#### Scenario: Wrong file extension
- **WHEN** a cargo kind declares node `tests/foo.py::t`
- **THEN** `SOURCE_MISSING` reads `'tests/foo.py::t' does not resolve: a rust address names a '.rs' file`

### Requirement: A file address names a regular file inside the target
A path SHALL be `/`-separated and relative to the target. It SHALL resolve when it names
a regular file, following symlinks whose resolution stays inside the target. A path
that is absolute, has a `..` component, or resolves through a symlink to a location
outside the target SHALL NOT resolve, whether or not that location exists. The reasons
SHALL be:

- `no file at '<path>'` — nothing exists there, or a dangling symlink
- `'<path>' is outside the target` — absolute, `..`, or a symlink escaping the target
- `'<path>' is not a regular file` — a directory or other non-file
- `'<path>' could not be read: <error>` — any other I/O failure, with the system's error text

Verified by: `cargo test -p latt-adapter --locked source`

#### Scenario: Climbing out of the target
- **WHEN** a `file` kind declares node `../secrets.txt` and that file exists beside the target
- **THEN** `SOURCE_MISSING` reads `'../secrets.txt' does not resolve: '../secrets.txt' is outside the target`

#### Scenario: Symlink escaping the target
- **WHEN** `docs/link.md` in the target is a symlink to a file outside the target
- **THEN** node `docs/link.md` does not resolve, with reason `'docs/link.md' is outside the target`

#### Scenario: Directory is not a file
- **WHEN** a `file` kind declares node `docs` and `docs` is a directory
- **THEN** `SOURCE_MISSING` reads `'docs' does not resolve: 'docs' is not a regular file`

### Requirement: Languages define where a definition counts
Each language SHALL fix its file extension, its containers, how deep a chain reaches,
and where a definition counts. Text that only looks like a definition — inside a
string, a comment, or a nested body — SHALL NOT count. No language requires a name
prefix or a test marker; that is the kind's `id_pattern`'s job.

- **`python`** (`.py`): a chain is `name` or `Class::name`, at most two names. `name`
  counts as a `def`, `async def` or `class` at module top level. `Class` counts as a
  top-level `class`, and `name` within it as a `def` or `async def` directly in its
  body. A definition inside an `if`, `try`, function or nested class does not count,
  matching `adapters/_python_tests.py`. Decorators do not affect counting.
- **`rust`** (`.rs`): a chain is `mod::…::name`, any depth. Each `mod` counts as an
  inline `mod <name> { … }` item in the file or in the enclosing inline module, and
  `name` counts as a `fn` item at that module level, whatever its qualifiers (`pub`,
  `async`, `const`, `unsafe`, `extern`) or attributes. An `fn` inside a function body,
  an `impl` or `trait` block, or a macro invocation or definition does not count. A
  module declared out of line (`mod tests;`) is not followed: its file is its own path.
- **`go`** (`.go`): a chain is one name. It counts as a top-level `func <name>`, generic
  or not. A method (`func (r T) name`), a function literal, and a subtest
  (`t.Run("…")`, named only at run time) do not count.

A chain that does not resolve SHALL report the shortest prefix of it the file lacks,
joined by the separator. A missing container reads `'<path>' defines no <noun>
'<prefix>'`, where the noun is `module` for rust and `class` for python (go has no containers), and a missing
final name reads `'<path>' defines no '<prefix>'`.

Verified by: `cargo test -p latt-adapter --locked source`, and
`.venv/bin/python -m pytest tests/test_adapter_md.py -k "sources or differential"`

#### Scenario: Renamed Python test
- **WHEN** node `tests/test_a.py::test_old` is declared under `pytest` and the file
  defines `test_new` but not `test_old`
- **THEN** `SOURCE_MISSING` reads
  `'tests/test_a.py::test_old' does not resolve: 'tests/test_a.py' defines no 'test_old'`

#### Scenario: Missing class reports the class
- **WHEN** node `tests/test_a.py::TestGone::test_b` is declared and no top-level class `TestGone` exists
- **THEN** the reason is `'tests/test_a.py' defines no class 'TestGone'`

#### Scenario: Python method of a top-level class
- **WHEN** `tests/test_a.py` defines `class TestGroup:` with a decorated `async def test_b`
- **THEN** node `tests/test_a.py::TestGroup::test_b` resolves

#### Scenario: Python nested class is reported, not passed
- **WHEN** node `tests/test_a.py::Outer::Inner::test_x` is declared
- **THEN** the reason is `python addresses reach at most 2 names`

#### Scenario: Python definition inside a string or conditional
- **WHEN** `def test_ghost` appears only inside a triple-quoted string, and `def test_cond`
  only inside a module-level `if` block
- **THEN** neither `tests/test_a.py::test_ghost` nor `tests/test_a.py::test_cond` resolves

#### Scenario: Rust integration test
- **WHEN** node `crates/x/tests/schema.rs::loads_profile` is declared under `cargo` and
  the file has a top-level `#[test] fn loads_profile()`
- **THEN** it resolves

#### Scenario: Rust unit test in an inline module
- **WHEN** `crates/x/src/lib.rs` holds `#[cfg(test)] mod tests { #[test] fn splits() {} }`
- **THEN** node `crates/x/src/lib.rs::tests::splits` resolves, and
  `crates/x/src/lib.rs::splits` reports `'crates/x/src/lib.rs' defines no 'splits'`

#### Scenario: Renamed Rust module
- **WHEN** node `crates/x/src/lib.rs::unit::splits` is declared and the file's inline module is `tests`
- **THEN** the reason is `'crates/x/src/lib.rs' defines no module 'unit'`

#### Scenario: Rust fn inside an impl, a body or a macro does not count
- **WHEN** `fn ghost` appears only as a method in an `impl` block, inside another
  function's body, and inside a `macro_rules!` definition
- **THEN** `<path>::ghost` does not resolve

#### Scenario: Go test function
- **WHEN** node `pkg/store/store_test.go::TestPut` is declared under `gotest` and the
  file has a top-level `func TestPut(t *testing.T)`
- **THEN** it resolves

#### Scenario: Go method or subtest does not count
- **WHEN** `TestGhost` appears only as a method `func (s *Suite) TestGhost()` and as a
  subtest name `t.Run("TestGhost", …)`
- **THEN** `pkg/store/store_test.go::TestGhost` reports
  `'pkg/store/store_test.go' defines no 'TestGhost'`

#### Scenario: Go chain deeper than one name
- **WHEN** node `pkg/store/store_test.go::TestPut::empty` is declared
- **THEN** the reason is `go addresses reach at most 1 names`

#### Scenario: Rust fn inside a string or a comment does not count
- **WHEN** `fn ghost()` appears only in a raw string `r#"…"#`, a nested block comment and a doc comment
- **THEN** `<path>::ghost` does not resolve

### Requirement: An unknown form is a broken setup
An `id_names` value the adapter cannot honour (an unknown preset or language, or a
structured block of the wrong shape) can reach it only through a direct invocation,
since the core rejects it at load. It SHALL fail the adapter with exit 2, naming the
kind and the offending value, and no document SHALL be written. It is a broken adapter
input, not malformed target content.

Verified by: `cargo test -p latt-adapter --locked source`

#### Scenario: Directly invoked with an unknown form
- **WHEN** the md adapter is run directly with a resolved profile whose kind declares `id_names: jest`
- **THEN** it exits 2 naming the kind and `jest`, and writes no document

### Requirement: Unreadable source is reported, not passed
A source file the resolver cannot read under its language SHALL produce one warning
`PARSE_ERROR` per file per run, at the file and the line where reading failed (line 0
when no line applies), with message `could not read <language> source '<path>': <reason>`.
The reasons SHALL include an unterminated string or comment, an unbalanced bracket,
invalid UTF-8, and an I/O error reading a file that resolved as a regular file (for
example, permission denied). The file address's `could not be read` reason is for a
failure before that point, when the path itself cannot be resolved. A node naming that file SHALL then emit no `SOURCE_MISSING`. It
was not checked, and the `PARSE_ERROR` already says so.

A file SHALL be read at most once per run, however many nodes name it. That index
SHALL NOT outlive the run.

Every language SHALL accept a leading byte order mark (encoding, not content) and CRLF
line endings. Python SHALL handle string prefixes (`r`, `b`, `u`, `f` and their
combinations, either case), single- and triple-quoted strings, comments, backslash
continuation, implicit continuation inside brackets, tab or space indentation, a
trailing comment after a class header's `:`, and f-string replacement fields that reuse
the enclosing quote or carry a comment (PEP 701), read as Python 3.12 reads them. Rust SHALL handle line
and doc comments, nested block comments, string, byte-string, raw and raw-byte strings
with any number of `#`, char literals as distinct from lifetimes, and attributes. Go SHALL handle line and
(non-nesting) block comments, interpreted strings, multi-line raw strings in backticks,
and rune literals.

Verified by: `cargo test -p latt-adapter --locked source`

#### Scenario: Unterminated Python string
- **WHEN** `tests/test_bad.py` opens a triple-quoted string at line 3 that never closes,
  and two nodes name tests in it
- **THEN** one `PARSE_ERROR` at `tests/test_bad.py:3` reads
  `could not read python source 'tests/test_bad.py': unterminated string`, and neither
  node produces `SOURCE_MISSING`

#### Scenario: Unterminated Rust block comment
- **WHEN** `tests/bad.rs` opens `/* /* */` at line 5 and never closes the outer comment
- **THEN** one `PARSE_ERROR` at `tests/bad.rs:5` reads
  `could not read rust source 'tests/bad.rs': unterminated block comment`

#### Scenario: Unterminated Go raw string
- **WHEN** `pkg/bad_test.go` opens a backtick raw string at line 9 that never closes
- **THEN** one `PARSE_ERROR` at `pkg/bad_test.go:9` reads
  `could not read go source 'pkg/bad_test.go': unterminated raw string`

#### Scenario: Bracket continuation does not end a Python class body
- **WHEN** a method body in `class TestGroup:` holds a call whose arguments continue
  on a line at column 0, and `def test_after` follows in the class body
- **THEN** node `tests/test_a.py::TestGroup::test_after` resolves

#### Scenario: PEP 701 f-string
- **WHEN** a top-level line reads `x = f"{"def test_fake(): pass"}"` and is followed by `def test_real(): pass`
- **THEN** `test_real` resolves and `test_fake` does not

#### Scenario: Comment inside an f-string replacement field
- **WHEN** a multi-line replacement field holds `# unmatched quote '` before its closing `}`
- **THEN** the file scans without a `PARSE_ERROR` and a following `def test_real()` resolves

#### Scenario: Lifetime is not a char literal
- **WHEN** `fn helper<'a>(x: &'a str) {}` is followed by a top-level `fn real() {}`
- **THEN** `<path>::real` resolves

