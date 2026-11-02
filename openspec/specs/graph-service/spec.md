# graph-service Specification

## Purpose
Synchronous graph operations over borrowed host-owned input, with owned reports,
explicit input attribution and compatible CLI execution.
## Requirements
### Requirement: Supported borrowed graph execution
Lattice SHALL expose a synchronous supported service over borrowed coherent graph,
resolved profile and host context, returning owned reports for reach, path, orphans,
counts, coverage, at, validation, summary, trace and supplied-view comparison. It SHALL
reuse existing structural semantics, preserve public IDs, parallel edges, undeclared
endpoints and deterministic ordering, and SHALL NOT perform filesystem/network I/O,
mutate input, or retain cross-snapshot state during execution. Host orchestration SHALL
supply filesystem existence facts for at selection independently of graph execution.

Verified by: `cargo test -p latt --test service` and `cargo test --doc -p latt-core`.

#### Scenario: Reuse a borrowed view
- **WHEN** a host executes multiple operations over one coherent immutable view
- **THEN** each returns an owned report with existing operation semantics and the input remains unchanged

#### Scenario: Select an empty source path
- **WHEN** the host supplies a true path-existence fact and no provenance matches the selected path
- **THEN** at returns an empty successful answer without performing a filesystem probe

### Requirement: Supported service shapes admit additions
Every struct and enum in the supported service surface that may gain a field or variant
SHALL be non-exhaustive to code outside the crate: the response envelope, the service
error, input records and diagnostics, the input context, every report type the
service returns, the profile types the loader builds, the stored graph's nodes, edges
and pathways, the adapter selection types, and the output format. The hidden items
(the layouts and their results, the fuse
manifest and its sources, the fuse error's struct variants, and the composed fuse
report and its parts) are not supported surface, and SHALL be non-exhaustive all the
same, so promoting one later is not a break. Enums that name the two sides of a partition (a declared or vacant
endpoint, a real or virtual layout node) stay exhaustive. A condition SHALL be opaque:
its fields private, built through one constructor per operator or by parsing the
CLI's filter grammar, and read through accessors. Types the host builds as inputs by literal (the view, edge specs) are outside
this rule until a constructor is decided.
Errors that carry only a message SHALL expose it through an accessor, not a public
field; parsing a CLI-style filter SHALL fail with the filter error type, not a bare
string; the output format SHALL parse from its spelling through `FromStr` with a typed
error, list every format in a slice whose length is not part of its type, and not
implement a command-line parser's trait; and the crate SHALL NOT have two public enums of the same name. A host SHALL build an input context from its default through builder
methods. `Severity` SHALL stay exhaustive. The validate report SHALL be a struct
carrying its findings, so it can gain metadata later. Edge-kind filters SHALL be
borrowed string slices. A finding's sort key SHALL be an opaque ordered value, so a
new tiebreaker does not change its type. A trace entry's attrs SHALL use the same JSON
map type the trace JSON serializes. Converting a day count to a civil date SHALL clamp
the count to the representable range and never panic.

Verified by: `cargo test -p latt --locked`,
`cargo test -p latt-core --locked --test conditions`,
`cargo test --workspace --doc --locked`, and `./tools/verify-rust-tooling.sh`. The
semver-checks lints the change raises are listed under Breaking in the release that
shipped them and reconciled by `tools/check-breaking.sh`.

#### Scenario: Host builds an input context
- **WHEN** a host outside the crate builds an input context with an identity, a declared scope, a status and a date
- **THEN** it does so through `InputContext::default()` and the `with_*` builders, and the semver-checks log records `InputContext` as newly non-exhaustive

#### Scenario: Validate report keeps CLI bytes
- **WHEN** `latt validate` renders a validate report in plain, json or rich format
- **THEN** the output bytes equal the existing baselines, because the payload is still the findings list

#### Scenario: Edge-kind filter from a slice
- **WHEN** a host executes reach or path with edge kinds `["verifies"]` passed as a string slice
- **THEN** the answer equals the one the same kinds gave as a set before

#### Scenario: Extreme day counts
- **WHEN** a day count of `i64::MIN` or `i64::MAX` is converted to a civil date
- **THEN** the result is the earliest or latest representable date, with no panic

#### Scenario: Successive days
- **WHEN** two day counts inside the representable range differ by one
- **THEN** their dates are consecutive and in the same order

#### Scenario: Error text through an accessor
- **WHEN** a host reads the text of a contract error from ingestion or a pathway error
- **THEN** it calls `message()`, and the error's `Display` output is the same text as before

#### Scenario: Count direction is a distinct name
- **WHEN** code in the workspace reads an `EDGE_COUNT` configuration's counted end
- **THEN** it is a `CountEnd`, and `Direction` names only the traversal direction

#### Scenario: Filter parse error is typed
- **WHEN** a host parses the filter `no-operator`
- **THEN** it gets a filter error whose `message()` is `invalid filter 'no-operator': expected an attribute operator`, and the CLI prints the same text for a bad `--filter`

#### Scenario: Summary error text through an accessor
- **WHEN** the `latt` binary reads why a profile cannot be summarized
- **THEN** it calls `message()` on the summary error, now unstable, and the error's `Display` output is the same text as before

#### Scenario: Summary error text through the service
- **WHEN** a host executes `Request::Summary` against a profile that cannot be summarized
- **THEN** it gets a service error of kind `InvalidConfiguration` whose `message()` is the text `latt summary` prints for the same profile

#### Scenario: Format parses from its spelling
- **WHEN** a host parses `json` and `xml` as an output format
- **THEN** `json` yields the JSON format, `xml` fails with a parse-format error naming `xml`, and every format in `Format::ALL` parses back from its `as_str` spelling

#### Scenario: Format flag keeps its CLI bytes
- **WHEN** `latt validate --help` runs, and `latt validate` runs with `--format xml` or `--format jsn`
- **THEN** the help line and both errors, including the possible values and the similar-value tip, are byte-identical to those before the change

#### Scenario: A fuse that cannot start reports through a constructor
- **WHEN** `latt fuse` is given a manifest that does not exist
- **THEN** it prints a report built by `FuseReport::failed` holding one `FUSE_ERROR` finding from no source, and exits 2

#### Scenario: Host builds every filter operator
- **WHEN** a host outside the crate builds conditions for equality, inequality, membership, the four orderings, a regular-expression match given as pattern text, and presence
- **THEN** it does so through `Condition`'s constructors without naming a `regex` type, and `execute` answers each as the equivalent `--filter` or profile condition does

#### Scenario: Parsed and constructed conditions agree
- **WHEN** a host parses `status=done` and constructs the equality condition for attr `status` and value `"done"`
- **THEN** both select the same nodes in an orphans request

#### Scenario: A bad pattern is a typed error
- **WHEN** a host builds a regular-expression condition from the pattern text `[unclosed`
- **THEN** it gets a filter error whose `message()` names the pattern and the attr, then carries the regular-expression error the CLI prints for the same pattern given as `--filter`

### Requirement: Guard supported profile preparation
The service SHALL provide I/O-free profile-preparation entry points for an
already-parsed YAML value and for YAML text. Each SHALL reject any unresolved top-level
extends key with ProfileError before loading the value, naming inheritance resolution
as the cause. Each SHALL accept already-resolved profiles through existing value
validation. Text that does not parse as YAML SHALL be a ProfileError naming the origin
the host supplied. The value loader itself rejects a leftover `extends` as an unknown
top-level key; the entry points exist to name why the key is there. Hosts supplying a
Profile outside these entry points remain responsible for inheritance resolution. The
YAML value type in the value entry point's signature SHALL be re-exported at the crate
root, so a host names the same version the crate uses.

Verified by: `cargo test -p latt --test service`.

#### Scenario: Self-contained child still names a parent
- **WHEN** service preparation receives a profile with every required key and an extends key
- **THEN** preparation returns ProfileError naming inheritance resolution, without accessing the named path or silently dropping inherited content

#### Scenario: Already resolved profile
- **WHEN** service preparation receives a valid resolved profile without extends
- **THEN** it produces the same Profile as existing value validation

#### Scenario: Profile from text
- **WHEN** a host prepares the same resolved profile from its YAML text
- **THEN** it produces the same Profile as preparing the parsed value

#### Scenario: Unparseable profile text
- **WHEN** a host prepares profile text that is not valid YAML
- **THEN** preparation returns ProfileError naming the supplied origin

### Requirement: Separate input context diagnostics and execution
The service SHALL carry host-declared scope and input status explicitly, including
Unknown, without inferring completeness from diagnostic counts. Successful execution
SHALL mean completion over supplied input only. Recoverable input issues SHALL remain
structured diagnostics with input-side attribution; structural findings SHALL remain
report content. An empty answer SHALL succeed; inability to execute SHALL return a
typed error without a payload and with available input context/diagnostics. The service
SHALL NOT present partial execution as complete or model errors as process exit codes.
Envelope diagnostics SHALL preserve resolve_adapter_issues results without applying
strict promotion or SUPPRESS. Payload findings SHALL retain their operation-specific
validation policy. Consumers SHALL distinguish input attribution from payload findings
and SHALL NOT concatenate the channels into duplicate findings or severity totals.
Missing-path/unknown-ID errors SHALL be InvalidRequest; summary configuration errors
SHALL be InvalidConfiguration; returned path-construction invariant failures SHALL be
InternalInvariant, classified by origin rather than parsing message text.

Verified by: `cargo test -p latt --test service`.

#### Scenario: Unknown completeness with no diagnostics
- **WHEN** the host supplies Unknown input status and the operation has no diagnostics
- **THEN** the response retains Unknown while reporting completed execution

#### Scenario: Missing supplied-view node
- **WHEN** a query names a node absent from the supplied graph
- **THEN** the service returns InvalidRequest without claiming whether that node exists elsewhere

#### Scenario: Two-view diagnostics
- **WHEN** comparison receives recoverable input issues in each supplied graph
- **THEN** its response retains each issue with Before or After attribution and both input contexts

#### Scenario: Strict payload and adapter diagnostic
- **WHEN** strict validation receives a warning-severity adapter issue
- **THEN** its payload promotes the issue to Error while its envelope retains Warning as input attribution

#### Scenario: Suppressed payload and adapter diagnostic
- **WHEN** validation receives an adapter issue matching SUPPRESS
- **THEN** the payload marks it suppressed while the envelope retains its unsuppressed adapter status

### Requirement: Host authority and scope isolation
Hosts SHALL authorize input before ingestion, including attrs, edges, provenance and
diagnostics. Lattice SHALL operate only on supplied input and SHALL NOT attest host
identity, freshness, scientific conclusions or settlement. Indexes SHALL be bounded
to immutable input. Host context SHALL remain separate from Lattice compatibility
versions and SHALL NOT confer authorization merely by naming an actor.

Verified by: `cargo test -p latt-core --test embedding`; actual host
integration requires the pending host test tier recorded in the archived borrowed-graph-service acceptance matrix.

#### Scenario: Shared and independent readings
- **WHEN** the two authorized actor fixtures each execute counts and reverse reach
- **THEN** A contains one shared reading, B contains that reading and one independent reading, and multiple assertions do not synthesize extra reading nodes

#### Scenario: Actor-visible response
- **WHEN** actor A queries its authorized view
- **THEN** successful reports, diagnostics and host context contain no actor-B-only identifiers or attrs

### Requirement: Preserve existing CLI and provenance contracts
CLI transports SHALL retain live adapter loading, existing plain/json/rich payloads,
stdout/stderr placement and command-specific exit semantics when using the service.
Migration SHALL preserve existing CLI error text bytes even though Rust consumers
use error categories rather than parsing messages. Summary SHALL continue gating
on envelope adapter issues, not suppressed payload findings or aggregate counts;
answered queries SHALL continue exiting 0 regardless of payload findings.
Graph-only comparison SHALL NOT materialize Git revisions. Existing file/line
provenance SHALL keep its serialized form, a `file` string and a `line` integer, and
interface-document versions SHALL remain unchanged; the range a `line` may take is the
adapter-contract's "Document schema". New service
responses SHALL NOT silently replace or wrap existing serialized CLI payloads.

Verified by: `cargo test -p latt --test service_cli_compat`.

#### Scenario: Query with adapter errors
- **WHEN** a CLI query completes with error-severity adapter diagnostics
- **THEN** it retains its prior stdout answer, stderr diagnostics and exit 0

#### Scenario: Suppression does not change summary adapter exit
- **WHEN** an error-severity adapter issue matches a SUPPRESS entry and summary completes
- **THEN** summary retains the unsuppressed stderr diagnostic and exit 1

#### Scenario: Unknown node preserves CLI error
- **WHEN** a migrated CLI query names an unknown node
- **THEN** it retains the baseline stderr bytes and exit 2

### Requirement: Document the supported embedding surface
The service SHALL document supported entry points, reused input/report types, error
categories, and independent Rust/profile/interface/output compatibility policies.
Documentation SHALL explain the reserved complete-execution status independently of
input completeness and the owned host-context attribution. It SHALL state that
non-file locators remain host-owned and are not converted into editable file paths.
It SHALL state that a host declaring complete input status with an unknown scope gets
an InvalidRequest error at run time. It SHALL state that the re-exported YAML crate's
version and the profile accessors' ordered-map return types are committed, and that a
new request parameter or a new input-context field is a minor-version event.
Qualification SHALL cover Rust 1.99 Linux x86_64 with default and no-default features.
Unsupported targets, concurrency, cancellation and resource budgets SHALL NOT be
represented as qualified. Breaking supported pre-1.0 Rust API changes SHALL require
a minor version change, changelog and migration guidance.
The documentation SHALL list the supported module-level items of `latt-core` and
`latt-adapter`, and the unstable ones. Every item their rustdoc shows SHALL be
supported. An unstable or binary-internal item SHALL be `#[doc(hidden)]`, and no
supported signature SHALL name one.

Verified by: documentation inspection against `docs/embedding.md`, `cargo test`,
`cargo test --no-default-features`, `cargo doc -p latt-core --no-deps`, and
`crates/latt-core/tests/embedding_surface.rs`.

#### Scenario: Supported items are listed
- **WHEN** a supported item is removed, renamed or hidden, or the list names an item the crate does not export
- **THEN** the embedding surface test fails, under default and no-default features

#### Scenario: Existing CLI JSON consumer
- **WHEN** a consumer upgrades to the service-enabled implementation
- **THEN** its existing CLI JSON remains compatible independently of the new Rust service API

#### Scenario: Document a non-file host reference
- **WHEN** embedding documentation describes a host-owned event or database locator
- **THEN** it states the source-action limitation without promising a service conversion to file provenance

#### Scenario: Complete status without a scope
- **WHEN** a host reads the embedding documentation on input status
- **THEN** it learns that complete status with an unknown scope is rejected as InvalidRequest

### Requirement: Host-supplied validation date
The input context SHALL let the host supply the calendar date that validation compares
note `until` dates against, in every request that validates: Validate, Trace, At and
Summary. When the host supplies none, the service SHALL use the current UTC date, read
once per request, so one response never mixes two dates. The
date SHALL affect nothing but note expiry. A profile resolved from a value rather than
a file SHALL reject `annotations` as a profile error, because there is no file to
resolve the path against.

Verified by: `cargo test -p latt --locked --test service note`.

#### Scenario: Host date decides expiry
- **WHEN** a host supplies 2026-10-01 and a profile's attached note declares
  `until: 2026-09-30`
- **THEN** the validation payload carries `NOTE_EXPIRED` whatever the system clock reads

#### Scenario: Value-loaded profile with annotations
- **WHEN** a host prepares a profile from a value that declares `annotations`
- **THEN** preparation fails with a profile error naming `annotations`

### Requirement: A finding's provenance admits additions
The provenance the service reports on nodes, edges and the findings found in a file SHALL
be non-exhaustive to code outside the crate, built through its constructors (`new`, and
`new_file` for a file as a whole) and read through `file()` and `line()`, its fields
private. Its `file()` SHALL always name a file, never a reserved name. Its constructor's
line SHALL be an unsigned 32-bit number,
the type the adapter crate's provenance uses; zero SHALL mean the adapter knew no line.
`line()` SHALL return that line as an `Option<NonZeroU32>`, `None` for zero, so a host
reading the line must handle the whole-file case. `Display` writes the wire form,
`file:0` for that case, as the reports' JSON and plain output write zero. A profile's notes
entry SHALL be located the same way: `Profile::note_location(code, node)` SHALL return the
notes file and the entry's line as a provenance, `None` when no entry names that pair, and
a provenance whose `line()` is `None` when the entry has no line Lattice can name.

Verified by: `cargo test -p latt-core --locked --test provenance_line`,
`cargo test -p latt --locked --test finding_notes location`, and
`./tools/verify-rust-tooling.sh` with the Breaking bullets reconciled. The private
fields are lint-driven: `struct_pub_field_missing` and `struct_pub_field_now_doc_hidden`
on `Provenance.file` and `Provenance.line`, as `struct_marked_non_exhaustive` was. No
lint reports a changed parameter or return type, so the bullets for `Provenance::new`'s
line type and the removed `Profile::note_line` are written and checked by hand against this
requirement.

#### Scenario: A host reads a finding's line
- **WHEN** a host reads the line of a finding's provenance attributed to a source line
- **THEN** it is `Some` of a `NonZeroU32`, and `u32::from` gives the value earlier releases returned

#### Scenario: A host reads a whole-file finding's line
- **WHEN** a host reads the line of a provenance built with line zero
- **THEN** it is `None`

#### Scenario: A host locates a notes entry
- **WHEN** a host asks for the notes entry of a code on a node, and the notes file is a
  flow-style list
- **THEN** it gets the notes file with no line; for a block-style entry it gets the line its
  `-` is on; for a pair no entry names it gets `None`

#### Scenario: A host builds provenance
- **WHEN** a host outside the crate builds a provenance for `REQUIREMENTS.md` line 47
- **THEN** it calls `Provenance::new("REQUIREMENTS.md", 47)`, and a struct literal does not compile

### Requirement: A finding names its origin
A finding's `origin` SHALL be an `Origin`, non-exhaustive to code outside the crate:
`File` with the provenance of a place in a file the adapter read or named, or a variant
for an input Lattice reports on as a whole: `Profile`, `FuseProfile`, `FuseSource` (a
fuse source that could not run) and `Suggestions`. `Origin::location()` SHALL return the
provenance of a `File` and `None` for the others, so no host can read an input reported
as a whole as a path. `Issue::new` SHALL take the origin as `impl Into<Origin>`, and a
`Provenance` SHALL convert into `Origin::File`.

The reports SHALL write the four inputs as the reserved names `<profile>`,
`<fuse-profile>`, `<source>` and `<suggestions>` at line 0, byte for byte as before the
type existed, and `Display` SHALL write the same `file:line`. Fuse SHALL read those names
in a source's trace back into their variants; an adapter's interface document SHALL NOT
carry them, its findings being `File` origins whatever path they write. Sorting SHALL use
the written file and line, so the order of findings is unchanged.

Verified by: `cargo test -p latt-core --locked --test provenance_line origin`,
`cargo test -p latt --locked --test query at_`, and `./tools/verify-rust-tooling.sh`
with the Breaking bullets reconciled: `Issue.provenance` fails `struct_pub_field_missing`;
the `Issue::new` parameter is checked by hand, as no lint reports a parameter type.

#### Scenario: A profile finding names no file
- **WHEN** a SUPPRESS entry matches no finding
- **THEN** its `SUPPRESS_UNUSED` finding's origin is `Origin::Profile`, `location()` is
  `None`, and it displays as `<profile>:0`

#### Scenario: A file finding converts from a provenance
- **WHEN** a host builds `Origin::from(Provenance::new("r.md", 3))`
- **THEN** `location()` is that provenance and it displays as `r.md:3`

### Requirement: The supported embedding surface is service-first
The supported module-level items of `latt-core` SHALL be those a host needs to load a
profile, build a graph and ask the service a question:

- `service`: the requests, responses, input context, errors and entry points, and the
  only supported path to the report types and to the traversal `Direction`;
- `graph`: the stored graph and its nodes, edges, edge specs and pathways;
- `document`: graph construction from an adapter document or a native adapter run,
  with its errors;
- `profile`: loading, the profile and its error, the node-kind, edge-kind, attribute
  and pathway-binding types, and conditions with `Comparable` and the filter error;
- `validate::apply_suppressions` and `validate::apply_messages`, the profile policy a
  host applies to the envelope's unsuppressed diagnostics;
- `output`: the renderer with its payload, format and format error;
- the re-exported YAML crate, `suggest`'s version constants, and `document`'s interface
  version constants.

Each item SHALL be documented at exactly one path. A `service` re-export of an item
supported at its defining module SHALL be unstable, and so SHALL a defining module
whose items are supported only through `service`. Free functions whose answer
`service::execute` or `service::compare` returns, the generic graph algorithms and the
adjacency index, the profile's validation-configuration types and the `Profile`
accessors returning them, the condition operator enum, the query error, ANSI stripping,
the suggestion renderer, and trait implementations naming an unstable type SHALL be
unstable. In `latt-adapter`, the bounded subprocess runner SHALL be unstable.

No supported item's signature, field, variant payload, associated type or trait
implementation SHALL name an unstable or private item, or a type from a crate other
than the standard library, `serde`, `serde_json`, `serde_norway` and `latt-adapter`.

Verified by: `cargo test -p latt-core --locked --test embedding_surface` and
`cargo test -p latt-core --locked --no-default-features --test embedding_surface`
(the list); `cargo test -p latt-core --locked --test supported_signatures` (the
signature walk over rustdoc JSON for `latt-core` and `latt-adapter`, default and
no-default features); `cargo test -p latt --locked --test host_recipe` (the host
recipe against `latt validate`); `./tools/verify-rust-tooling.sh` with the Breaking
bullets reconciled.

#### Scenario: A report type has one documented path
- **WHEN** the `latt-core` rustdoc is generated
- **THEN** `ValidateReport` is documented under `service` and nowhere else, and `use latt_core::service::ValidateReport` compiles

#### Scenario: A defining-module item has one documented path
- **WHEN** the `latt-core` rustdoc is generated
- **THEN** `Profile` is documented under `profile` and not under `service`

#### Scenario: A hidden function stays callable
- **WHEN** the `latt` binary calls `query::reach`
- **THEN** it compiles and behaves as before, and `query::reach` is absent from the rustdoc

#### Scenario: Supported signatures name only supported items
- **WHEN** the signature walk runs over the rustdoc JSON of both crates
- **THEN** it reports no supported item naming an unstable item, a private item, or a type outside the listed crates

#### Scenario: The signature walk catches a leak
- **WHEN** a supported item's signature names an unstable item, as `impl From<&FuseReport> for Payload` and `ConditionOp::Matches(Regex)` did before this change
- **THEN** the signature walk fails naming the supported item and the item it names

#### Scenario: A host validates through the service
- **WHEN** a host loads a profile, runs its native adapter with `document::run_adapter_spec`, executes `Request::Validate`, and applies `apply_suppressions` and `apply_messages` to the envelope diagnostics
- **THEN** it uses supported items only, and its findings equal those `latt validate` prints for the same profile and target

