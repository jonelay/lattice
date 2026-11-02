//! The supported synchronous interface: queries and reports over a graph the
//! host already holds, with input attribution.
//!
#![doc = include_str!("../../../docs/embedding.md")]
#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
use std::path::Path;

#[doc(hidden)]
pub use crate::document::ContractError;
#[doc(hidden)]
pub use crate::graph::LatticeGraph;
#[doc(hidden)]
pub use crate::profile::{Comparable, Condition, ConditionOp, Profile, ProfileError};
pub use crate::query::Direction;
pub use crate::types::{
    AtReport, CivilDate, CountsReport, CoverageReport, DiffReport, EdgeChangedRef, EdgeRef,
    FindingTally, Issue, KindCoverage, NodeChangedRef, NodeRef, Note, Origin, OrphanEntry,
    OrphansReport, PathReport, PathwayEntry, Provenance, ReachReport, Severity, StatusRollup,
    StructuralSummary, SummaryReport, TraceEdge, TraceEntry, TraceReport, Until, ValidateReport,
    ValidateScope,
};
use crate::{profile, query, summary, trace, validate};

#[derive(Clone, Copy, Debug)]
/// A coherent immutable graph, profile, and host-declared input context.
pub struct View<'a> {
    /// The already-authorized graph to query.
    pub graph: &'a LatticeGraph,
    /// The resolved profile governing graph semantics.
    pub profile: &'a Profile,
    /// Host attribution copied into the response.
    pub context: &'a InputContext,
}

/// Host-supplied identity, scope, and completeness metadata for one input.
///
/// Build it from [`InputContext::default`] and the `with_*` methods; a new
/// field is then a minor-version addition rather than a break.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InputContext {
    /// Opaque host-safe identity for this snapshot, when one is available.
    pub identity: Option<String>,
    /// The records the host declares this view to cover.
    pub scope: Scope,
    /// The host's completeness claim for the declared scope.
    pub input_status: InputStatus,
    /// The date validation compares finding-note `until` dates against.
    /// `None` reads the current UTC date once per request.
    pub today: Option<CivilDate>,
}

impl InputContext {
    /// Return the context with an opaque, host-safe snapshot identity.
    #[must_use]
    pub fn with_identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    /// Return the context with the records the host declares it to cover.
    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// Return the context with the host's completeness claim.
    ///
    /// [`InputStatus::CompleteForDeclaredScope`] needs a [`Scope::Declared`]:
    /// with [`Scope::Unknown`] every request fails as
    /// [`ErrorKind::InvalidRequest`]. The check runs per request, not here.
    #[must_use]
    pub fn with_status(mut self, status: InputStatus) -> Self {
        self.input_status = status;
        self
    }

    /// Return the context with the date note expiry is judged against.
    #[must_use]
    pub fn with_today(mut self, today: CivilDate) -> Self {
        self.today = Some(today);
        self
    }
}

/// Host-declared scope of the supplied graph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Scope {
    /// No scope boundary is declared.
    #[default]
    Unknown,
    /// A named host scope, optionally known to be bounded.
    Declared {
        /// Opaque, display-safe scope label.
        label: String,
        /// Whether records outside this scope are deliberately excluded.
        bounded: bool,
    },
}

/// Host-declared completeness of an input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputStatus {
    /// Completeness is not known.
    #[default]
    Unknown,
    /// The input is complete for its accompanying declared scope.
    CompleteForDeclaredScope,
    /// The input is known to omit records from its declared or unknown scope.
    Partial,
}

/// Completion of the operation, independent of supplied-input completeness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExecutionStatus {
    /// The operation finished over all supplied input.
    Complete,
}

/// Role of an input in a request or comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputSide {
    /// The sole input to a non-comparison request.
    Current,
    /// The earlier input to a comparison.
    Before,
    /// The later input to a comparison.
    After,
}

/// An adapter issue attributed to one input side.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InputDiagnostic {
    /// The input that produced the issue.
    pub side: InputSide,
    /// The resolved adapter issue.
    pub issue: Issue,
}
/// Host context attributed to one input side.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InputRecord {
    /// The input represented by this record.
    pub side: InputSide,
    /// The host-declared context copied from the view.
    pub context: InputContext,
}
/// Owned result envelope returned by a completed service operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Response<T> {
    /// Operation-specific result.
    pub payload: T,
    /// Context records for every supplied input.
    pub inputs: Vec<InputRecord>,
    /// Resolved adapter diagnostics, kept separate from payload findings.
    pub diagnostics: Vec<InputDiagnostic>,
    /// Whether execution completed over the supplied input.
    pub execution: ExecutionStatus,
}

/// A synchronous operation over one [`View`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Request<'a> {
    /// Find the transitive closure from an origin node, excluding the origin.
    ///
    /// The walk follows edge endpoints as written, so it passes through an
    /// endpoint no adapter declared; only declared nodes enter the answer.
    /// An unknown origin or an edge kind the profile does not declare is a
    /// request error, not an empty answer.
    Reach {
        /// Node ID at which traversal begins.
        origin: &'a str,
        /// Direction in which edges are followed.
        direction: Direction,
        /// Allowed edge kinds; empty permits every kind. Order and repeats
        /// do not matter.
        edge_kinds: &'a [&'a str],
        /// Conditions every returned node must satisfy. They select from the
        /// closure after the walk and never cut it: a node behind one that
        /// fails them is still reached.
        filters: &'a [Condition],
        /// Whether to mark each reached node tainted when *every* path to it
        /// crosses an undeclared endpoint; one clean path leaves it untainted.
        check_resolved: bool,
    },
    /// Find one shortest path from `src` to `tgt` along outgoing edges.
    ///
    /// Among paths of equal length, the one through lexicographically earlier
    /// neighbours wins, so the answer is deterministic. A pair with no path
    /// is a not-found report, not an error; an unknown endpoint or an edge
    /// kind the profile does not declare is a request error.
    Path {
        /// Starting node ID.
        src: &'a str,
        /// Destination node ID.
        tgt: &'a str,
        /// Allowed edge kinds; empty permits every kind. Order and repeats
        /// do not matter.
        edge_kinds: &'a [&'a str],
    },
    /// Find declared nodes not named by any edge.
    Orphans {
        /// Optional node-kind restriction.
        kind: Option<&'a str>,
        /// Conditions every returned node must satisfy.
        filters: &'a [Condition],
    },
    /// Count nodes and edges by kind.
    Counts {
        /// Conditions nodes must satisfy before contributing to counts.
        filters: &'a [Condition],
    },
    /// Calculate per-kind incoming and outgoing edge coverage.
    Coverage,
    /// Select trace entries attributed to a source path.
    At {
        /// Host target root used for lexical path matching.
        target: &'a Path,
        /// Source path to select.
        path: &'a str,
        /// Host-provided result of its path-existence check.
        host_path_exists: bool,
        /// Conditions every selected node must satisfy.
        filters: &'a [Condition],
    },
    /// Run profile validation.
    Validate {
        /// Whether warnings are promoted within validation payload findings.
        strict: bool,
    },
    /// Build the configured or structural summary.
    Summary,
    /// Build the full trace payload.
    Trace {
        /// Whether warnings are promoted within trace payload findings.
        strict: bool,
    },
}

/// Owned payload returned by [`execute`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Report {
    /// Reachability result.
    Reach(ReachReport),
    /// Shortest-path result.
    Path(PathReport),
    /// Orphan selection result.
    Orphans(OrphansReport),
    /// Per-kind counts.
    Counts(CountsReport),
    /// Per-kind edge coverage.
    Coverage(CoverageReport),
    /// Source-path trace selection.
    At(AtReport),
    /// Validation findings.
    Validate(ValidateReport),
    /// Configured or structural summary.
    Summary(SummaryReport),
    /// Full trace report.
    Trace(TraceReport),
}

/// Machine-readable category of a service failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The request or its input context is invalid.
    InvalidRequest,
    /// The resolved profile cannot configure the requested operation.
    InvalidConfiguration,
    /// A core invariant failed while constructing a result.
    InternalInvariant,
}
/// Failed service operation with retained input attribution and diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
#[non_exhaustive]
pub struct ServiceError {
    /// Stable error category; callers should branch on this rather than text.
    pub kind: ErrorKind,
    /// Human-readable explanation, which may change editorially.
    pub message: String,
    /// Context records for every supplied input.
    pub inputs: Vec<InputRecord>,
    /// Diagnostics available before the operation failed.
    pub diagnostics: Vec<InputDiagnostic>,
}

/// Load a profile from an already-parsed YAML value, for hosts that hold their
/// profile in memory.
///
/// The value must already be resolved: this reads no file, so `extends` cannot
/// be followed and is rejected. Use [`load_profile`](crate::profile::load_profile)
/// for a profile on disk.
///
/// # Errors
///
/// [`ProfileError`] when the value carries `extends`, declares `annotations`
/// (a notes file needs a profile file to resolve against), or fails profile
/// validation; `where_` names it in the message.
///
/// # Examples
///
/// ```
/// use latt_core::service::prepare_profile;
///
/// let raw = latt_core::serde_norway::from_str(r#"
/// name: demo
/// profile_version: "1.0.0"
/// node_kinds:
///   req:
///     id_pattern: "^REQ-\\d+$"
/// edge_kinds: {}
/// "#)?;
/// let profile = prepare_profile(raw, "demo")?;
/// assert!(profile.node_kinds().contains_key("req"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn prepare_profile(raw: serde_norway::Value, where_: &str) -> Result<Profile, ProfileError> {
    if raw
        .as_mapping()
        .is_some_and(|map| map.contains_key(serde_norway::Value::String("extends".into())))
    {
        return Err(ProfileError::Invalid(format!(
            "{where_}: resolve 'extends' before service profile preparation"
        )));
    }
    profile::load_profile_value(raw, where_)
}

/// Load a profile from YAML text, as [`prepare_profile`] does from its value.
///
/// # Errors
///
/// [`ProfileError::Yaml`], with `where_` as its path, when the text is not
/// YAML; otherwise as [`prepare_profile`].
///
/// # Examples
///
/// ```
/// use latt_core::service::{ProfileError, prepare_profile_str};
///
/// let profile = prepare_profile_str(
///     "name: demo\nprofile_version: \"1.0.0\"\nnode_kinds: {}\nedge_kinds: {}\n",
///     "demo",
/// )?;
/// assert!(profile.node_kinds().is_empty());
/// assert!(matches!(
///     prepare_profile_str("name: [unclosed", "demo"),
///     Err(ProfileError::Yaml { .. })
/// ));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn prepare_profile_str(text: &str, where_: &str) -> Result<Profile, ProfileError> {
    let raw = serde_norway::from_str(text).map_err(|source| ProfileError::Yaml {
        path: where_.into(),
        source,
    })?;
    prepare_profile(raw, where_)
}

fn input_state(views: &[(InputSide, View<'_>)]) -> (Vec<InputRecord>, Vec<InputDiagnostic>) {
    let inputs = views
        .iter()
        .map(|(side, view)| InputRecord {
            side: *side,
            context: view.context.clone(),
        })
        .collect();
    let diagnostics = views
        .iter()
        .flat_map(|(side, view)| {
            validate::resolve_adapter_issues(view.graph, view.profile)
                .into_iter()
                .map(|issue| InputDiagnostic { side: *side, issue })
        })
        .collect();
    (inputs, diagnostics)
}

fn check_context(inputs: &[InputRecord]) -> Result<(), (ErrorKind, String)> {
    if inputs.iter().any(|input| {
        input.context.input_status == InputStatus::CompleteForDeclaredScope
            && input.context.scope == Scope::Unknown
    }) {
        return Err((
            ErrorKind::InvalidRequest,
            "complete input status requires a declared scope".into(),
        ));
    }
    Ok(())
}

fn request_error(error: query::QueryError) -> (ErrorKind, String) {
    (ErrorKind::InvalidRequest, error.into_message())
}
fn path_error(error: query::PathError) -> (ErrorKind, String) {
    match error {
        query::PathError::Request(error) => request_error(error),
        query::PathError::Invariant(message) => (ErrorKind::InternalInvariant, message.into()),
    }
}

/// Run one request over the supplied view only; results do not establish host
/// authority.
///
/// # Errors
///
/// [`ServiceError`], carrying the input records and diagnostics gathered before
/// the failure, with kind:
/// - [`ErrorKind::InvalidRequest`] for a malformed request (an unknown node, a
///   bad filter) or an input context claiming completeness for an undeclared scope;
/// - [`ErrorKind::InvalidConfiguration`] when the profile cannot configure the
///   operation, such as a `Summary` whose `SUMMARY` config is unusable;
/// - [`ErrorKind::InternalInvariant`] when the core breaks its own invariant.
///
/// # Examples
///
/// ```
/// use latt_core::document::{ingest_document, parse_document};
/// use latt_core::service::{
///     Direction, InputContext, Report, Request, View, execute, prepare_profile_str,
/// };
///
/// let profile = prepare_profile_str(r#"
/// name: demo
/// profile_version: "1.0.0"
/// node_kinds:
///   req: {id_pattern: "^REQ-\\d+$"}
///   test: {id_pattern: "^T-\\d+$"}
/// edge_kinds:
///   verifies: {allowed: [[test, req]]}
/// "#, "demo")?;
/// let graph = ingest_document(parse_document(r#"{
///   "interface_version": "1.2",
///   "nodes": [
///     {"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 1}},
///     {"id": "T-1", "kind": "test", "attrs": {}, "provenance": {"file": "t.rs", "line": 9}}],
///   "edges": [{"src": "T-1", "tgt": "REQ-1", "kind": "verifies",
///              "provenance": {"file": "t.rs", "line": 9}}],
///   "pathways": [], "findings": []
/// }"#)?)?;
/// let context = InputContext::default().with_identity("snapshot-1");
/// let view = View { graph: &graph, profile: &profile, context: &context };
///
/// let response = execute(view, Request::Reach {
///     origin: "REQ-1",
///     direction: Direction::Reverse,
///     edge_kinds: &["verifies"],
///     filters: &[],
///     check_resolved: false,
/// })?;
/// let Report::Reach(reach) = response.payload else { unreachable!() };
/// assert_eq!(reach.nodes[0].id, "T-1");
/// assert_eq!(response.inputs[0].context.identity.as_deref(), Some("snapshot-1"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn execute(view: View<'_>, request: Request<'_>) -> Result<Response<Report>, ServiceError> {
    let (inputs, diagnostics) = input_state(&[(InputSide::Current, view)]);
    let payload = (|| {
        check_context(&inputs)?;
        let (graph, profile) = (view.graph, view.profile);
        let today = view.context.today.unwrap_or_else(CivilDate::today_utc);
        match request {
            Request::Reach {
                origin,
                direction,
                edge_kinds,
                filters,
                check_resolved,
            } => query::reach(
                graph,
                profile,
                origin,
                edge_kinds,
                direction,
                filters,
                check_resolved,
            )
            .map(Report::Reach)
            .map_err(request_error),
            Request::Path {
                src,
                tgt,
                edge_kinds,
            } => query::path_typed(graph, profile, src, tgt, edge_kinds)
                .map(Report::Path)
                .map_err(path_error),
            Request::Orphans { kind, filters } => query::orphans(graph, profile, kind, filters)
                .map(Report::Orphans)
                .map_err(request_error),
            Request::Counts { filters } => {
                Ok(Report::Counts(query::counts(graph, profile, filters)))
            }
            Request::Coverage => Ok(Report::Coverage(query::coverage(graph, profile))),
            Request::At {
                target,
                path,
                host_path_exists,
                filters,
            } => query::at_in_view(
                graph,
                profile,
                validate::validate_on(graph, profile, false, today),
                target,
                path,
                env!("CARGO_PKG_VERSION"),
                filters,
                host_path_exists,
            )
            .map(Report::At)
            .map_err(request_error),
            Request::Validate { strict } => Ok(Report::Validate(ValidateReport {
                findings: validate::validate_on(graph, profile, strict, today),
                scope: crate::scope::KindTally::of(graph, profile).scope(),
            })),
            Request::Summary => summary::build_summary_on(profile, graph, today)
                .map(Report::Summary)
                .map_err(|error| (ErrorKind::InvalidConfiguration, error.into_message())),
            Request::Trace { strict } => Ok(Report::Trace(trace::build_trace_report(
                profile,
                graph,
                validate::validate_on(graph, profile, strict, today),
                env!("CARGO_PKG_VERSION"),
            ))),
        }
    })();
    match payload {
        Ok(payload) => Ok(Response {
            payload,
            inputs,
            diagnostics,
            execution: ExecutionStatus::Complete,
        }),
        Err((kind, message)) => Err(ServiceError {
            kind,
            message,
            inputs,
            diagnostics,
        }),
    }
}

/// Diff two supplied views; labels are opaque and never materialized as Git
/// revisions.
///
/// # Errors
///
/// [`ServiceError`] with [`ErrorKind::InvalidRequest`] when either input context
/// claims completeness for an undeclared scope; the diff itself cannot fail.
///
/// # Examples
///
/// ```
/// use latt_core::document::{ingest_document, parse_document};
/// use latt_core::service::{InputContext, View, compare, prepare_profile_str};
///
/// let profile = prepare_profile_str(
///     "name: demo\nprofile_version: \"1.0.0\"\nnode_kinds: {req: {id_pattern: \"^REQ-\\\\d+$\"}}\nedge_kinds: {}\n",
///     "demo",
/// )?;
/// let graph = |ids: &[&str]| {
///     let nodes: Vec<String> = ids.iter().map(|id| format!(
///         r#"{{"id": "{id}", "kind": "req", "attrs": {{}}, "provenance": {{"file": "r.md", "line": 1}}}}"#
///     )).collect();
///     ingest_document(parse_document(&format!(
///         r#"{{"interface_version": "1.2", "nodes": [{}], "edges": [], "pathways": [], "findings": []}}"#,
///         nodes.join(",")
///     ))?)
/// };
/// let (before, after) = (graph(&["REQ-1"])?, graph(&["REQ-1", "REQ-2"])?);
/// let context = InputContext::default();
/// let view = |graph| View { graph, profile: &profile, context: &context };
///
/// let diff = compare("v1", view(&before), "v2", view(&after))?.payload;
/// assert_eq!(diff.nodes_added[0].id, "REQ-2");
/// assert!(diff.nodes_removed.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn compare(
    before_label: &str,
    before: View<'_>,
    after_label: &str,
    after: View<'_>,
) -> Result<Response<DiffReport>, ServiceError> {
    let (inputs, diagnostics) =
        input_state(&[(InputSide::Before, before), (InputSide::After, after)]);
    if let Err((kind, message)) = check_context(&inputs) {
        return Err(ServiceError {
            kind,
            message,
            inputs,
            diagnostics,
        });
    }
    Ok(Response {
        payload: query::diff(before_label, before.graph, after_label, after.graph),
        inputs,
        diagnostics,
        execution: ExecutionStatus::Complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_constructor_failure_retains_internal_origin() {
        let message = PathReport::found("x".into(), "y".into(), vec![], vec![]).unwrap_err();
        let (kind, actual) = path_error(query::PathError::Invariant(message));
        assert_eq!(kind, ErrorKind::InternalInvariant);
        assert_eq!(actual, message);
    }
}
