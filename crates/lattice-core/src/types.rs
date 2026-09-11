//! Provenance, severity and findings: the vocabulary every other module reports in.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

/// Where a node, edge or issue came from in the source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub file: String,
    pub line: i64,
}

impl Provenance {
    pub fn new(file: impl Into<String>, line: i64) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// Issue severity. Only `Error` sets a non-zero exit code; `Hint` is advice
/// the tool cannot stand behind. Nothing (not `--strict`, not a profile
/// override) promotes a finding out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    /// The wire spelling, which is also what `plain` and `json` output carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
            Severity::Hint => "hint",
        }
    }

    /// The wire spelling in upper case, which is how `plain` and `rich` label a
    /// finding. A constant rather than `to_uppercase`, which would allocate once
    /// per finding per render.
    pub fn as_upper(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
            Severity::Info => "INFO",
            Severity::Hint => "HINT",
        }
    }

    /// Parse the wire spelling, or `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "error" => Some(Severity::Error),
            "warning" => Some(Severity::Warning),
            "info" => Some(Severity::Info),
            "hint" => Some(Severity::Hint),
            _ => None,
        }
    }
}

/// One finding, from either an adapter's parse or a validation check.
///
/// `code` is the stable machine-readable name (PARSE_ERROR, VACANCY);
/// `message` is prose for a human and is not a contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub provenance: Provenance,
    pub node_id: Option<String>,
    /// A machine-readable qualifier of what the finding established, set only
    /// by validators that define one (coverage's `unverified`/`unknown`).
    /// `None` serializes as an absent key, never as null, so findings that
    /// never had a state stay byte-identical to before the field existed.
    pub state: Option<String>,
    /// Set by a profile SUPPRESS entry after severity resolution. A suppressed
    /// finding keeps every other field, stays in JSON output, and is left out
    /// of the exit-code decision and the human-facing formats. `false`
    /// serializes as an absent key, for the same reason `state` does.
    pub suppressed: bool,
}

impl Issue {
    pub fn new(
        severity: Severity,
        code: impl Into<String>,
        message: impl Into<String>,
        provenance: Provenance,
        node_id: Option<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            provenance,
            node_id,
            state: None,
            suppressed: false,
        }
    }

    /// True when this finding moves the exit code: error severity and not
    /// suppressed. The one place the two conditions are combined, so no
    /// command counts a suppressed error by reading `severity` alone.
    #[must_use]
    pub fn gates(&self) -> bool {
        self.severity == Severity::Error && !self.suppressed
    }

    /// The total order findings are reported in: location, then code, then identity.
    ///
    /// Severity is deliberately absent. Findings read in source order, not worst
    /// first; a severity term would reorder output when a profile overrides one.
    #[must_use]
    pub fn sort_key(&self) -> (&str, i64, &str, &str, &str) {
        (
            &self.provenance.file,
            self.provenance.line,
            &self.code,
            self.node_id.as_deref().unwrap_or(""),
            &self.message,
        )
    }
}

/// What `summary` has to show, which depends on whether the profile said what
/// a summary means. A `SUMMARY` config selects the status rollup; without one
/// the command falls back to structural statistics rather than refusing.
#[derive(Debug)]
pub enum SummaryReport {
    Configured(StatusRollup),
    Structural(StructuralSummary),
}

/// Zero-config summary payload: counts by kind and finding tallies.
///
/// Every kind the profile declares gets an entry even at zero, so an empty
/// register reads as "declared but unused" rather than as "unknown".
#[derive(Debug)]
pub struct StructuralSummary {
    pub node_counts: BTreeMap<String, i64>,
    pub edge_counts: BTreeMap<String, i64>,
    pub finding_counts: Vec<FindingTally>,
}

/// How many findings one code produced at one severity.
#[derive(Debug, PartialEq, Eq)]
pub struct FindingTally {
    pub code: String,
    pub severity: Severity,
    pub count: i64,
}

/// Status rollup payload: one row per group, counts keyed by status.
///
/// `status_keys` is carried beside the counts rather than derived from them: the
/// columns come from the profile's declared enum as well as from what the
/// register holds, so a status no node currently carries still gets a zero
/// column instead of vanishing.
#[derive(Debug)]
pub struct StatusRollup {
    pub group_key: String,
    pub status_keys: Vec<String>,
    pub groups: Vec<(String, BTreeMap<String, i64>)>,
}

impl StatusRollup {
    /// Column totals, including the `total` column itself.
    #[must_use]
    pub fn totals(&self) -> BTreeMap<String, i64> {
        let mut totals = BTreeMap::new();
        for key in self.status_keys.iter().chain([&"total".to_string()]) {
            let sum = self
                .groups
                .iter()
                .map(|(_, counts)| counts.get(key).copied().unwrap_or(0))
                .sum();
            totals.insert(key.clone(), sum);
        }
        totals
    }
}

/// A node named in a query answer: its ID and declared kind.
#[derive(Debug, PartialEq, Eq)]
pub struct NodeRef {
    pub id: String,
    pub kind: String,
    /// `Some(true)` when every path the walk found to this node crosses an
    /// undeclared endpoint; `None` when the walk was not asked to tell.
    pub tainted: Option<bool>,
}

/// An edge named in a query answer, as the (src, kind, tgt) triple it is.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdgeRef {
    pub src: String,
    pub tgt: String,
    pub kind: String,
}

/// Reachability answer: the transitive closure from `origin`, origin excluded.
#[derive(Debug)]
pub struct ReachReport {
    pub origin: String,
    /// `"reaches"` or `"reached-by"`: which way the walk followed edges.
    pub direction: String,
    /// The restriction the caller asked for; empty means every edge kind.
    pub edge_kinds: Vec<String>,
    pub nodes: Vec<NodeRef>,
}

/// Path answer: either a validated path or a not-connected result.
#[derive(Debug)]
pub struct PathReport {
    src: String,
    tgt: String,
    outcome: PathOutcome,
}

#[derive(Debug)]
enum PathOutcome {
    Found {
        nodes: Vec<String>,
        edges: Vec<String>,
    },
    NotFound,
}

impl PathReport {
    /// Build a found result when nodes and intervening edges form a valid path.
    pub fn found(
        src: String,
        tgt: String,
        nodes: Vec<String>,
        edges: Vec<String>,
    ) -> Result<Self, &'static str> {
        if nodes.is_empty() {
            return Err("a found path must contain at least one node");
        }
        if edges.len() + 1 != nodes.len() {
            return Err("a found path must have one fewer edge than nodes");
        }
        if nodes.first().map(String::as_str) != Some(&src) {
            return Err("first node must equal src");
        }
        if nodes.last().map(String::as_str) != Some(&tgt) {
            return Err("last node must equal tgt");
        }
        Ok(Self {
            src,
            tgt,
            outcome: PathOutcome::Found { nodes, edges },
        })
    }

    /// Build a not-connected result.
    #[must_use]
    pub fn not_found(src: String, tgt: String) -> Self {
        Self {
            src,
            tgt,
            outcome: PathOutcome::NotFound,
        }
    }

    #[must_use]
    pub fn src(&self) -> &str {
        &self.src
    }

    #[must_use]
    pub fn tgt(&self) -> &str {
        &self.tgt
    }

    #[must_use]
    pub fn found_path(&self) -> Option<(&[String], &[String])> {
        match &self.outcome {
            PathOutcome::Found { nodes, edges } => Some((nodes, edges)),
            PathOutcome::NotFound => None,
        }
    }
}

/// One orphan: a declared node no edge names.
#[derive(Debug)]
pub struct OrphanEntry {
    pub id: String,
    pub kind: String,
    pub provenance: Provenance,
}

/// Orphans answer, with the kind restriction it was computed under.
#[derive(Debug)]
pub struct OrphansReport {
    pub kind_filter: Option<String>,
    pub orphans: Vec<OrphanEntry>,
}

/// Counts answer: per-kind tallies, declared kinds present even at zero.
#[derive(Debug)]
pub struct CountsReport {
    pub nodes: BTreeMap<String, i64>,
    pub edges: BTreeMap<String, i64>,
}

/// Coverage answer: per-kind edge-direction statistics, ordered by kind name.
/// Declared kinds appear even at zero, on the same terms as `CountsReport`.
#[derive(Debug)]
pub struct CoverageReport {
    pub kinds: Vec<KindCoverage>,
}

/// How many nodes of one kind exist and how many any edge enters or leaves.
///
/// Percentages ride along rather than being left to the reader: they are the
/// figure the report exists to give, and every consumer would otherwise round
/// them differently.
#[derive(Debug, PartialEq)]
pub struct KindCoverage {
    pub kind: String,
    pub total: i64,
    pub incoming: i64,
    pub outgoing: i64,
    pub incoming_pct: f64,
    pub outgoing_pct: f64,
}

/// Two-revision diff answer: what the register gained, lost, and changed
/// between two live adapter runs. Provenance is excluded from every identity
/// here, so a declaration that merely moved lines does not appear.
#[derive(Debug)]
pub struct DiffReport {
    pub rev_a: String,
    pub rev_b: String,
    pub nodes_added: Vec<NodeRef>,
    pub nodes_removed: Vec<NodeRef>,
    /// Present at both revisions with a different kind or attrs; the kind
    /// shown is revision B's.
    pub nodes_changed: Vec<NodeChangedRef>,
    pub edges_added: Vec<EdgeRef>,
    pub edges_removed: Vec<EdgeRef>,
    /// Present at both revisions under the same (src, tgt, kind) with
    /// different attrs. Parallel edges pair up in document order.
    pub edges_changed: Vec<EdgeChangedRef>,
    /// Axes added, removed, or with a different order or current position.
    pub pathways_changed: Vec<String>,
}

impl DiffReport {
    /// True when the two revisions ingested identically.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes_added.is_empty()
            && self.nodes_removed.is_empty()
            && self.nodes_changed.is_empty()
            && self.edges_added.is_empty()
            && self.edges_removed.is_empty()
            && self.edges_changed.is_empty()
            && self.pathways_changed.is_empty()
    }
}

/// A node whose kind or attrs differ between the two diffed revisions: its ID,
/// revision B's kind, and both attr maps, so the reader sees what changed,
/// not just that something did.
#[derive(Debug, PartialEq, Eq)]
pub struct NodeChangedRef {
    pub id: String,
    pub kind: String,
    pub attrs_a: serde_json::Map<String, serde_json::Value>,
    pub attrs_b: serde_json::Map<String, serde_json::Value>,
}

/// An edge whose attrs differ between the two diffed revisions: the identity
/// tuple and both attr maps, on the same terms as [`NodeChangedRef`].
#[derive(Debug, PartialEq, Eq)]
pub struct EdgeChangedRef {
    pub src: String,
    pub tgt: String,
    pub kind: String,
    pub attrs_a: serde_json::Map<String, serde_json::Value>,
    pub attrs_b: serde_json::Map<String, serde_json::Value>,
}

/// One outgoing edge in a trace report, retaining attrs and provenance.
#[derive(Debug)]
pub struct TraceEdge {
    pub tgt: String,
    pub kind: String,
    pub attrs: serde_json::Map<String, serde_json::Value>,
    pub provenance: Provenance,
}

/// One node in a trace report, carrying its edges and attached findings.
#[derive(Debug)]
pub struct TraceEntry {
    pub id: String,
    pub kind: String,
    pub attrs: BTreeMap<String, Value>,
    pub provenance: Provenance,
    /// The attr to show in the trace row's summary column, resolved from the profile.
    pub summary_attr: Option<String>,
    /// Outgoing edges, already ordered by kind, target, and provenance.
    pub edges: Vec<TraceEdge>,
    pub findings: Vec<Issue>,
}

impl TraceEntry {
    /// Repeated edges count once each, because the trace preserves rather than
    /// deduplicates them.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// The answer to `query at`: what the register holds from one source path.
#[derive(Debug)]
pub struct AtReport {
    /// The path as asked, before any target-joining.
    pub path: String,
    /// Entries whose node is declared at the path, edges and findings attached.
    pub entries: Vec<TraceEntry>,
    /// Findings at the path that sit inside none of those entries: attached
    /// to a node declared elsewhere, or attachable to no node at all.
    pub findings: Vec<Issue>,
}

/// An ordering pathway carried through the trace payload.
#[derive(Debug)]
pub struct PathwayEntry {
    pub name: String,
    pub order: Vec<String>,
    pub current: String,
}

/// Full trace-report payload: header, node entries, findings, and pathways.
#[derive(Debug)]
pub struct TraceReport {
    pub header: BTreeMap<String, String>,
    pub entries: Vec<TraceEntry>,
    /// Findings with no `node_id`, or one naming no explicitly added node. They
    /// belong to no entry, and dropping them would hide a finding that still
    /// moves the exit code.
    pub unattachable_findings: Vec<Issue>,
    pub pathways: Vec<PathwayEntry>,
}

/// Source attribution kept separately from the original file and line.
#[derive(Clone, Debug)]
pub struct FuseProvenance {
    pub source: String,
    pub location: Provenance,
}

#[derive(Debug)]
pub struct FuseNode {
    pub id: String,
    pub kind: String,
    pub attrs: serde_json::Map<String, Value>,
    pub provenance: FuseProvenance,
}

#[derive(Debug)]
pub struct FuseEdge {
    pub src: String,
    pub tgt: String,
    pub kind: String,
    pub attrs: serde_json::Map<String, Value>,
    pub provenance: FuseProvenance,
    pub source_kind: String,
    pub target_kind: Option<String>,
    pub target_source: Option<String>,
}

#[derive(Debug)]
pub struct FuseFinding {
    pub issue: Issue,
    pub source: Option<String>,
    pub locations: Vec<FuseProvenance>,
}

/// Composed graph and findings, in manifest source order. A failed source leaves
/// the graph empty; findings from healthy sources still remain visible.
#[derive(Debug, Default)]
pub struct FuseReport {
    pub header: BTreeMap<String, String>,
    pub nodes: Vec<FuseNode>,
    pub edges: Vec<FuseEdge>,
    pub pathways: Vec<PathwayEntry>,
    pub findings: Vec<FuseFinding>,
    pub could_run: bool,
}

impl FuseReport {
    pub fn exit_code(&self) -> u8 {
        if !self.could_run {
            2
        } else if self.findings.iter().any(|f| f.issue.gates()) {
            1
        } else {
            0
        }
    }
}
