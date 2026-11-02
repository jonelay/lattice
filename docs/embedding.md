# Embedding Lattice

For Rust programs that hold a register in memory and want Lattice's validation and
queries as a library call instead of a `latt` run. Command-line users do not need
this page.

`latt_core::service` is the supported synchronous interface over a host-owned graph
and resolved profile. A call:

- borrows a coherent view of the graph and profile;
- performs no filesystem or network I/O;
- returns an owned report, which can outlive the graph and profile.

A call does not authorize the input, choose its freshness, or write derived state back
to the host.

## Add the dependency

The crates are not published to crates.io (`publish = false`). Depend on `latt-core`
through git or a local path, with `serde_json` to build the interface document the
examples ingest:

```toml
[dependencies]
latt-core = { git = "https://github.com/jonelay/lattice", tag = "v0.8.0" }
# or, from a checkout: latt-core = { path = "/path/to/lattice/crates/latt-core" }
serde_json = "1"
```

- The crate needs the Rust version `rust-toolchain.toml` pins in the checkout (1.99).
- `cargo doc -p latt-core --open` builds its API documentation.
- Installing the `latt` command installs nothing a Rust build can depend on.

## A first call

```rust
use latt_core::{document::ingest_document, service::*};

fn main() -> Result<(), Box<dyn std::error::Error>> {
let profile = prepare_profile_str(r#"
name: example
profile_version: "1.0.0"
node_kinds: {}
edge_kinds: {}
"#, "example")?;
let graph = ingest_document(serde_json::json!({
    "interface_version": "1.2", "nodes": [], "edges": []
}))?;
let context = InputContext::default().with_identity("snapshot-1");
let answer = execute(
    View { graph: &graph, profile: &profile, context: &context },
    Request::Counts { filters: &[] },
)?;
// Counts are about this supplied graph. Unknown completeness stays Unknown.
assert_eq!(answer.inputs[0].context.input_status, InputStatus::Unknown);
Ok(())
}
```

The examples on this page are the `latt_core::service` rustdoc and run as doctests.

## Snapshots and borrowing

A view used later keeps its input immutably borrowed. Changing the graph while a view
is live does not compile; the borrow checker rejects it with E0502:

```compile_fail,E0502
use latt_core::{graph::LatticeGraph, profile::Profile, service::*};
fn mutate(graph: &mut LatticeGraph, profile: &Profile, context: &InputContext) {
    let view = View { graph, profile, context };
    graph.add_issue(Issue::new(Severity::Warning, "PARSE_ERROR", "row",
                               Provenance::new("input.json", 1), None));
    execute(view, Request::Counts { filters: &[] }).unwrap();
}
```

The same function without the change compiles:

```
use latt_core::{graph::LatticeGraph, profile::Profile, service::*};
fn read_only(graph: &LatticeGraph, profile: &Profile, context: &InputContext) {
    let _ = Issue::new(Severity::Warning, "PARSE_ERROR", "row",
                       Provenance::new("input.json", 1), None);
    let view = View { graph, profile, context };
    execute(view, Request::Counts { filters: &[] }).unwrap();
}
```

Hosts may repeat calls on one immutable snapshot. They must replace that snapshot when
its source or authorization changes.

## Build the graph

| Your input | Call |
|---|---|
| Records the host holds | Write the interface document [adapters.md](https://github.com/jonelay/lattice/blob/v0.8.0/docs/adapters.md#writing-your-own-adapter) describes (`interface_version`, `nodes`, `edges`, each with a `provenance` object), then pass it to `document::ingest_document` |
| That document as text | `document::ingest_text`: the same graph and the same refusal messages, without first holding a JSON tree of the whole document |
| A register in a repository | `document::resolve_adapter`, then `document::run_adapter_spec`, which run an adapter in-process (below) |

Ingestion keeps the schema and version checks and the duplicate findings. A graph built
by hand proves nothing about correct interface ingestion.

To read a repository register:

- `resolve_adapter` picks the profile's native adapter, or the one the host names.
- `run_adapter_spec` runs it. It takes the resolved profile JSON from
  `profile::resolved_document` in place of the YAML text.
- The adapter's findings come back in the response envelope unsuppressed, with their
  adapter's text. Before showing or gating on them, apply the profile's SUPPRESS with
  `validate::apply_suppressions` and its declared `message`s with
  `validate::apply_messages`, as the CLI does:

```rust,no_run
use latt_core::document::{resolve_adapter, run_adapter_spec};
use latt_core::profile::{load_profile, resolved_document};
use latt_core::service::{InputContext, Request, View, execute};
use latt_core::validate::{apply_messages, apply_suppressions};
use std::path::Path;

let profile = load_profile(Path::new("lattice.yaml"))?;
let resolved = resolved_document(&profile)?;
let spec = resolve_adapter(None, &profile)?; // the profile's `adapter.native`
let graph = run_adapter_spec(&spec, &resolved, Path::new("."), profile.adapter_timeout())?;
let context = InputContext::default();
let mut answer = execute(
    View { graph: &graph, profile: &profile, context: &context },
    Request::Validate { strict: false },
)?;
apply_suppressions(answer.diagnostics.iter_mut().map(|d| &mut d.issue), &profile);
apply_messages(answer.diagnostics.iter_mut().map(|d| &mut d.issue), &profile, &graph);
let envelope_gates = answer.diagnostics.iter().any(|d| d.issue.gates());
# let _ = envelope_gates;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Prepare the profile

| Call | Use it for |
|---|---|
| `profile::load_profile` | a profile file in a repository; it resolves inheritance |
| `service::prepare_profile` | an already-resolved YAML value |
| `service::prepare_profile_str` | the text of an already-resolved profile |
| `Profile::require_version` | pinning an exact profile revision on the loaded profile, as the CLI's `--profile-version` does |

`prepare_profile` and `prepare_profile_str` reject a remaining `extends` key, so the
host resolves inheritance first. They also reject an `annotations` key, which needs a
profile file to resolve against.

## Run requests

The supported service entry points:

| Function | Does |
|---|---|
| `prepare_profile`, `prepare_profile_str` | build a `Profile` (above) |
| `execute` | answer one `Request` over a `View` |
| `compare` | compare two supplied graphs with opaque labels; it runs no Git and compares no authorization policies |

Inputs and results are documented by their public Rust types. Requests reuse the core's
existing semantics.

A `Request` is one of `Reach` (either direction), `Path`, `Orphans`, `Counts`,
`Coverage` (per-kind incidence coverage), `At` (file-provenance selection, below),
`Validate`, `Summary` or `Trace`.

- `Reach` and `Path` take their edge-kind filter as a `&[&str]`. Order and repeats do
  not matter, and an empty slice permits every kind.
- `Validate` answers with a `ValidateReport`, whose `findings` and `scope` are what the
  CLI renders.

Hosts must authorize both views of a `compare` for the recipient before disclosing their
difference.

**At.** `At` receives a lexical target, a path, and a host-provided existence fact. The
service only matches provenance and uses the fact.

- A repository host may compute the same path-existence probes as the CLI.
- A host without a filesystem source passes `Path::new("")` and false. Exact file
  strings and slash-delimited descendants can still match supplied provenance.
- Non-file locators remain in host mappings; generalized provenance is a separate
  contract.

## Render a report

`output::output_result(&answer.payload, format)` renders a Report with the existing
plain, json or rich formatting. It does not serialize the response envelope.

- `format` is an `output::Format`. It parses from its `--format` spelling with
  `str::parse`, failing with `output::ParseFormatError`.
- `Format::ALL` lists every format. A format can be added in a minor release, so a
  `match` on one needs a wildcard arm.

## Types

The service holds the report types and nested report entries, `Issue` with its `Note`,
`Until` and `CivilDate`, `Severity`, `Origin`, `Provenance` and `Direction`. The graph,
profile, condition and error types are used from `graph`, `profile` and `document`,
where they are defined. Which items are supported is listed under
[API stability](#api-stability).

| Type | Construct with | Read with |
|---|---|---|
| `Issue` | `Issue::new`, which takes the finding's origin as anything that converts into an `Origin`, a `Provenance` included | its fields |
| `Origin` | `Origin::File` with the provenance of a place in a file; `Profile`, `FuseProfile`, `FuseSource` or `Suggestions` for an input Lattice reports on as a whole | `origin.location()`: the provenance, or `None` for the whole-input variants |
| `Provenance` | `Provenance::new(file, line)`, where `line` is a `u32`, zero when no exact line is available; or `Provenance::new_file(file)` | `file()` and `line()`; `line()` is an `Option<NonZeroU32>`, `None` for that zero, so reading the line means handling the whole-file case |
| `InputContext` | `InputContext::default()` and its `with_*` methods | its fields |
| `Node` | (from the graph) | its fields; attr locations are private: read them with `attr_provenance()`, in attr-name order, `attr_location(attr)` or `provenance_of(attr)` |
| `profile::Condition` | a constructor per operator (`eq`, `ne`, `one_of`, `lt`, `gt`, `lte`, `gte`, `matches`, `present`), or `Condition::parse` for a CLI-style filter | opaque |

**Non-exhaustive types.** `Issue`, `Provenance`, `InputContext`, the response envelope,
`ServiceError` and every report struct are non-exhaustive. So are the profile types the
loader builds (`NodeKind`, `EdgeKind`, `AttrSchema`, `AttrType`, `PathwayBinding`) and
the stored graph's `Node`, `Edge` and `Pathway`. Read them through their public fields
and accessors, construct them as the table shows, match with `..`, and give a `match` on
one of their enums a wildcard arm.

**Origins in text.** The JSON writes the whole-input origins as `<profile>` and the
other reserved names ([json.md](json.md)). In Rust, match on the variant rather than on
that string. `Provenance`'s `Display` writes the wire form, `file:0` for the whole-file
case, as the CLI's plain output does; `Origin`'s `Display` writes `<profile>:0` the same
way.

**Conditions.** `Condition::parse` splits at the earliest operator, taking the longest
operator there. Each refusal is a `profile::FilterError` whose `message()` names it:

- `matches` refuses a pattern that is not a regular expression;
- `parse` refuses a filter with no operator, an empty attr name or a bad `~=` pattern.

Parsing and host loading remain outside execution.

## Context, diagnostics and errors

### Input context

- The default input context has Unknown scope and Unknown input status.
- `InputContext::with_today` fixes the date finding notes expire against. Without it,
  each request reads the current UTC date once.
- Declared scope can be bounded. `CompleteForDeclaredScope` is valid only with a
  declared scope. Building it with an Unknown scope succeeds, but every request over it
  then fails with `ErrorKind::InvalidRequest`: the check runs per request.
- Context is copied into reports and errors for attribution and ownership across host
  tasks. Lattice does not attest it or use it as a cache key.
- An empty `diagnostics` never establishes complete input.
- `ExecutionStatus::Complete` says the operation finished over supplied input. It does
  not upgrade Unknown or Partial input status. `Complete` is its only variant; the enum
  is `#[non_exhaustive]` so a future status can be added without an API break. No
  partial execution result is currently supported.

### Two finding channels

Envelope diagnostics, the response's `diagnostics` field, hold exactly the adapter's
findings, each an `Issue` beside the input side it came from. They are resolved:
overrides and pathway resolution apply; strict promotion and SUPPRESS do not.

Validation-derived payload findings keep their own policy, so their severity and
suppression can differ. A warning adapter finding can be Error in a strict validation
payload and Warning in the envelope. Do not concatenate both channels into one findings
list or double-count their severities.

| Operation | Findings/display and CLI policy |
|---|---|
| Validate, Trace | Payload findings control display and gating; the envelope is input attribution |
| At | Findings inside the answer do not gate; the CLI also reports the adapter's findings on stderr |
| Summary | The payload is a rollup/aggregate; the CLI applies the profile's SUPPRESS to the adapter's findings, then exits 1 only on one that still gates, as `validate` would |
| Other queries, coverage, compare | Answer plus separate envelope diagnostics; a completed CLI query exits 0 (exception below) |

Where the service and the CLI differ for those other queries, a host applies the CLI's
rules itself:

- When the graph has no node and an adapter finding gates, the CLI withholds the answer
  and exits 2: the register could not be read (for compare, at either revision). The
  service still returns the answer.
- The CLI names on stderr any filter attr that no node carries. The service does not,
  and the answer is unchanged.

The envelope never carries `suppressed`. The CLI applies the profile's SUPPRESS to it
before printing and gating; a host that shows or gates on envelope diagnostics applies
the same policy itself with `validate::apply_suppressions`. Preserve these distinctions
in integrated displays. Lattice does not gate on findings; hosts own that policy.

### Errors

An empty answer succeeds. Preparation returns `ContractError` or `ProfileError`;
execution returns `ServiceError`.

| Error | When | What it carries |
|---|---|---|
| `ContractError`, `graph::PathwayError` | preparation (`ContractError`); `graph::check_pathway` (`PathwayError`) | only an explanation, read through `message()` or `Display` |
| `ProfileError` | preparation | for `ProfileError::Yaml`, `Display` names only the file; the parser's line and column are on its `source()` |
| `ServiceError` with `ErrorKind::InvalidRequest` | unknown IDs or kinds, absent At paths, invalid context combinations | input contexts and available diagnostics; no partial payload |
| `ServiceError` with `ErrorKind::InvalidConfiguration` | summary configuration failures | the same |
| `ServiceError` with `ErrorKind::InternalInvariant` | returned path-construction invariant failures | the same |

- Error text is for people and may change editorially. Do not parse it; branch on the
  `ServiceError`'s kind.
- A host showing a `ProfileError` walks the `source()` chain, as the CLI's `caused by:`
  line does.
- The CLI preserves its existing message bytes and exit-code rules. The library does not
  use process exit codes as an error type.

### Authorization

- Authorize nodes, edges, attrs, diagnostics and provenance before constructing the
  view. An actor label alone does not authorize anything.
- Error text can echo a requested ID. A host that must not reveal whether a hidden ID
  exists normalizes the error text it shows its users.
- A no-path answer in a bounded view says nothing about records the view left out.
- What an answer means in the host's domain, and who may see or replay it, stay with
  the host.

## API stability

Every item `latt-core` and `latt-adapter` show in their rustdoc is supported: its
removal or an incompatible change is a breaking change, recorded under Breaking in
`CHANGELOG.md`. Items marked `#[doc(hidden)]` are absent from the rustdoc and carry no
promise.

The list below names the module-level items. A listed type's documented methods, fields
and variants are supported with it. A repository test imports every one of them and
checks that this list names the same items.

### Supported items

- `latt_core`: `serde_norway`
- `latt_core::document`: `INTERFACE_VERSION`, `SUPPORTED_INTERFACE_VERSIONS`, `ContractError`, `ResolveError`, `parse_document`, `ingest_document`, `ingest_text`, `AdapterSpec`, `NativeAdapter`, `resolve_adapter`, `run_adapter_spec`
- `latt_core::graph`: `Pathway`, `PathwayError`, `check_pathway`, `DuplicateNode`, `Node`, `Edge`, `EdgeSpec`, `LatticeGraph`
- `latt_core::output`: `Format`, `ParseFormatError`, `Payload`, `output_result`
- `latt_core::profile`: `ProfileError`, `AttrType`, `AttrSchema`, `NodeKind`, `EdgeKind`, `PathwayBinding`, `Comparable`, `Condition`, `FilterError`, `Profile`, `resolved_document`, `load_profile`, `load_profile_value`
- `latt_core::service`: `Direction`, `AtReport`, `CivilDate`, `CountsReport`, `CoverageReport`, `DiffReport`, `EdgeChangedRef`, `EdgeRef`, `FindingTally`, `Issue`, `KindCoverage`, `NodeChangedRef`, `NodeRef`, `Note`, `Origin`, `OrphanEntry`, `OrphansReport`, `PathReport`, `PathwayEntry`, `Provenance`, `ReachReport`, `Severity`, `StatusRollup`, `StructuralSummary`, `SummaryReport`, `TraceEdge`, `TraceEntry`, `TraceReport`, `Until`, `ValidateReport`, `ValidateScope`, `View`, `InputContext`, `Scope`, `InputStatus`, `ExecutionStatus`, `InputSide`, `InputDiagnostic`, `InputRecord`, `Response`, `Request`, `Report`, `ErrorKind`, `ServiceError`, `prepare_profile`, `prepare_profile_str`, `execute`, `compare`
- `latt_core::suggest`: `SUGGESTION_VERSION`, `SUPPORTED_SUGGESTION_VERSIONS`
- `latt_core::validate`: `apply_messages`, `apply_suppressions`
- `latt_adapter`: `decode_utf8`, `decode_lines`, `InvalidLine`, `INTERFACE_VERSION`, `Document`, `Node`, `Edge`, `Pathway`, `Severity`, `Issue`, `Provenance`
- `latt_adapter::codes`: `PARSE_ERROR`, `PATHWAY_INVALID`, `EXTERNAL_REF`, `SOURCE_MISSING`, `NO_MATCHING_FILES`, `WRAPPED_REF`, `AMBIGUOUS_REF`, `UNREAD_ENTRIES`, `ALL`
- `latt_adapter::fence`: `FenceState`

Each item is documented at one path. The report types, `Direction` and the entry
points live in `service`. `ContractError`, `LatticeGraph`, `Profile`, `ProfileError`,
`Comparable` and `Condition` live in the module that defines them; `service`'s
re-exports of them are hidden.

The workspace's `latt-trustfall` crate, which cross-checks the validators against
Trustfall, is development-only: it is neither published nor installed, and its own API
is not supported.

## Compatibility

The service brings no async runtime, durable cache, index shared across snapshots or
wire envelope.

These are independent compatibility axes: Rust API, profile schema and version,
interface version, report JSON and host metadata. Before 1.0, a break in the supported
Rust API is a minor version bump, with a changelog entry and migration guidance.

Some of the Rust API is committed in ways its types do not show:

| Item | Commitment |
|---|---|
| `serde_norway` | re-exported at the crate root because `prepare_profile` takes its `Value`; a move off its 0.9 line is a breaking change |
| `serde_json` | committed to its 1.x line: `ingest_document`, `load_profile_value`, `Condition::eq` and the attr maps name its `Value` |
| `regex` | uncommitted: `Condition::matches` takes pattern text, and no supported item names a `regex` type |
| `Profile`'s accessors | return `&BTreeMap`; deterministic output relies on their sorted order, so that type is committed |
| `serde_json::Map` iteration order | uncommitted: a host whose dependencies enable `serde_json/preserve_order` sees attrs in insertion order, but `output_result`'s JSON sorts them by key either way |
| a new `Request` parameter, a new `Scope::Declared` field | a minor-version event: each changes a variant hosts construct |
| a new `InputContext` field | a minor-version event, additive only because the struct is non-exhaustive |

The service is tested on Rust 1.99 on Linux `x86_64`, with default and no-default
features. A call is single-threaded and has no cancellation or resource budget: a host
that needs hard limits runs calls behind its own worker boundary. Lattice's own tests
use synthetic fixtures, so a host's authorization needs the host's own tests.
