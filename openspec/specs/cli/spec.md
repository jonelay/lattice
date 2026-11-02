# cli Specification

## Purpose
The CLI entry point — subcommand structure, format flag, profile selection,
and exit code semantics.
## Requirements
### Requirement: CLI entry point
`latt` SHALL be a CLI with subcommands. The subcommands are `validate`,
`summary`, `trace`, `coverage`, the `query` family, `describe`, `fuse`, `show`, `browse` and
`adapter`. `validate`, `summary`, `trace`, `coverage`, `query` and `describe` take `--profile`,
`--target`, `--adapter` and `--format` (`describe`: the describe-command capability). `fuse` takes `--manifest`, `--format` and `--strict` in
place of a profile and target (the program-composition capability). `show` takes
`--profile` and no register options (Requirement: Show command). `adapter` takes a native
adapter name, `--profile` and `--target` only (Requirement: Adapter command). `browse` takes
`--profile`, `--target`, `--adapter`, `--strict`, `--filter`, and `--glyphs` but not `--format` —
it owns its terminal directly and does not route output through `output_result`.
Every command that takes `--profile` also takes the optional `--profile-version`
(Requirement: Profile version pin); `fuse` does not.
`validate` and `trace` additionally take `--strict`;
`summary`, `coverage`, and `query` do not — `summary` runs no validation pass,
and only error-severity adapter issues affect its exit code; `coverage` is a
report that produces no findings; `query` produces no findings at all, so no
severity affects its exit code (see the `query` capability).

The adapter issues `summary` reports and exits on SHALL carry resolved severities —
profile overrides, then pathway demotion — the same severities `validate` would report for
the same graph, and SHALL honour the profile's `SUPPRESS` entries the same way: a
suppressed adapter issue is omitted from `plain` and `rich`, kept in `json` with
`"suppressed": true`, and does not exit 1. The same applies to the adapter issues
`coverage`, `query` and `query diff` report on stderr. When every adapter issue is
suppressed, `plain` and `rich` write nothing to stderr. Running no validation pass means
`summary` computes no findings of its own; it does not mean it reports an adapter issue
at a severity, or a suppression, the profile has already said is wrong. A register from
which the adapter read no node is still refused on the adapter's unsuppressed report
(see Exit codes).

All subcommands that previously required `--adapter` SHALL accept it as optional.
When supplied, `--adapter` names either a known native adapter by name (e.g.
`--adapter md`) or an external program path. When omitted, the adapter is
resolved from the profile's `adapter.native` key. See the
`builtin-adapter-dispatch` capability for resolution precedence.

`browse` SHALL be feature-gated behind the `tui` feature of the `latt` package, which
is on by default, so `cargo install latt` builds it. Without it, the
`browse` variant is absent from the CLI. The `latt-core` library SHALL have no `tui`
feature and no normal dependency on `clap` or the terminal crates. Interactive
commands own their terminal; batch commands route through `output_result`.

Verified by: `cargo test --test cli` and `cargo test --test native`.

#### Scenario: Run validate
- **WHEN** the user runs `latt validate --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, validates, and reports findings

#### Scenario: Run validate with native adapter
- **WHEN** the user runs `latt validate --profile p.yaml --adapter md --target /repo`
- **THEN** lattice loads the profile, runs the native md adapter, ingests its document,
  validates, and reports findings

#### Scenario: Run validate with profile-declared adapter
- **WHEN** the user runs `latt validate --profile p.yaml --target /repo` and the
  profile declares `adapter: { native: md }`
- **THEN** latt resolves the md native adapter from the profile and proceeds as normal

#### Scenario: Run validate with explicit external adapter
- **WHEN** the user runs `latt validate --profile p.yaml --adapter ./adapters/md --target /repo`
- **THEN** lattice runs the external adapter program, unchanged from current behavior

#### Scenario: Run summary
- **WHEN** the user runs `latt summary --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, and reports the configured status rollup

#### Scenario: Run trace
- **WHEN** the user runs `latt trace --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, validates the graph, and outputs the trace report

#### Scenario: Run coverage
- **WHEN** the user runs `latt coverage --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, and reports per-kind coverage statistics

#### Scenario: Run query
- **WHEN** the user runs `latt query counts --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, and reports the answer

#### Scenario: Run browse
- **WHEN** the user runs `latt browse --profile p.yaml --target /repo` with the `tui` feature enabled
- **THEN** the TUI starts in alternate screen showing the node list view

#### Scenario: Browse absent without tui feature
- **WHEN** lattice is built without the `tui` feature and the user runs `latt browse`
- **THEN** clap reports an unrecognized subcommand error

#### Scenario: Summary honours a profile severity override on an adapter code
- **WHEN** the profile overrides an error-severity adapter code to `info` and `summary` runs
- **THEN** `summary` reports that issue at `info` and exits 0

#### Scenario: Summary honours a pathway demotion
- **WHEN** a pathway binding demotes an error-severity adapter issue to `info` and `summary`
  runs
- **THEN** `summary` reports that issue at `info` and exits 0, agreeing with `validate`

#### Scenario: Summary honours a SUPPRESS on an adapter code
- **WHEN** the profile suppresses `PARSE_ERROR` and the adapter reports one at error
  severity
- **THEN** `summary` exits 0 as `validate` does, `plain` stderr omits the finding, and
  `json` stderr carries it with `"suppressed": true`

#### Scenario: A query omits a suppressed adapter issue from plain stderr
- **WHEN** the profile suppresses `PARSE_ERROR` and `query counts` runs with `--format plain`
- **THEN** the answer is printed, stderr carries no `PARSE_ERROR` line, and the exit is 0

#### Scenario: Stderr is empty when every adapter issue is suppressed
- **WHEN** the profile suppresses every adapter issue and `summary` or `query counts` runs
  with `--format plain` or `--format rich`
- **THEN** stderr is empty: no blank line and no `No findings.`

### Requirement: Version flag
`latt --version` SHALL print the installed package version and exit 0. The version
SHALL come from the installed package metadata, the same source as the trace report
header, so the two cannot disagree.

#### Scenario: Version flag
- **WHEN** the user runs `latt --version`
- **THEN** lattice prints the package version and exits 0

### Requirement: Format flag
Every subcommand except `browse` and `show` SHALL accept `--format=plain|json|rich` (default: `rich` when TTY,
`plain` otherwise). The parser SHALL reject any other value as a usage error, naming
the value and the three accepted ones, with exit 2: the flag parses straight into
`output::Format`, so the dispatcher never sees a format it does not know.

Verified by: `cargo test --test cli format`.

#### Scenario: Invalid format value
- **WHEN** `latt validate` runs with `--format yaml`
- **THEN** lattice exits 2 and stderr names `yaml` and the accepted values

#### Scenario: Auto-detect format
- **WHEN** `latt validate` runs with stdout connected to a TTY and no `--format` flag
- **THEN** output uses rich format

#### Scenario: Pipe auto-detect
- **WHEN** `latt validate` output is piped (not a TTY) and no `--format` flag
- **THEN** output uses plain format

### Requirement: Exit codes
Lattice SHALL use three exit codes: 0 when the command succeeded with no errors, 1 when
the register produced error-severity findings, and 2 when lattice could not run the
command at all (unreadable profile, profile refused by `--profile-version`, adapter program that cannot be run, adapter that
exits non-zero, adapter output that cannot be parsed or does not satisfy the interface
schema, or an interface version the core does not support). With `--strict` (on the
commands that take it), warnings are promoted to errors before the exit code decision.

A suppressed finding SHALL NOT count toward exit code 1, whatever its severity and
whether or not `--strict` promoted it. Suppression runs after promotion and before the
exit-code decision, so the count that decides between 0 and 1 is the error-severity
findings that are not suppressed. A suppressed finding is still reported in `json`
(see the `output` capability); it is excluded from the gate, not from the record.

Exit 2 distinguishes "lattice is misconfigured" from "the register has errors", so a
caller can tell a broken setup from a genuine finding.

A report that cannot be written to stdout was not delivered, so a failed stdout write
SHALL exit 2 whatever the findings, never 0 over a truncated report and never a
panic's 101. A write refused by a full disk or similar SHALL be said on stderr; a
closed pipe is the reader's choice and SHALL be quiet.

`query` subcommands and `coverage` never exit 1: they produce no findings, so their
exit codes are 0 (answered) or 2 (could not run or could not pose the question) only —
the three-valued contract stays with `validate`, `summary`, `trace`, and `fuse`. A register
from which the adapter read no node while reporting an error-severity issue is a
question that cannot be posed (see the `query` capability).

Verified by: `cargo test --test cli exit`, `cargo test --test cli an_unwritable_stdout_exits_two`, and `cargo test --test query exit`

#### Scenario: Stdout cannot be written
- **WHEN** `latt validate` writes its report to a full device
- **THEN** lattice says it could not write output on stderr and exits 2

#### Scenario: Clean validation
- **WHEN** validation produces zero errors
- **THEN** exit code is 0

#### Scenario: Errors found
- **WHEN** validation produces errors
- **THEN** exit code is 1

#### Scenario: Suppressed errors do not gate
- **WHEN** validation produces error-severity findings and every one of them is
  suppressed by the profile
- **THEN** exit code is 0

#### Scenario: One unsuppressed error still gates
- **WHEN** validation produces two error-severity findings and the profile
  suppresses one of them
- **THEN** exit code is 1

#### Scenario: Strict-promoted then suppressed does not gate
- **WHEN** `--strict` promotes a warning to error and that finding is suppressed
- **THEN** exit code is 0

#### Scenario: Configuration failure
- **WHEN** `--adapter` names a program that cannot be run
- **THEN** exit code is 2, not 1

#### Scenario: Adapter process failure
- **WHEN** the adapter program exits non-zero, or writes output that does not satisfy the
  interface
- **THEN** exit code is 2, not 1, and no findings are reported

#### Scenario: Query never exits 1
- **WHEN** a `query` subcommand runs against a register whose adapter emitted error-severity issues
  beside at least one node
- **THEN** the exit code is 0, with the issues reported on stderr

#### Scenario: Query against an unreadable register
- **WHEN** a `query` subcommand other than `at` or `diff`, or `coverage`, runs against a
  register whose adapter emitted an error-severity issue and no node
- **THEN** no answer is printed and the exit code is 2

#### Scenario: Coverage never exits 1
- **WHEN** `latt coverage` runs against a register whose adapter emitted error-severity issues
  beside at least one node
- **THEN** the exit code is 0, with the issues reported on stderr

### Requirement: Adapter loading
The `--adapter` flag SHALL be optional. When supplied, it names either a known
native adapter by name or an external executable program path. Core SHALL
resolve which adapter to use via the precedence defined in the
`builtin-adapter-dispatch` capability.

Core SHALL reject output that does not satisfy the interface schema rather than
attempting to validate a partial graph. See the `adapter-contract` capability
for the document's shape and failure modes.

Verified by: `cargo test --test cli adapter` and `cargo test --test native`

#### Scenario: Adapter program runs
- **WHEN** `--adapter ./adapters/openspec` is passed and the program is executable
- **THEN** lattice runs it with the profile and target path and ingests its output

#### Scenario: Adapter program not found
- **WHEN** `--adapter ./nonexistent` is passed
- **THEN** lattice exits 2 with an error message naming the program

#### Scenario: Adapter emits nothing
- **WHEN** the adapter program exits 0 having written no output
- **THEN** lattice exits 2 rather than reporting an empty register

#### Scenario: Adapter output is the wrong shape
- **WHEN** the adapter writes a document that does not satisfy the interface schema
- **THEN** lattice exits 2 with an error naming the schema failure

#### Scenario: No adapter specified and no profile native
- **WHEN** `--adapter` is omitted and the profile has no `adapter.native` key
- **THEN** lattice exits 2 naming both resolution paths

### Requirement: Summary rejects a mistyped config
When the profile's SUMMARY config carries a non-string value for `node_kind`,
`status_attr`, or `group_by_attr`, `latt summary` SHALL exit 2 with an error
naming the key and the value's type. It SHALL NOT roll up nothing at exit 0 — an
empty rollup produced by a mistyped config is silence about a declared config.

Verified by: `cargo test --test cli summary_config`

#### Scenario: Non-string summary config value
- **WHEN** the profile's SUMMARY config sets `node_kind: 123` and `lattice
  summary` runs
- **THEN** lattice exits 2 and stderr names `node_kind` and `int`

### Requirement: Summary rejects a status named total
Each rollup row keeps its per-status counts and its `total` in one map, so when a status
column would be named `total` — declared in the status attr's enum or held by a node of
the rolled-up kind — `latt summary` SHALL exit 2 naming the value and the attr,
rather than count those nodes twice under one column. A node with no value for the
status attr counts under `unknown`, the same column as a node whose value is literally
`unknown`. Status columns are sorted.

Verified by: `cargo test --test cli a_status_named_total_exits_two_rather_than_double_counting`

#### Scenario: A node's status is total
- **WHEN** a rolled-up node carries `status: total` and `latt summary` runs
- **THEN** lattice exits 2 and stderr names `'total'`

### Requirement: Suggestions flag
`validate` SHALL accept `--suggestions <file>`, and SHALL accept it more than once. Each
occurrence names one suggestion document; the core reads each and renders its entries
alongside the findings it computed itself.

Absent the flag, behaviour is unchanged: no document is read and no suggestion is rendered.

Verified by: `cargo test --test suggestions flag`

#### Scenario: One document
- **WHEN** the user runs `latt validate --profile p.yaml --adapter ./a --target /repo --suggestions s.json`
- **THEN** the report carries the register's findings and the document's suggestions

#### Scenario: Several documents
- **WHEN** the flag is given twice naming two documents
- **THEN** the report carries the entries of both, each identifying the producer that emitted it

#### Scenario: Absent flag changes nothing
- **WHEN** `validate` runs without `--suggestions`
- **THEN** the output is byte-identical to the same invocation with every `--suggestions`
  occurrence removed — the overlay adds to a report and never alters the rest of it

### Requirement: The suggestions overlay never changes the exit code
Every finding the overlay produces SHALL be `hint` severity — both a rendered suggestion and
any complaint about a document's contents. The exit code of a run with `--suggestions` SHALL
equal the exit code of the same run without it, under every flag including `--strict`.

A document the core cannot use at all is a different matter and stays exit 2 — see the
`suggestion-document` capability. That is a broken setup, not a finding.

Verified by: `cargo test --test suggestions exit`

#### Scenario: Suggestions on a clean register
- **WHEN** the register validates clean and a document supplies twenty suggestions
- **THEN** the exit code is 0, with and without `--strict`

#### Scenario: Suggestions alongside real findings
- **WHEN** the register produces one error-severity finding and a document supplies suggestions
- **THEN** the exit code is 1, the same as without the overlay

### Requirement: An unresolvable suggestion is reported, never dropped
Where a suggestion names a `src` or `tgt` that is not a node in the ingested graph, the core
SHALL emit a `SUGGESTION_UNRESOLVED` finding naming the entry and the ID that did not resolve,
and SHALL continue with the remaining entries. It SHALL NOT drop the entry in silence.

The finding is `hint` severity.

Verified by: `cargo test --test suggestions unresolved`

#### Scenario: Stale target ID
- **WHEN** a document proposes an edge to a requirement ID that no longer exists
- **THEN** the report carries a `SUGGESTION_UNRESOLVED` hint naming that ID, and the document's
  other suggestions still render

#### Scenario: Every entry stale
- **WHEN** no ID in the document resolves
- **THEN** the report carries one `SUGGESTION_UNRESOLVED` hint per entry and the exit code is
  unchanged — never an empty, silent overlay

### Requirement: An unranked run is visible
Where a suggestion document carries a positive `unranked` count, the core SHALL emit one
`SUGGESTION_UNRANKED` finding naming the producer and the count, with provenance
`<suggestions>:0` and no `node_id`, beside whatever entries the document renders. A count of
zero, or a document with no count, SHALL add nothing.

The finding is `hint` severity, so it never changes the exit code, under `--strict` too.
An empty document from a producer that ranked nothing would otherwise read as "nothing to
suggest", which is the one answer the tool must never give.

Verified by: `cargo test --test suggestions unranked`

#### Scenario: A producer that ranked nothing
- **WHEN** a document carries no suggestions and `unranked: 3`
- **THEN** the report carries one `SUGGESTION_UNRANKED` hint naming the producer and 3, and the
  exit code is 0 with and without `--strict`

#### Scenario: Nothing left unranked
- **WHEN** a document carries `unranked: 0`, or is a `1.0` document without the key
- **THEN** the report carries no `SUGGESTION_UNRANKED` finding

#### Scenario: A partly ranked run
- **WHEN** a document carries suggestions and `unranked: 1`
- **THEN** the report carries the `SUGGESTED_EDGE` hints and one `SUGGESTION_UNRANKED` hint

### Requirement: Profile version pin
Every command that takes `--profile` SHALL accept an optional `--profile-version <X.Y.Z>`.
When it is given, lattice SHALL load the profile and then refuse it unless its
`profile_version` equals the pin exactly, before any adapter runs. The pin SHALL be held to
the `profile_version` shape (X.Y.Z, each component a non-negative integer without leading
zeros); a pin of any other shape is refused without comparing. Under that shape equal text
is equal version, so the comparison is on the values alone, with no ordering.

A refusal is a broken setup, not a finding: lattice SHALL report it on stderr as a
profile error naming the profile's version and the pin, write nothing to stdout, and exit 2,
never 1. Without the option, any version the profile loader accepts runs. `query diff` and
a `browse` reload of the profile honour the pin as the other commands do.

Verified by: `cargo test --test cli profile_version_pin`, `cargo test --test query pin`,
and `cargo test --test profile_schema pin`

#### Scenario: Matching pin
- **WHEN** the profile declares `profile_version: "1.0.0"` and `latt validate` runs with
  `--profile-version 1.0.0`
- **THEN** the run is the same as without the option

#### Scenario: Mismatched pin
- **WHEN** the profile declares `profile_version: "1.0.0"` and `validate`, `summary`,
  `trace`, `coverage`, a `query` or `show` runs with `--profile-version 1.1.0`
- **THEN** stderr names both `1.0.0` and `1.1.0`, stdout is empty, and the exit code is 2
  even when the register would have produced error-severity findings

#### Scenario: Malformed pin
- **WHEN** `--profile-version 1.0` is given
- **THEN** lattice reports that `1.0` is not a valid version and exits 2

#### Scenario: Diff honours the pin
- **WHEN** `latt query diff` runs with a `--profile-version` the profile does not declare
- **THEN** no revision is diffed, stdout is empty, and the exit code is 2

### Requirement: Show command
`latt show --profile <file>` SHALL print the resolved profile document to stdout and
exit 0, or report why the profile could not be loaded and exit 2. It takes no `--adapter` and
no `--target`: it reads no register and runs no adapter. It takes `--profile-version`, and a
profile the pin refuses prints no document.

The output is the resolved profile document verbatim, not routed through the output dispatcher.

Verified by: `cargo test --test cli show`

#### Scenario: Resolved document on stdout
- **WHEN** the user runs `latt show --profile path/to/profile.yaml`
- **THEN** stdout is the resolved profile document, carrying `resolved_schema`, and the exit
  code is 0

#### Scenario: An adapter runs on what show printed
- **WHEN** `latt show` output is saved and passed to an adapter program as its
  `--profile` argument
- **THEN** the adapter emits an interface document, the same one the core would have ingested

#### Scenario: A bad profile is exit 2
- **WHEN** the named profile does not load
- **THEN** the error is reported and the exit code is 2, never 1

### Requirement: Adapter command
`latt adapter <name> --profile <path> --target <dir>` SHALL run the native adapter
`<name>` and write its interface document as JSON to stdout. `--profile` names the
*resolved* profile document (`latt show` output), as the external-adapter contract
specifies. The command takes no `--adapter`, `--format` or `--strict`. It SHALL exit 0
when it writes a document, including a document that carries `PARSE_ERROR` findings. It
SHALL exit 2 with `Error: <detail>` on stderr and nothing on stdout when the adapter
cannot produce a document: the profile is unreadable or is YAML, or it has an
`adapter:` key the adapter does not read.

A `<name>` that is not a compiled native adapter SHALL exit 2 with the
native-availability guidance of `builtin-adapter-dispatch` › Native adapter names. The
command never resolves `<name>` through PATH.

Verified by: `cargo test --test native adapter_command` and
`cargo test --no-default-features --test dispatch adapter_command`

#### Scenario: Run an adapter by hand
- **WHEN** `latt show --profile profiles/md.yaml > p.json` and then
  `latt adapter md --profile p.json --target tests/fixtures/mini-md` run
- **THEN** stdout is the interface document for `mini-md` and the exit code is 0

#### Scenario: YAML profile
- **WHEN** `latt adapter md --profile profiles/md.yaml --target .` runs
- **THEN** it exits 2 with `could not load profile profiles/md.yaml` and the JSON
  reader's detail, and stdout is empty

#### Scenario: Unknown name
- **WHEN** `latt adapter yaml --profile p.json --target .` runs
- **THEN** it exits 2 naming `yaml` and listing the native adapters

#### Scenario: Build without native adapters
- **WHEN** a build without `native-adapters` runs `latt adapter md`
- **THEN** it exits 2 and the message names the disabled `native-adapters` feature

### Requirement: A missing profile names the profiles beside it
When `--profile` names no readable file, the command SHALL exit 2 as before, and the
message SHALL also list, in name order, each `.yaml` or `.yml` file in the named path's
directory (the working directory for a bare name) that holds a top-level `node_kinds:`
line, as candidates for the profile meant. When there is none, the message SHALL say
that no profile was found there. The search reads that one directory and descends into
none.

Verified by: `cargo test -p latt --locked --test cli missing_profile`.

#### Scenario: A misnamed profile
- **WHEN** `latt validate --profile lattice.yml --target .` runs in a directory holding
  `lattice.yaml`, which declares `node_kinds`, and `notes.yaml`, which does not
- **THEN** the exit code is 2 and the message names `lattice.yaml` and does not name
  `notes.yaml`

#### Scenario: No candidate
- **WHEN** `--profile` names a missing file in a directory with no YAML file declaring
  `node_kinds`
- **THEN** the exit code is 2 and the message says no profile was found in that directory
