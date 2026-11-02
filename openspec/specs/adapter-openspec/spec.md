# adapter-openspec Specification

## Purpose
An adapter reading an OpenSpec register — `openspec/specs/<capability>/spec.md` — together
with the requirement citations its test files already carry, so a repo whose contract lives
in OpenSpec can be audited for citations naming no requirement and requirements no test
cites. Lattice's own register is the first consumer.
## Requirements
### Requirement: Run as a stdlib-only interface program
The adapter SHALL be a program that, given `--profile` and `--target`, writes an interface
document to stdout and exits 0. Malformed input SHALL become an `Issue` in that document
and SHALL NOT change the exit status; a non-zero exit SHALL mean the adapter itself broke.
It SHALL depend on nothing outside the Python standard library. A profile the adapter
cannot run from — a missing `adapter.paths.spec_dir`, a `tests` value that is not a list,
or a `dialect` it does not know — SHALL exit 2 with one line on stderr naming the key and, for a dialect, the supported ones; it
SHALL NOT print a traceback.

#### Scenario: Well-formed register
- **WHEN** the adapter runs against a target whose register parses
- **THEN** it exits 0 and stdout is an interface document the core ingests

#### Scenario: Malformed register still exits 0
- **WHEN** the target contains a spec file the adapter cannot read
- **THEN** the adapter exits 0 and the document carries a `PARSE_ERROR` issue

#### Scenario: Unknown dialect is one line, exit 2
- **WHEN** a test glob entry names a `dialect` the adapter does not know
- **THEN** the adapter exits 2, stdout is empty, and stderr is one line naming that
  dialect and the supported ones, with no traceback

#### Scenario: A spec-only profile
- **WHEN** the profile's `adapter.paths.tests` is absent or an empty list
- **THEN** the adapter exits 0, emits the capability and requirement nodes, and emits no
  `test` node and no issue about tests

**Verified by:** `.venv/bin/python -m pytest -q tests/test_adapter_openspec.py`

### Requirement: Build capability and requirement nodes from the register
The adapter SHALL read every `spec.md` under the profile's `spec_dir`, emitting one
`capability` node per file identified by its containing directory name, and one
`requirement` node per `### Requirement: <title>` heading identified by
`<capability>/<title>`: the capability's directory name, `/`, and the title text
exactly as written. Each requirement SHALL carry a `contains` edge from its capability.
Identified by its title alone, a requirement collided with a same-titled one in
another capability: one node survived, the other's attrs were lost, and its `contains`
edge landed on the survivor. The capability half matches the capability kind's
`^[a-z0-9-]+$`, which has no `/`, so a title containing `/` stays unambiguous.
`profiles/openspec.yaml` declares this shape as `profile_version` `1.4.0`.
Requirement nodes SHALL carry their scenario count and their title text as attrs, and
the profile SHALL bind summary attrs (`title` on `requirement`, `function` on `test`)
so a suggestion producer has text to rank on.

A requirement's scenario count, like its body, SHALL end at the next heading of level 1
through 3. A `#### Scenario:` heading under no requirement — before the first, or under
a section such as `### MODIFIED:` — SHALL be counted onto no node; the adapter SHALL
emit one `PARSE_ERROR` per such section, at its heading, naming the heading and how many
scenarios it holds.

A requirement node SHALL additionally carry its **body text**: every line between its
`### Requirement:` heading and the next heading of level 1 through 3, so the normative
prose, the rationale and the `#### Scenario:` blocks all travel with the node. A verification
pointer line SHALL be excluded from that body, in **every** form the register writes it —
both `Verified by:` and `**Verified by:**`. It names a test command rather than describing
the requirement, and the same command boilerplate recurs under most requirements, so
including it pulls unrelated requirements toward every test. Excluding only one form leaves
the pointer in the bodies that use the other, which a measured run of this register showed
leaves ten of them in place. A requirement whose heading is followed by no body text
SHALL carry no body attr rather than an empty one.

#### Scenario: Capability and its requirements
- **WHEN** `openspec/specs/validation/spec.md` declares a requirement titled "Issue model"
- **THEN** the graph holds a `capability` node `validation`, a `requirement` node
  `validation/Issue model`, and a `contains` edge from the first to the second

#### Scenario: A title shared by two capabilities is two requirements
- **WHEN** capabilities `alpha` and `beta` each declare `### Requirement: Shared title`
- **THEN** the graph holds `alpha/Shared title` and `beta/Shared title`, each with its
  own scenario count and body and a `contains` edge from its own capability

#### Scenario: Scenario headings are counted, not emitted
- **WHEN** a requirement declares three `#### Scenario:` headings
- **THEN** the requirement node's scenario count attr is 3 and no scenario node exists

#### Scenario: A section heading ends the requirement above
- **WHEN** a spec holds one scenario before its first requirement, and a
  `### MODIFIED:` heading with two scenarios between two requirements
- **THEN** each requirement counts only its own scenario, and one `PARSE_ERROR` at each
  section heading names it with 1 and 2 scenarios

#### Scenario: Requirement nodes carry their title as text
- **WHEN** a requirement titled "Issue model" is emitted
- **THEN** its node carries a `title` attr equal to `Issue model`, the attr the
  profile names as the requirement kind's `summary_attr`

#### Scenario: Requirement nodes carry their body text
- **WHEN** a requirement's heading is followed by prose and two scenario blocks, and the
  next `### Requirement:` heading follows those
- **THEN** the node's body attr holds the prose and both scenario blocks, and stops at the
  next requirement heading

#### Scenario: A verification pointer is excluded from the body, in either form
- **WHEN** one requirement's body contains a `Verified by:` line and another's contains a
  `**Verified by:**` line
- **THEN** neither line appears in its node's body attr, and the surrounding prose is present
  in both

#### Scenario: A requirement with no body carries no body attr
- **WHEN** a `### Requirement:` heading is immediately followed by the next heading
- **THEN** the node carries no body attr

**Verified by:** `.venv/bin/python -m pytest -q tests/test_adapter_openspec.py`

### Requirement: Build test nodes from the cited test files
The adapter SHALL read every file matched by the profile's test globs and emit one `test`
node per test function found, identified as `<target-relative path>::<function name>` —
with the enclosing class qualifying a method, `<path>::<class>::<function name>` —
whether or not that function is covered by a citation. A test function covered by no
citation SHALL enter the graph with no edges rather than be omitted.

The dialect applies to every file the glob matches, whatever its extension. A glob that
matched at least one file and yielded no test node from any of them SHALL emit one
`PARSE_ERROR` issue of warning severity naming the file count, the glob and the dialect;
a single matched file that yields no test (a helper module beside the tests) SHALL
stay silent. The whole-glob zero is the wrong dialect or the wrong glob, and must not
read as a register with nothing in it.

TypeScript discovery SHALL read a test as an `it(...)` or `test(...)` call opening a
line of code, with any modifiers and argument groups between the callee and the name
(`.skip`, `.skipIf(cond)`, `.each(table)`), the name being the first argument group's
leading string literal, which may sit on a later line than the callee and after an
argument group of any length. Its function name
SHALL be the enclosing `describe` names and its own joined by `::`, a `describe` lasting
while the brace depth stays above where its line began. A line that opens inside a
template literal (including a `${...}` hole's surrounding text) or a block comment is
text and declares nothing; braces and quotes inside string, template and regex literals
SHALL NOT count.

A test node models a **source definition**, for Rust, Python and TypeScript alike: the node is the
function as written, not a runtime test case, and the ID is not joinable against a test
runner's own case identifiers.

Python discovery SHALL be structural — the parsed module, not a pattern over its lines:
a `test_`-named `def` or `async def` at module top level or as a direct method of a
top-level class is a test, and text that merely resembles a definition, such as a `def`
inside a string literal, is not. Rust discovery SHALL read `#[test]` and the `fn` it
marks only from code: a line that opens inside a string literal or block comment is
text, and declares nothing.

A test node SHALL additionally carry its **body text**: the lines of the function's body,
bounded by the dialect's own block rule — brace depth for Rust, counting the declaration
line's own brace and disregarding braces inside string literals, character literals and
comments; the first line dedented past the `def` for Python; for TypeScript, the
callback's braces after the name, or, for a callback with none, the expression after its
`=>` up to the call's closing parenthesis. A test function with an empty
body SHALL carry no body attr rather than an empty one. The Rust rule SHALL lex the
function as one stream, not line by line: a string literal continued past a trailing `\`,
a raw string, or a block comment spanning lines keeps its braces out of the count on every
line it covers, not only the one that opened it.

Where the adapter cannot establish the bounds of a function's body it SHALL emit a
`PARSE_ERROR` issue and carry no body attr for that node, rather than extending to the next
recognizable boundary. An unbounded extraction attaches a *different* test's code to the
node, which is wrong text presented as the node's own — the silent-mishandling failure the
register's contract forbids, and one no reviewer can see in a ranked list.

#### Scenario: Uncited test is present, not dropped
- **WHEN** a test file declares a test function above any citation comment
- **THEN** a `test` node exists for it carrying no `verifies` edge

#### Scenario: Test identity is path-qualified
- **WHEN** two test files each declare a function of the same name
- **THEN** two distinct `test` nodes exist, each named by its target-relative path

#### Scenario: Test identity is class-qualified
- **WHEN** one file declares a function name both at top level and as a method of a class
- **THEN** two distinct `test` nodes exist, the method's carrying its class name

#### Scenario: An async test function is discovered
- **WHEN** a Python test file declares `async def test_a` under a citation
- **THEN** a `test` node exists for it carrying the citation's `verifies` edge

#### Scenario: A definition inside a string literal is not a test
- **WHEN** a string literal in a Python test file contains a line reading `def test_x():`
- **THEN** no `test` node is emitted for that text

#### Scenario: A Rust test inside a raw string or block comment is not a test
- **WHEN** a raw string literal or a block comment in a Rust test file contains the lines
  `#[test]` and `fn fake() {}`
- **THEN** no `test` node is emitted for `fake`

#### Scenario: A TypeScript test is named by its describe path
- **WHEN** a TypeScript file declares `it("reads x", ...)` inside `describe("parser", ...)`
  and the same name inside a second `describe`
- **THEN** a `test` node `<path>::parser::reads x` exists, distinct from the second one

#### Scenario: TypeScript inside a template literal or block comment is not code
- **WHEN** a template literal in a TypeScript test file holds the lines
  `// Requirement: solo/Solo reports` and `it("fake", () => {});`
- **THEN** no `test` node is emitted for `fake` and no edge to `solo/Solo reports`

#### Scenario: TypeScript modifiers and a multi-line table are read
- **WHEN** a TypeScript file declares `it.skipIf(cond)('a', ...)` and an `it.each([...])`
  whose table spans several lines before its name
- **THEN** a `test` node exists for each, named by its string literal

#### Scenario: Test nodes carry their function body
- **WHEN** a test function declares three statements and another test function follows it
- **THEN** the node's body attr holds those three statements and none of the following
  function

#### Scenario: A test with an empty body carries no body attr
- **WHEN** a test function's body is empty
- **THEN** the node carries no body attr

#### Scenario: A one-line function does not absorb the functions after it
- **WHEN** a Rust test is written `fn t() {}` on one line and two further tests follow
- **THEN** that node carries no body attr and neither following test's code appears in it

#### Scenario: An indented closing brace still bounds the body
- **WHEN** a Rust test function's closing brace is indented rather than at column zero
- **THEN** the body stops at that brace and the following function is not included

#### Scenario: A brace inside a string or comment does not bound the body
- **WHEN** a test function's body contains `"}"` in a string literal before its real end
- **THEN** the body continues past it to the function's actual closing brace

#### Scenario: A string literal or comment spanning lines does not bound the body
- **WHEN** a Rust test function's body contains a `format!` string whose lines end in `\`
  and whose continuation lines carry `{{`, `}}` and `\"`, a raw string or a block comment
  spanning lines with `}` inside, or a `'}'` character literal
- **THEN** no `PARSE_ERROR` is emitted and the body runs to the function's actual closing
  brace, excluding the function that follows it

#### Scenario: Unestablishable bounds are reported, not guessed
- **WHEN** a test function's body has no resolvable closing brace before end of file
- **THEN** the adapter emits a `PARSE_ERROR` issue, the node carries no body attr, and the
  adapter still exits 0 having read the rest of the file

#### Scenario: A glob whose files hold no test item is reported
- **WHEN** a `rust` glob matches one `.ts` file holding no Rust test
- **THEN** the adapter emits one warning `PARSE_ERROR` reading `1 file matched test glob
  '<glob>' and none held a rust test item`, no per-file issue, and no `test` node

#### Scenario: A helper module beside real tests is silent
- **WHEN** a glob matches a helper module with no test and files that hold tests
- **THEN** no zero-yield issue is emitted

**Verified by:** `.venv/bin/python -m pytest -q tests/test_adapter_openspec.py`

### Requirement: A citation is a section header binding the tests that follow it
A citation SHALL be a whole-line comment whose text after the comment marker begins
`Requirement:` followed by a requirement ID, `<capability>/<title>`, and it SHALL bind every test function from that line until the next
citation in the same file or end of file. Directly consecutive citation lines — each on
the line immediately after the previous, with nothing between them — SHALL form one
stack that binds the same group: each bound test SHALL carry one `verifies` edge per
citation line in the stack, and each edge SHALL carry the provenance of its own
citation line. A citation separated from the previous one by any other line SHALL
replace it rather than join it. The adapter SHALL recognise the comment markers of each
language it is pointed at, and only as comments: a line reading like a citation inside a
string literal, or inside a Rust block comment, is text and cites nothing.

#### Scenario: One citation binds the group beneath it
- **WHEN** a citation is followed by two test functions and then a second citation
- **THEN** the first two tests each carry a `verifies` edge to the first requirement and
  neither carries one to the second

#### Scenario: Stacked citations bind the group to every named requirement
- **WHEN** two citation lines on directly consecutive lines are followed by two test
  functions
- **THEN** each of the two tests carries two `verifies` edges, one to each cited
  requirement, and each edge's provenance line is the citation line that declared it

#### Scenario: A citation inside a string literal cites nothing
- **WHEN** a citation is followed by a string literal holding a line that reads like a
  second citation, and then a test function
- **THEN** the test carries one `verifies` edge, to the first requirement

#### Scenario: A blank line breaks the stack
- **WHEN** a citation line is followed by a blank line, a second citation line, and a
  test function
- **THEN** the test carries exactly one `verifies` edge, to the second requirement

#### Scenario: Citation naming no declared requirement
- **WHEN** a citation names a title no spec file declares
- **THEN** the `verifies` edge is still emitted and validation reports `VACANCY`

**Verified by:** `.venv/bin/python -m pytest -q tests/test_adapter_openspec.py`

### Requirement: A verifies edge records citation, never outcome
A `verifies` edge SHALL mean that a test cites a requirement. It SHALL NOT mean the test
passed, was run, or was not skipped. The adapter SHALL NOT read test results, exit codes,
or any report of execution.

#### Scenario: Failing and passing tests are indistinguishable in the graph
- **WHEN** a cited test would fail if run
- **THEN** its `verifies` edge is identical to that of a cited test that would pass

**Verified by:** inspection — the adapter reads only source text, and its test suite
provides it no execution record to read.

### Requirement: One requirement title per citation line
The adapter SHALL treat the entire citation text as a single requirement ID and SHALL
NOT split it on any separator. A line naming two requirements therefore resolves to
neither and is reported as a dangling reference, which is the signal to split it into two
citation lines.

#### Scenario: Compound citation is one unresolved title
- **WHEN** a citation reads
  `Requirement: output/Three output formats / output/Output dispatcher`, binds
  exactly one test, and no requirement carries that exact ID
- **THEN** one `verifies` edge is emitted naming that whole string, and validation reports
  one `VACANCY` — one per bound test, since the finding is per edge

**Verified by:** `.venv/bin/python -m pytest -q tests/test_adapter_openspec.py`

### Requirement: Unreadable input is reported, never dropped
A spec file the adapter cannot read, a `spec_dir` that does not resolve, a readable
`spec_dir` holding no `<capability>/spec.md`, a directory
under `spec_dir` or under a test glob that the glob would read and cannot, a test glob
matching nothing, a Python test file that does not parse, and a TypeScript test call whose name is not a
string literal SHALL each produce an issue
naming what was not read. The adapter SHALL continue past any one of them and SHALL NOT
raise.

#### Scenario: Undecodable spec file
- **WHEN** a file under `spec_dir` cannot be decoded as UTF-8
- **THEN** a `PARSE_ERROR` is at each undecodable line, shown as written, and the
  remaining lines and files are still read

#### Scenario: Undecodable line inside a requirement
- **WHEN** a line of a requirement's section, its heading included, is not valid UTF-8
- **THEN** that requirement is not read and a second `PARSE_ERROR` at its heading names
  the line; the requirements before and after it are read with their own bodies and
  scenarios, and none takes the withheld requirement's

#### Scenario: Undecodable test file
- **WHEN** a test file a glob matches is not valid UTF-8
- **THEN** each undecodable line is a `PARSE_ERROR`, shown as written, and the file
  yields no test

#### Scenario: A directory the adapter cannot read
- **WHEN** a capability directory under `spec_dir`, or a directory a test glob would
  search, cannot be read
- **THEN** a `PARSE_ERROR` names that directory and every readable spec and test is
  still read

#### Scenario: A spec directory holding no spec file
- **WHEN** `spec_dir` is a readable directory and no `<capability>/spec.md` lies under it
- **THEN** a warning `PARSE_ERROR` names `spec_dir` and the `<capability>/spec.md`
  layout, rather than the run appearing to read an empty register; when `spec_dir`
  itself cannot be listed, only that directory's `PARSE_ERROR` is reported

#### Scenario: A test glob matching nothing
- **WHEN** a configured test glob matches no file
- **THEN** an issue reports the empty glob rather than the run appearing to have no tests

#### Scenario: A Python test file that does not parse
- **WHEN** a scanned Python file carries a syntax error
- **THEN** a warning `PARSE_ERROR` names it, no test from it is guessed at, and the
  remaining files are still read

#### Scenario: A TypeScript test with no literal name
- **WHEN** a TypeScript file declares `it(name, () => {...})` with `name` a variable
- **THEN** a warning `PARSE_ERROR` names the call's line and no test is guessed at

**Verified by:** `.venv/bin/python -m pytest -q tests/test_adapter_openspec.py`

### Requirement: Coverage is visible and never exit-affecting
The profile SHALL configure `COVERAGE` over `requirement` via the `verifies` edge at
severity `hint`, so uncited requirements are reported on every run and contribute to no
exit code, including under `--strict`. `VACANCY` SHALL remain at severity `error`.

#### Scenario: Uncited requirements under strict
- **WHEN** the register holds requirements no test cites and no dangling citations exist
- **THEN** `latt validate --strict` reports them and exits 0

#### Scenario: A dangling citation fails the run
- **WHEN** any citation names no declared requirement
- **THEN** `latt validate` reports `VACANCY` at error severity and exits 1

**Verified by:** `target/debug/latt validate --profile profiles/openspec.yaml
--adapter ./adapters/openspec --target . --strict`

### Requirement: Lattice audits its own register
Lattice's own repository SHALL be a target of this adapter, and the run SHALL exit 0 —
no citation in the Rust or Python suites naming a requirement the register does not
declare. Uncited requirements are reported as hints and do not affect that result.

#### Scenario: The self-audit is clean
- **WHEN** the adapter runs against this repository with `profiles/openspec.yaml`
- **THEN** no `VACANCY` is reported and the exit status is 0

#### Scenario: A count check accompanies the finding count
- **WHEN** the self-audit is used as evidence
- **THEN** node and edge counts are reported alongside it, because zero dangling
  references over zero edges is satisfied by an adapter that built no edges

**Verified by:** `target/debug/latt validate --profile profiles/openspec.yaml
--adapter ./adapters/openspec --target .` for the exit status, and — the adapter reads
the core-resolved document, not the raw YAML —
`target/debug/latt show --profile profiles/openspec.yaml >| /tmp/p.json &&
./adapters/openspec --profile /tmp/p.json --target . | jq '(.nodes|length),
(.edges|length)'` for the counts that must accompany it.

