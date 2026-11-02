//! Checking a graph against its profile: ID syntax, kinds, attrs, edges, coverage.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

mod adapter_issues;
mod collect;
mod conditions;
mod coverage;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::Value;

use crate::graph::{LatticeGraph, Node};
use crate::profile::{PlanEntry, Profile, ValidationSpec};
use crate::template::{Subject, Template};
use crate::types::{CivilDate, Issue, Origin, Severity};

#[doc(hidden)]
pub use adapter_issues::resolve_adapter_issues;
pub(crate) use collect::{collect, sort_issues};
pub use conditions::eval_conditions;

#[derive(Clone, Copy)]
pub(crate) enum FindingCode {
    AttrEnum,
    AttrListItems,
    AttrRequired,
    AttrType,
    Constraint,
    PathwayUnresolved,
    ConfigError,
    Coverage,
    CoverageDeep,
    CoverageUnknown,
    Vacancy,
    EdgeConstraint,
    EmptyKind,
    EdgeCount,
    EdgeWhere,
    IdFormat,
    Unreferenced,
    Untraced,
    SuggestedEdge,
    SuggestionUnresolved,
    SuggestionUnranked,
    SuppressUnused,
    OverrideUnused,
    UnknownKind,
    NoteFileMissing,
    NoteInvalid,
    NoteUnused,
    NoteExpired,
}

impl FindingCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AttrEnum => "ATTR_ENUM",
            Self::AttrListItems => "ATTR_LIST_ITEMS",
            Self::AttrRequired => "ATTR_REQUIRED",
            Self::AttrType => "ATTR_TYPE",
            Self::Constraint => "CONSTRAINT",
            Self::PathwayUnresolved => "PATHWAY_UNRESOLVED",
            Self::ConfigError => "CONFIG_ERROR",
            Self::Coverage => "COVERAGE",
            Self::CoverageDeep => "COVERAGE_DEEP",
            Self::CoverageUnknown => "COVERAGE_UNKNOWN",
            Self::Vacancy => "VACANCY",
            Self::EdgeConstraint => "EDGE_CONSTRAINT",
            Self::EmptyKind => "EMPTY_KIND",
            Self::EdgeCount => "EDGE_COUNT",
            Self::EdgeWhere => "EDGE_WHERE",
            Self::IdFormat => "ID_FORMAT",
            Self::Unreferenced => "UNREFERENCED",
            Self::Untraced => "UNTRACED",
            Self::SuggestedEdge => "SUGGESTED_EDGE",
            Self::SuggestionUnresolved => "SUGGESTION_UNRESOLVED",
            Self::SuggestionUnranked => "SUGGESTION_UNRANKED",
            Self::SuppressUnused => "SUPPRESS_UNUSED",
            Self::OverrideUnused => "OVERRIDE_UNUSED",
            Self::UnknownKind => "UNKNOWN_KIND",
            Self::NoteFileMissing => "NOTE_FILE_MISSING",
            Self::NoteInvalid => "NOTE_INVALID",
            Self::NoteUnused => "NOTE_UNUSED",
            Self::NoteExpired => "NOTE_EXPIRED",
        }
    }

    pub(crate) fn from_str(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == code)
    }

    /// True when every finding of this code carries a `node_id`, so a declared
    /// message may name the node. The configuration-level codes report on the
    /// profile, a notes file or a suggestion set, and carry none.
    pub(crate) const fn carries_node(self) -> bool {
        !matches!(
            self,
            Self::PathwayUnresolved
                | Self::ConfigError
                | Self::EmptyKind
                | Self::CoverageUnknown
                | Self::SuggestionUnresolved
                | Self::SuggestionUnranked
                | Self::SuppressUnused
                | Self::OverrideUnused
                | Self::NoteFileMissing
                | Self::NoteInvalid
                | Self::NoteUnused
                | Self::NoteExpired
        )
    }

    /// Every code, in declaration order.
    pub(crate) const ALL: [Self; 28] = [
        Self::AttrEnum,
        Self::AttrListItems,
        Self::AttrRequired,
        Self::AttrType,
        Self::Constraint,
        Self::PathwayUnresolved,
        Self::ConfigError,
        Self::Coverage,
        Self::CoverageDeep,
        Self::CoverageUnknown,
        Self::Vacancy,
        Self::EdgeConstraint,
        Self::EmptyKind,
        Self::EdgeCount,
        Self::EdgeWhere,
        Self::IdFormat,
        Self::Unreferenced,
        Self::Untraced,
        Self::SuggestedEdge,
        Self::SuggestionUnresolved,
        Self::SuggestionUnranked,
        Self::SuppressUnused,
        Self::OverrideUnused,
        Self::UnknownKind,
        Self::NoteFileMissing,
        Self::NoteInvalid,
        Self::NoteUnused,
        Self::NoteExpired,
    ];
}

/// Every code the core and `fuse` emit, with its owner and default severity.
///
/// The core's rows of `contracts/codes.tsv`; adapter codes are the adapters'.
#[must_use]
#[doc(hidden)]
pub fn core_codes() -> Vec<(&'static str, &'static str, Severity, bool)> {
    let core = FindingCode::ALL
        .into_iter()
        .map(|c| (c.as_str(), "core", default_severity(c), c.carries_node()));
    let fuse = crate::fuse::CODES
        .iter()
        .map(|&c| (c, "fuse", Severity::Error, crate::fuse::carries_node(c)));
    core.chain(fuse).collect()
}

/// Every code a shipped component emits: the core's, `fuse`'s and the adapters'.
///
/// A severity override for a code outside this set can only act on an issue some
/// other adapter emits (validation spec, `OVERRIDE_UNUSED`).
#[must_use]
#[doc(hidden)]
pub fn shipped_codes() -> BTreeSet<&'static str> {
    core_codes()
        .into_iter()
        .map(|(code, ..)| code)
        .chain(latt_adapter::codes::ALL.iter().copied())
        .collect()
}

/// Codes a release retired, each with its replacement code or codes and the release.
///
/// A profile naming one is refused at load (profile-schema spec);
/// `contracts/retired-codes.tsv` pins the list.
#[doc(hidden)]
pub const RETIRED_CODES: &[(&str, &str, &str)] = &[
    ("AXIS_INVALID", "PATHWAY_INVALID", "0.7.0"),
    ("AXIS_UNRESOLVED", "PATHWAY_UNRESOLVED", "0.7.0"),
    ("DANGLING_REF", "VACANCY", "0.6.0"),
    ("ORPHAN_NODE", "UNREFERENCED and UNTRACED", "0.7.0"),
];

/// True when every finding of `code` carries a `node_id`: a core or fuse code
/// that says so. An adapter's or unknown code promises nothing.
pub(crate) fn code_carries_node(code: &str) -> bool {
    FindingCode::from_str(code).map_or_else(
        || crate::fuse::CODES.contains(&code) && crate::fuse::carries_node(code),
        FindingCode::carries_node,
    )
}

/// The message a finding carries: the entry's declared template rendered over
/// the generated text, or the generated text when the entry declares none.
fn render_message(
    template: Option<&Template>,
    generated: String,
    code: FindingCode,
    severity: Severity,
    node: Option<&Node>,
) -> String {
    match template {
        None => generated,
        Some(template) => template.render(&Subject {
            message: &generated,
            code: code.as_str(),
            severity,
            node,
        }),
    }
}

/// Render the declared `message` of every severity-only entry onto the
/// findings carrying its code, adapter issues included.
///
/// A finding whose code has no such entry keeps its text. Entries that declare
/// their own check render at the finding instead, where the entry is known.
/// Runs after `--strict`, so `{severity}` is the severity reported.
///
/// # Examples
///
/// ```
/// use latt_core::validate::{apply_messages, validate};
/// # use latt_core::service::prepare_profile;
/// # let profile = prepare_profile(serde_norway::from_str(r#"
/// # name: demo
/// # profile_version: "1.0.0"
/// # node_kinds:
/// #   req:
/// #     id_pattern: "^REQ-\\d+$"
/// # edge_kinds: {derives: {allowed: [[req, req]]}}
/// # validations:
/// #   - UNTRACED:
/// #       message: "{id} leads nowhere"
/// # "#)?, "demo")?;
/// # use latt_core::graph::LatticeGraph;
/// # use latt_core::types::Provenance;
/// # let mut graph = LatticeGraph::new();
/// # graph.add_node("REQ-1", "req", serde_json::Map::new(), Provenance::new("reqs.md", 3))?;
/// let mut issues = validate(&graph, &profile, false);
/// // `validate` renders messages itself; a host holding adapter issues apart
/// // from the graph renders them with this call. Rendering twice is harmless.
/// apply_messages(issues.iter_mut(), &profile, &graph);
/// let untraced = issues.iter().find(|issue| issue.code == "UNTRACED").ok_or("no UNTRACED")?;
/// assert_eq!(untraced.message, "REQ-1 leads nowhere");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn apply_messages<'a>(
    issues: impl IntoIterator<Item = &'a mut Issue>,
    profile: &Profile,
    graph: &LatticeGraph,
) {
    let mut templates: BTreeMap<&str, &Template> = BTreeMap::new();
    for entry in profile.validation_plan().entries() {
        if let (ValidationSpec::SeverityOnly, Some(template)) = (&entry.spec, &entry.message) {
            templates.insert(entry.code.as_str(), template);
        }
    }
    if templates.is_empty() {
        return;
    }
    for issue in issues {
        let Some(template) = templates.get(issue.code.as_str()) else {
            continue;
        };
        let node = issue.node_id.as_deref().and_then(|id| graph.node(id));
        issue.message = template.render(&Subject {
            message: &issue.message,
            code: &issue.code,
            severity: issue.severity,
            node,
        });
    }
}

/// The replacement for a retired code, when `code` is one.
pub(crate) fn retired_replacement(code: &str) -> Option<(&'static str, &'static str)> {
    RETIRED_CODES
        .iter()
        .find(|(retired, ..)| *retired == code)
        .map(|&(_, replacement, release)| (replacement, release))
}

/// The `VACANCY` message for an edge whose target is not declared. Fuse
/// matches a source's finding against it to withdraw one it resolved, so the
/// two must be built by this one function.
pub(crate) fn vacancy_target_message(src: &str, tgt: &str, kind: &str) -> String {
    format!("edge '{src}'->'{tgt}' (kind '{kind}'): target '{tgt}' does not exist")
}

/// The shipped severity of each core finding code, before profile overrides.
/// One branch does not consult overrides: a `VACANCY` on a `cross_source` edge
/// kind is always a hint, because the target may live in another source.
pub(crate) fn default_severity(code: FindingCode) -> Severity {
    match code {
        FindingCode::IdFormat
        | FindingCode::UnknownKind
        | FindingCode::EdgeConstraint
        | FindingCode::Vacancy
        | FindingCode::AttrRequired
        | FindingCode::AttrType
        | FindingCode::AttrEnum
        | FindingCode::AttrListItems
        | FindingCode::NoteInvalid
        | FindingCode::ConfigError => Severity::Error,
        // The overlay codes. Registered here so a profile trying to promote one
        // is reported as a CONFIG_ERROR by the pre-pass in `validate`, which is
        // the only place that sees the override whether or not a document ran.
        FindingCode::CoverageUnknown
        | FindingCode::SuggestedEdge
        | FindingCode::SuggestionUnresolved
        | FindingCode::SuggestionUnranked => Severity::Hint,
        // Info, not hint: a profile may promote it to warning so `--strict`
        // gates stale suppressions, and hint would foreclose that.
        // `EMPTY_KIND` is info for the same reason: "a declared kind read
        // nothing" is what a consumer gating on an empty register promotes.
        FindingCode::SuppressUnused
        | FindingCode::OverrideUnused
        | FindingCode::NoteUnused
        | FindingCode::EmptyKind => Severity::Info,
        // Explicitly warning, not via the fallthrough: hint is never promotable,
        // so a hint default here would foreclose gating on deep coverage for
        // every profile permanently (coverage-query spec).
        FindingCode::Constraint
        | FindingCode::EdgeCount
        | FindingCode::EdgeWhere
        | FindingCode::PathwayUnresolved
        | FindingCode::Coverage
        | FindingCode::CoverageDeep
        | FindingCode::Unreferenced
        | FindingCode::Untraced
        | FindingCode::NoteFileMissing
        | FindingCode::NoteExpired => Severity::Warning,
    }
}

pub(crate) fn resolve_severity(default: Severity, override_: Option<Severity>) -> Severity {
    match override_ {
        Some(o) if default == Severity::Hint && o != Severity::Hint => default,
        Some(o) => o,
        None => default,
    }
}

fn severity_for(code: FindingCode, profile: &Profile) -> Severity {
    resolve_severity(
        default_severity(code),
        profile.validation_overrides().get(code.as_str()).copied(),
    )
}

/// The severity one repeated-check entry's findings carry: its own `severity`
/// when it declares one, else the code's.
fn entry_severity(entry: &PlanEntry, code: FindingCode, profile: &Profile) -> Severity {
    entry
        .severity
        .unwrap_or_else(|| severity_for(code, profile))
}

pub(crate) fn issue(
    code: FindingCode,
    message: String,
    origin: impl Into<Origin>,
    profile: &Profile,
    node_id: Option<String>,
) -> Issue {
    Issue::new(
        severity_for(code, profile),
        code.as_str(),
        message,
        origin,
        node_id,
    )
}

/// Check a graph against its profile and return every issue found.
///
/// Covers ID syntax, unknown kinds, attribute schemas, edge endpoint pairs,
/// dangling references and the profile's configured validations. Collects rather
/// than stopping, so one malformed node never hides the rest; `strict` promotes
/// warnings to errors after collection, and the profile's SUPPRESS entries are
/// applied last, so a suppressed finding is reported but never gates.
///
/// Finding notes expire against today's UTC date; [`validate_on`] takes the date.
///
/// # Examples
///
/// ```
/// use latt_core::validate::validate;
/// # use latt_core::service::prepare_profile;
/// # let profile = prepare_profile(serde_norway::from_str(r#"
/// # name: demo
/// # profile_version: "1.0.0"
/// # node_kinds:
/// #   req:
/// #     id_pattern: "^REQ-\\d+$"
/// # edge_kinds: {derives: {allowed: [[req, req]]}}
/// # "#)?, "demo")?;
/// # use latt_core::graph::LatticeGraph;
/// # use latt_core::types::Provenance;
/// let mut graph = LatticeGraph::new();
/// graph.add_node("req-1", "req", serde_json::Map::new(), Provenance::new("reqs.md", 3))?;
///
/// let issues = validate(&graph, &profile, false);
/// let mut codes: Vec<&str> = issues.iter().map(|issue| issue.code.as_str()).collect();
/// codes.sort_unstable();
/// // `req-1` misses the kind's `id_pattern`; a lone node is also unreferenced and
/// // untraced; and the declared edge kind no edge used is reported at the profile.
/// assert_eq!(codes, ["EMPTY_KIND", "ID_FORMAT", "UNREFERENCED", "UNTRACED"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
#[doc(hidden)]
pub fn validate(graph: &LatticeGraph, profile: &Profile, strict: bool) -> Vec<Issue> {
    validate_on(graph, profile, strict, CivilDate::today_utc())
}

/// Validate as [`validate`] does, expiring finding notes against `today`.
///
/// Notes attach, and their `NOTE_*` findings join the rest, before
/// stale-suppression detection, `--strict` and suppression, so each of those
/// treats a note finding as any other.
///
/// # Examples
///
/// ```
/// use latt_core::types::{CivilDate, Severity};
/// use latt_core::validate::validate_on;
/// # use latt_core::service::prepare_profile;
/// # let profile = prepare_profile(serde_norway::from_str(r#"
/// # name: demo
/// # profile_version: "1.0.0"
/// # node_kinds:
/// #   req:
/// #     id_pattern: "^REQ-\\d+$"
/// # edge_kinds: {derives: {allowed: [[req, req]]}}
/// # "#)?, "demo")?;
/// # use latt_core::graph::LatticeGraph;
/// # use latt_core::types::Provenance;
/// # let mut graph = LatticeGraph::new();
/// # graph.add_node("REQ-1", "req", serde_json::Map::new(), Provenance::new("reqs.md", 3))?;
/// let today = CivilDate::new(2026, 10, 25).ok_or("not a date")?;
///
/// let issues = validate_on(&graph, &profile, true, today);
/// // `strict` promotes the lone node's two warnings to errors and leaves the
/// // info `EMPTY_KIND` for the unused edge kind alone.
/// assert_eq!(issues.len(), 3);
/// assert_eq!(issues.iter().filter(|issue| issue.severity == Severity::Error).count(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
#[doc(hidden)]
pub fn validate_on(
    graph: &LatticeGraph,
    profile: &Profile,
    strict: bool,
    today: CivilDate,
) -> Vec<Issue> {
    let mut issues = collect(graph, profile);
    crate::notes::attach(&mut issues, graph, profile, today);
    finish(&mut issues, profile, Origin::Profile, strict);
    apply_messages(issues.iter_mut(), profile, graph);
    sort_issues(&mut issues);
    issues
}

/// The tail of the pipeline over findings already at their resolved severities:
/// stale-suppression detection, `--strict` promotion, then suppression. Split
/// from `collect` so a caller that merges in findings of its own (fuse) can
/// run the tail once over the union.
pub(crate) fn finish(issues: &mut Vec<Issue>, profile: &Profile, origin: Origin, strict: bool) {
    // Overrides first, so a SUPPRESS naming OVERRIDE_UNUSED is judged against
    // the findings it is there to silence.
    let unused = unused_overrides(issues.iter(), profile, &origin);
    issues.extend(unused);
    let unused = unused_suppressions(issues.iter(), profile, &origin);
    issues.extend(unused);
    promote(issues.iter_mut(), strict);
    apply_suppressions(issues.iter_mut(), profile);
}

pub(crate) fn promote<'a>(issues: impl IntoIterator<Item = &'a mut Issue>, strict: bool) {
    if !strict {
        return;
    }
    for issue in issues {
        if issue.severity == Severity::Warning {
            issue.severity = Severity::Error;
        }
    }
}

/// Mark every finding a SUPPRESS entry selects.
///
/// Runs after promotion on purpose: suppression declares the finding
/// structurally expected, and `--strict` is a severity policy, not a way to take
/// that declaration back.
///
/// The service's envelope diagnostics arrive unsuppressed by the graph-service
/// contract; a host that shows or gates on them applies this first, as the CLI
/// does. The first entry for a code decides which nodes it selects.
///
/// # Examples
///
/// ```
/// use latt_core::validate::{apply_suppressions, validate};
/// # use latt_core::service::prepare_profile;
/// # let profile = prepare_profile(serde_norway::from_str(r#"
/// # name: demo
/// # profile_version: "1.0.0"
/// # node_kinds:
/// #   req:
/// #     id_pattern: "^REQ-\\d+$"
/// # edge_kinds: {derives: {allowed: [[req, req]]}}
/// # validations:
/// #   - SUPPRESS:
/// #       code: UNTRACED
/// #       node_ids: [REQ-1]
/// # "#)?, "demo")?;
/// # use latt_core::graph::LatticeGraph;
/// # use latt_core::types::Provenance;
/// # let mut graph = LatticeGraph::new();
/// # graph.add_node("REQ-1", "req", serde_json::Map::new(), Provenance::new("reqs.md", 3))?;
/// let mut issues = validate(&graph, &profile, false);
/// // `validate` applies the profile's entries itself; a host that holds findings
/// // the service left unsuppressed applies them with this call.
/// for issue in &mut issues {
///     issue.suppressed = false;
/// }
/// apply_suppressions(issues.iter_mut(), &profile);
/// let suppressed: Vec<&str> = issues
///     .iter()
///     .filter(|issue| issue.suppressed)
///     .map(|issue| issue.code.as_str())
///     .collect();
/// assert_eq!(suppressed, ["UNTRACED"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn apply_suppressions<'a>(issues: impl IntoIterator<Item = &'a mut Issue>, profile: &Profile) {
    let index = SuppressionIndex::new(profile);
    for issue in issues {
        if let Some(suppressed) = index.decide(&issue.code, issue.node_id.as_deref()) {
            issue.suppressed = suppressed;
        }
    }
}

/// A profile's `SUPPRESS` entries keyed by code, built once so a caller
/// deciding many findings does not rebuild it per finding.
pub(crate) struct SuppressionIndex<'p> {
    by_code: HashMap<&'p str, Option<HashSet<&'p str>>>,
}

impl<'p> SuppressionIndex<'p> {
    pub(crate) fn new(profile: &'p Profile) -> Self {
        // The first entry for a code decides, as a linear `find` would; its node
        // list becomes a set so a long list costs one lookup per finding.
        let mut by_code: HashMap<&str, Option<HashSet<&str>>> = HashMap::new();
        for entry in profile.suppressions() {
            by_code.entry(entry.code.as_str()).or_insert_with(|| {
                entry
                    .node_ids
                    .as_ref()
                    .map(|ids| ids.iter().map(String::as_str).collect())
            });
        }
        Self { by_code }
    }

    /// Decide the suppression a finding of `code` on `node_id` takes: `None`
    /// when no entry names the code and the finding keeps the state it has.
    pub(crate) fn decide(&self, code: &str, node_id: Option<&str>) -> Option<bool> {
        self.by_code.get(code).map(|node_ids| match node_ids {
            None => true,
            Some(ids) => node_id.is_some_and(|id| ids.contains(id)),
        })
    }
}

/// One `OVERRIDE_UNUSED` per severity override for a code no shipped component
/// emits that no finding carried this run.
///
/// Such an override can only act on an issue some other adapter emits, so the
/// message states what the core knows and no more: an external adapter may
/// emit the code on another run, and a dormant override for a shipped code is
/// the normal state of a clean register and is never reported.
pub(crate) fn unused_overrides<'a>(
    issues: impl IntoIterator<Item = &'a Issue>,
    profile: &Profile,
    origin: &Origin,
) -> Vec<Issue> {
    let carried: BTreeSet<&str> = issues.into_iter().map(|i| i.code.as_str()).collect();
    let shipped = shipped_codes();
    profile
        .validation_overrides()
        .keys()
        .filter(|code| !shipped.contains(code.as_str()) && !carried.contains(code.as_str()))
        .map(|code| {
            issue(
                FindingCode::OverrideUnused,
                format!(
                    "override for '{code}': no shipped component emits this code and no \
                     finding carried it this run"
                ),
                origin.clone(),
                profile,
                None,
            )
        })
        .collect()
}

/// One `SUPPRESS_UNUSED` per entry whose selector matched nothing this run.
///
/// Runs once, before suppression, so a `SUPPRESS_UNUSED` entry is judged
/// against the findings the other entries produced here: it is used when any
/// of them is stale, and is never asked about itself twice. The message says
/// only what the core knows. A code it never saw may be a typo or an adapter
/// code that stayed dormant, and it cannot tell which.
pub(crate) fn unused_suppressions<'a>(
    issues: impl IntoIterator<Item = &'a Issue>,
    profile: &Profile,
    origin: &Origin,
) -> Vec<Issue> {
    let issues: Vec<&Issue> = issues.into_iter().collect();
    let unused_code = FindingCode::SuppressUnused.as_str();
    let (self_entry, entries): (Vec<_>, Vec<_>) = profile
        .suppressions()
        .iter()
        .partition(|entry| entry.code == unused_code);

    let mut out: Vec<Issue> = Vec::new();
    for entry in entries {
        if let Some(message) = unused_message(entry, issues.iter().copied()) {
            out.push(issue(
                FindingCode::SuppressUnused,
                message,
                origin.clone(),
                profile,
                None,
            ));
        }
    }
    for entry in self_entry {
        if let Some(message) = unused_message(entry, issues.iter().copied().chain(&out)) {
            out.push(issue(
                FindingCode::SuppressUnused,
                message,
                origin.clone(),
                profile,
                None,
            ));
        }
    }
    out
}

fn unused_message<'a>(
    entry: &crate::profile::SuppressConfig,
    issues: impl IntoIterator<Item = &'a Issue>,
) -> Option<String> {
    let code = entry.code.as_str();
    let mut carried = false;
    let mut matched: BTreeSet<&str> = BTreeSet::new();
    for issue in issues.into_iter().filter(|i| i.code == code) {
        carried = true;
        if let Some(id) = issue.node_id.as_deref() {
            matched.insert(id);
        }
    }
    let unmatched: Vec<&str> = match &entry.node_ids {
        None if carried => return None,
        None => Vec::new(),
        Some(ids) => ids
            .iter()
            .map(String::as_str)
            .filter(|id| !matched.contains(id))
            .collect(),
    };
    if entry.node_ids.is_some() && unmatched.is_empty() {
        return None;
    }
    let builtin = if FindingCode::from_str(code).is_some() {
        "a built-in code"
    } else {
        "not a built-in code"
    };
    let selector = if unmatched.is_empty() {
        String::new()
    } else {
        format!(" for node_ids {unmatched:?}")
    };
    let seen = if carried {
        "findings carried it this run, but none for those node_ids"
    } else {
        "no finding carried it this run"
    };
    Some(format!(
        "SUPPRESS entry for '{code}' matched no finding{selector}: '{code}' is {builtin}; {seen}"
    ))
}

/// Demote findings whose node is not yet due on its bound ordering pathway.
///
/// Runs after collection and before `--strict`, so a demoted finding is already
/// `info` when promotion looks at it and survives. Starts from each issue's
/// effective severity rather than recomputing it, or an adapter code with no
/// shipped default would silently drop to the warning fallback.
pub(super) fn resolve_axes(issues: &mut Vec<Issue>, graph: &LatticeGraph, profile: &Profile) {
    if profile.pathway_bindings().is_empty() {
        return;
    }

    // One finding per binding, not per issue: the mismatch is a property of the
    // pairing, and repeating it per finding would bury the findings it reports
    // about. Keyed on the code too, so two codes bound to one missing pathway each
    // say so; a single finding could name only one of them.
    let mut unresolved: BTreeMap<(&str, &str), Issue> = BTreeMap::new();

    for issue in issues.iter_mut() {
        let Some((code, binding)) = profile.pathway_bindings().get_key_value(&issue.code) else {
            continue;
        };

        let Some(pathway) = graph.pathway(&binding.pathway) else {
            unresolved
                .entry((code, &binding.pathway))
                .or_insert_with(|| {
                    Issue::new(
                        severity_for(FindingCode::PathwayUnresolved, profile),
                        FindingCode::PathwayUnresolved.as_str(),
                        format!(
                            "profile binds '{code}' to pathway '{}', which the target does not declare",
                            binding.pathway
                        ),
                        Origin::Profile,
                        None,
                    )
                });
            continue;
        };

        let Some(node) = issue.node_id.as_deref().and_then(|id| graph.node(id)) else {
            continue;
        };
        let Some(Value::String(position)) = node.attrs.get(&binding.position_attr) else {
            continue;
        };

        if pathway.is_member(position) && !pathway.is_after(position) {
            continue;
        }

        // Demotion only lowers: a finding already at info or hint keeps it.
        if matches!(issue.severity, Severity::Error | Severity::Warning) {
            issue.severity = Severity::Info;
        }
    }

    issues.extend(unresolved.into_values());
}

#[cfg(test)]
mod tests;
