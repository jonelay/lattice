# adapter-templates Specification

## Purpose
Templates a downstream developer copies to write an external adapter in Rust, Go or
Python: three implementations of one invented register format that must print the same
interface document, so each language's template is checked against the others.
## Requirements
### Requirement: Three templates print one document
`examples/adapters/` SHALL hold three adapter programs, in Rust (`rust/`), Go (`go/`) and
Python (`python/`), with one shared `profile.yaml`, one shared target `fixture/`, and
`expected.json`, the interface document each program SHALL print for that profile and
target. Documents compare as parsed JSON, so key order and whitespace do not count. The
fixture SHALL exercise every rule of the format below: a leading BOM, CRLF line ends,
comments and blank lines, a node with several edges, a malformed line, a line that is not
valid UTF-8, a reference to an undeclared ID, and a file the configuration does not
select.

Each template SHALL carry a test in its own language's test tool that builds the
fixture's document in-process, with the configuration `profile.yaml` gives, and compares
it with `expected.json`, so a copied template comes with a working test that needs no
`latt`. `tests/test_adapter_templates.py` SHALL run each program that its
toolchain allows on this host, compare the three, and run `latt validate` over the
fixture through each, checking the exit code and the node, edge and finding counts that
`expected.json` implies. A template whose toolchain is absent SHALL be skipped by name,
never counted as passing.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_templates.py`,
`cargo test -p lines-adapter --locked`, `go test ./...` in `examples/adapters/go`, and
`python3 -m unittest discover examples/adapters/python`.

#### Scenario: The templates agree
- **WHEN** each template runs with `--profile` set to the resolved `profile.yaml` and `--target` set to `fixture/`
- **THEN** each prints a document equal to `expected.json` and exits 0

#### Scenario: A template drifts
- **WHEN** one template's output differs from `expected.json` in any node, edge, finding or provenance
- **THEN** its own test and the cross-check fail, naming the template

#### Scenario: Validation through a template
- **WHEN** `latt validate --profile examples/adapters/profile.yaml --adapter <template> --target examples/adapters/fixture` runs
- **THEN** it exits 1 for the fixture's error findings, with the node and edge counts of `expected.json`

#### Scenario: Go is not installed
- **WHEN** the cross-check runs on a host with no `go` on PATH
- **THEN** the Go template's case is reported as skipped, naming the missing toolchain

### Requirement: The templates read the line register format
A template SHALL take `--profile <resolved-profile.json>` and `--target <dir>`. It SHALL
read the profile's `adapter:` block, which SHALL hold `directory`, a path relative to the
target, and `extension`, a file-name suffix. The core's own keys (`native`,
`timeout_seconds`, `command_timeout_seconds`) SHALL be accepted and ignored.

It SHALL read the regular files directly inside `directory` whose names end in
`extension`, in byte order of name, and no others. In each file it SHALL strip one
leading UTF-8 BOM, split lines on `\n`, drop one trailing `\r`, and skip lines that are
blank or whose first non-blank character is `#`. Every other line SHALL be a declaration:

```text
<kind> <id> [<edge-kind> <target-id>]... : <text>
```

The part before the first `:` SHALL be split on whitespace into a kind, an ID, and zero
or more edge-kind and target pairs; `<text>` is the rest of the line with surrounding
whitespace removed. A declaration SHALL become one node (`id`, `kind`, attr `text`,
provenance at the file relative to the target with `/` separators, and the line) and one
edge per pair, from the node's ID to the target ID, at the same provenance, with
`declared_by` set to the node's index in `nodes`. The template SHALL NOT check kinds, ID
shapes or targets; the core does that against the profile.

Verified by: the cases of `tests/test_adapter_templates.py` and the templates' own tests.

#### Scenario: A declaration with two edges
- **WHEN** a selected file has the line `test T-2 verifies REQ-2 verifies REQ-9: Relay opens`
- **THEN** the document holds node `T-2` of kind `test` with `text` `Relay opens`, and two `verifies` edges from `T-2`, to `REQ-2` and `REQ-9`, each with `declared_by` the index of `T-2`

#### Scenario: A file outside the selection
- **WHEN** `directory` holds `notes.txt` and `extension` is `.reg`
- **THEN** nothing from `notes.txt` appears in the document

### Requirement: Unreadable input becomes a finding
A template SHALL report every part of the register it cannot read as an error-severity
`PARSE_ERROR` finding with `node_id` null, and SHALL read the rest:

- a declaration line that does not match the grammar, at its line;
- a line that is not valid UTF-8, at its line, naming the one-based byte column of its
  first invalid byte;
- a selected file it cannot open or read, at line 0;
- a selected entry that is not a regular file, at line 0;
- an entry of `directory` whose name is not valid UTF-8, at line 0 of the directory;
- a `directory` that does not exist or is not a directory, at line 0, naming it.

A readable `directory` with no selected file SHALL give one info-severity
`NO_MATCHING_FILES` finding naming the directory and the extension. The template SHALL
exit 0 whenever it printed a document.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_templates.py`.

#### Scenario: A malformed line among good ones
- **WHEN** a file's third line is `test T-3 verifies` with no `:`
- **THEN** the document holds a `PARSE_ERROR` at that file and line 3, and the declarations on the other lines are read

#### Scenario: The directory is missing
- **WHEN** `directory` names a path the target does not hold
- **THEN** the document holds one `PARSE_ERROR` naming it at line 0, no nodes, and the template exits 0

#### Scenario: Nothing is selected
- **WHEN** `directory` exists and no file in it ends in `extension`
- **THEN** the document holds one `NO_MATCHING_FILES` info finding and no nodes

### Requirement: A template that cannot produce a document exits non-zero
A template SHALL print nothing on stdout and exit 2, with one line on stderr naming the
problem, when an argument is missing or unknown, when the profile file cannot be read or
is not a resolved profile (`resolved_schema` `"1"`), when `directory` or `extension` is
absent or not a string, or when the `adapter:` block holds another key. The core then
exits 2 and ingests nothing.

Verified by: `.venv/bin/python -m pytest tests/test_adapter_templates.py`.

#### Scenario: An unknown adapter key
- **WHEN** the profile's `adapter:` block holds `recursive: true`
- **THEN** each template exits 2 naming `recursive`, and stdout is empty

#### Scenario: Not a resolved profile
- **WHEN** `--profile` names the YAML profile rather than the core's resolved JSON
- **THEN** each template exits 2 naming the file

### Requirement: The templates stand alone and ship
Each template SHALL be copyable on its own. The Python template SHALL be one file using
only the standard library. The Go template SHALL be its own module using only the
standard library. The Rust template SHALL depend only on `latt-adapter` and `serde_json`, and
SHALL inherit no key from the workspace; it SHALL depend on `latt-adapter` by path inside this repository, and
`examples/adapters/README.md` SHALL give the git dependency a copy outside the repository
uses instead. No template SHALL import a module from `adapters/`.

`examples` SHALL ship in the GitHub release snapshot and in the release install, and
`docs/adapters.md` › Writing your own adapter SHALL link the templates.

Verified by: inspection of the three manifests and of `tools/snapshot-files` and
`tools/install-files`; `.venv/bin/python -m pytest tests/test_docs_paths.py`.

#### Scenario: The Python template outside the repository
- **WHEN** `examples/adapters/python/` is copied alone to another directory and run with `python3 -I`
- **THEN** it prints the same document as in place

