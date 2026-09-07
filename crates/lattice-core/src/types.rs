//! Provenance, severity and findings — the vocabulary every other module reports in.

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
/// the tool cannot stand behind, and nothing — not `--strict`, not a profile
/// override — promotes a finding out of it.
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
/// `code` is the stable machine-readable name (PARSE_ERROR, DANGLING_REF);
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
        }
    }

    /// The total order findings are reported in: location, then code, then identity.
    ///
    /// Severity is deliberately absent — findings read in source order, not worst
    /// first, and a severity term would reorder output when a profile overrides one.
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

/// Status rollup payload: one row per group, counts keyed by status.
///
/// `status_keys` is carried beside the counts rather than derived from them: the
/// columns come from the profile's declared enum as well as from what the
/// register holds, so a status no node currently carries still gets a zero
/// column instead of vanishing.
#[derive(Debug)]
pub struct SummaryReport {
    pub group_key: String,
    pub status_keys: Vec<String>,
    pub groups: Vec<(String, BTreeMap<String, i64>)>,
}

impl SummaryReport {
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

/// A node named in a query answer: its ID and declared kind, nothing more.
#[derive(Debug, PartialEq, Eq)]
pub struct NodeRef {
    pub id: String,
    pub kind: String,
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
    /// `"reaches"` or `"reached-by"` — which way the walk followed edges.
    pub direction: String,
    /// The restriction the caller asked for; empty means every edge kind.
    pub edge_kinds: Vec<String>,
    pub nodes: Vec<NodeRef>,
}

/// Path answer. When `found` is false the vectors are empty — "not connected"
/// is an answer, and the payload says so rather than being absent.
#[derive(Debug)]
pub struct PathReport {
    pub src: String,
    pub tgt: String,
    pub found: bool,
    /// The nodes along the path, `src` first, `tgt` last.
    pub nodes: Vec<String>,
    /// The edge kinds between them: one fewer than `nodes`.
    pub edges: Vec<String>,
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
    pub nodes_changed: Vec<NodeRef>,
    pub edges_added: Vec<EdgeRef>,
    pub edges_removed: Vec<EdgeRef>,
    /// Axes added, removed, or with a different order or current position.
    pub axes_changed: Vec<String>,
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
            && self.axes_changed.is_empty()
    }
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
    /// Outgoing targets by edge kind, each list already in its specified order.
    pub edges: BTreeMap<String, Vec<String>>,
    pub findings: Vec<Issue>,
}

impl TraceEntry {
    /// Targets across every edge kind. Repeated targets count once each, because
    /// the trace preserves rather than deduplicates them.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.values().map(Vec::len).sum()
    }
}

/// The answer to `query at`: what the register holds from one source path.
#[derive(Debug)]
pub struct AtReport {
    /// The path as asked, before any target-joining.
    pub path: String,
    /// Entries whose node is declared at the path, edges and findings attached.
    pub entries: Vec<TraceEntry>,
    /// Findings at the path that sit inside none of those entries — attached
    /// to a node declared elsewhere, or attachable to no node at all.
    pub findings: Vec<Issue>,
}

/// Full trace-report payload: header, node entries, and unattachable findings.
#[derive(Debug)]
pub struct TraceReport {
    pub header: BTreeMap<String, String>,
    pub entries: Vec<TraceEntry>,
    /// Findings with no `node_id`, or one naming no explicitly added node. They
    /// belong to no entry, and dropping them would hide a finding that still
    /// moves the exit code.
    pub unattachable_findings: Vec<Issue>,
}
