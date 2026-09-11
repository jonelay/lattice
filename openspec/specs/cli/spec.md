# cli Specification

## Purpose
The CLI entry point — subcommand structure, format flag, profile selection,
and exit code semantics.
## Requirements
### Requirement: CLI entry point
`lattice` SHALL be a CLI with subcommands. The subcommands are `validate`,
`summary`, `trace`, `coverage`, and the `query` family; all take `--profile`,
`--adapter`, `--target`, and `--format`. `validate` and `trace` additionally
take `--strict`; `summary`, `coverage`, and `query` do not — `summary` runs no
validation pass, and only error-severity adapter issues affect its exit code;
`coverage` is a report that produces no findings; `query` produces no findings
at all, so no severity affects its exit code (see the `query` capability).

The adapter issues `summary` reports and exits on SHALL carry resolved severities —
profile overrides, then pathway demotion — the same severities `validate` would report for
the same graph. Running no validation pass means `summary` computes no findings of its
own; it does not mean it reports an adapter issue at a severity the profile has already
said is wrong.

#### Scenario: Run validate
- **WHEN** the user runs `lattice validate --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, validates, and reports findings

#### Scenario: Run summary
- **WHEN** the user runs `lattice summary --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, and reports the configured status rollup

#### Scenario: Run trace
- **WHEN** the user runs `lattice trace --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, validates the graph, and outputs the trace report

#### Scenario: Run coverage
- **WHEN** the user runs `lattice coverage --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, and reports per-kind coverage statistics

#### Scenario: Run query
- **WHEN** the user runs `lattice query counts --profile path/to/profile.yaml --adapter ./path/to/adapter --target /path/to/repo`
- **THEN** lattice loads the profile, runs the adapter program and ingests its document, and reports the answer

#### Scenario: Summary honours a profile severity override on an adapter code
- **WHEN** the profile overrides an error-severity adapter code to `info` and `summary` runs
- **THEN** `summary` reports that issue at `info` and exits 0

#### Scenario: Summary honours a pathway demotion
- **WHEN** a pathway binding demotes an error-severity adapter issue to `info` and `summary`
  runs
- **THEN** `summary` reports that issue at `info` and exits 0, agreeing with `validate`

### Requirement: Version flag
`lattice --version` SHALL print the installed package version and exit 0. The version
SHALL come from the installed package metadata, the same source as the trace report
header, so the two cannot disagree.

#### Scenario: Version flag
- **WHEN** the user runs `lattice --version`
- **THEN** lattice prints the package version and exits 0

### Requirement: Format flag
All subcommands SHALL accept `--format=plain|json|rich` (default: `rich` when TTY,
`plain` otherwise).

#### Scenario: Auto-detect format
- **WHEN** `lattice validate` runs with stdout connected to a TTY and no `--format` flag
- **THEN** output uses rich format

#### Scenario: Pipe auto-detect
- **WHEN** `lattice validate` output is piped (not a TTY) and no `--format` flag
- **THEN** output uses plain format

### Requirement: Exit codes
Lattice SHALL use three exit codes: 0 when the command succeeded with no errors, 1 when
the register produced error-severity findings, and 2 when lattice could not run the
command at all (unreadable profile, adapter program that cannot be run, adapter that
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

`query` subcommands and `coverage` never exit 1: they produce no findings, so their
exit codes are 0 (answered) or 2 (could not run or could not pose the question) only —
the three-valued contract stays with `validate`, `summary`, and `trace`.

Verified by: `cargo test --test cli exit` and `cargo test --test query exit`

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
- **THEN** the exit code is 0, with the issues reported on stderr

#### Scenario: Coverage never exits 1
- **WHEN** `lattice coverage` runs against a register whose adapter emitted error-severity issues
- **THEN** the exit code is 0, with the issues reported on stderr

### Requirement: Adapter loading
The `--adapter` flag SHALL name an executable program. Core SHALL run it with the profile
and target path, read a serialized graph document from its stdout, and ingest that
document. Core SHALL NOT import adapter code into its own process.

Core SHALL reject output that does not satisfy the interface schema rather than attempting
to validate a partial graph. See the `adapter-contract` capability for the document's
shape and failure modes.

Verified by: `cargo test --test cli adapter`

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

### Requirement: Summary rejects a mistyped config
When the profile's SUMMARY config carries a non-string value for `node_kind`,
`status_attr`, or `group_by_attr`, `lattice summary` SHALL exit 2 with an error
naming the key and the value's type. It SHALL NOT roll up nothing at exit 0 — an
empty rollup produced by a mistyped config is silence about a declared config.

Verified by: `cargo test --test cli summary_config`

#### Scenario: Non-string summary config value
- **WHEN** the profile's SUMMARY config sets `node_kind: 123` and `lattice
  summary` runs
- **THEN** lattice exits 2 and stderr names `node_kind` and `int`

### Requirement: Suggestions flag
`validate` SHALL accept `--suggestions <file>`, and SHALL accept it more than once. Each
occurrence names one suggestion document; the core reads each and renders its entries
alongside the findings it computed itself.

The flag is repeatable because ranking is a job with several producers, and combining their
rankings is a scoring decision. A core that merged them would be choosing between rankers,
which is exactly the judgement it must leave to the sidecar and the reviewer.

Absent the flag, behaviour is unchanged: no document is read and no suggestion is rendered.

Verified by: `cargo test --test suggestions flag`

#### Scenario: One document
- **WHEN** the user runs `lattice validate --profile p.yaml --adapter ./a --target /repo --suggestions s.json`
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

A suggestion is a review candidate, not a verdict. An overlay that could fail a run would make
lattice the authority on a relationship no human has confirmed, and would make advice
indistinguishable from the register's own findings.

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

A suggestion document is scratch and goes stale as soon as the register moves under it. Silence
would read as "this ranker had nothing to say", which is the one answer that must never be
faked. The finding is `hint`, because a stale scratch file is not a fault in the register.

Verified by: `cargo test --test suggestions unresolved`

#### Scenario: Stale target ID
- **WHEN** a document proposes an edge to a requirement ID that no longer exists
- **THEN** the report carries a `SUGGESTION_UNRESOLVED` hint naming that ID, and the document's
  other suggestions still render

#### Scenario: Every entry stale
- **WHEN** no ID in the document resolves
- **THEN** the report carries one `SUGGESTION_UNRESOLVED` hint per entry and the exit code is
  unchanged — never an empty, silent overlay

### Requirement: Resolve command
`lattice resolve --profile <file>` SHALL print the resolved profile document to stdout and
exit 0, or report why the profile could not be loaded and exit 2. It takes no `--adapter` and
no `--target`: it reads no register and runs no adapter.

The document is the same handoff the core already writes to a scratch file when it runs an
adapter. Exposing it is what makes a register-consuming program runnable by hand — an adapter
invoked directly, or a suggestion producer, which needs `summary_attr` and the profile's
allowed endpoint pairs and cannot get either from an interface document. Without it, profile
resolution would have to be reimplemented outside the core, which is the duplication the
resolved-document handoff exists to prevent.

It is printed verbatim rather than through the output dispatcher. The dispatcher renders
reports in three formats for a reader; this is a machine handoff whose shape the
profile-schema capability owns, and a `rich` rendering of it would mean nothing.

Verified by: `cargo test --test cli resolve`

#### Scenario: Resolved document on stdout
- **WHEN** the user runs `lattice resolve --profile path/to/profile.yaml`
- **THEN** stdout is the resolved profile document, carrying `resolved_schema`, and the exit
  code is 0

#### Scenario: An adapter runs on what resolve printed
- **WHEN** `lattice resolve` output is saved and passed to an adapter program as its
  `--profile` argument
- **THEN** the adapter emits an interface document, the same one the core would have ingested

#### Scenario: A bad profile is exit 2
- **WHEN** the named profile does not load
- **THEN** the error is reported and the exit code is 2, never 1
