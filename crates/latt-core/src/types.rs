//! Provenance, severity and findings: the vocabulary every other module reports in.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use serde_json::Value;

/// Where a node, edge or issue came from in the source text.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Provenance {
    file: String,
    // Zero is the wire's "no exact line". Outside this crate `line()` is the only
    // reader; inside, `wire_line()` and `Display` write the zero on purpose.
    line: u32,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl Provenance {
    /// Construct source provenance from a path and a one-based line, zero when no
    /// exact line is available.
    #[must_use]
    pub fn new(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }

    /// Construct provenance for a file as a whole, with no line.
    ///
    /// ```
    /// use latt_core::service::Provenance;
    ///
    /// assert_eq!(Provenance::new_file("r.md").line(), None);
    /// ```
    #[must_use]
    pub fn new_file(file: impl Into<String>) -> Self {
        Self::new(file, 0)
    }

    /// Return the source path as reported by the adapter.
    #[must_use]
    pub fn file(&self) -> &str {
        &self.file
    }

    /// Return the one-based source line, or `None` when no exact line is available.
    ///
    /// ```
    /// use latt_core::service::Provenance;
    ///
    /// assert_eq!(Provenance::new("r.md", 0).line(), None);
    /// assert_eq!(Provenance::new("r.md", 3).line().map(|line| line.get()), Some(3));
    /// ```
    #[must_use]
    pub fn line(&self) -> Option<NonZeroU32> {
        NonZeroU32::new(self.line)
    }

    /// Return the line as the reports write it, zero when no exact line is available.
    pub(crate) fn wire_line(&self) -> u32 {
        self.line
    }

    /// Construct provenance from a line already read as an `Option`, so a caller
    /// holding one never spells the wire's zero.
    pub(crate) fn at_line(file: impl Into<String>, line: Option<NonZeroU32>) -> Self {
        Self::new(file, line.map_or(0, NonZeroU32::get))
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// Where a finding came from: a place in a file the adapter read or named, or an
/// input Lattice reads itself and reports on as a whole.
///
/// Only a [`Origin::File`] has a path. The others are written in the reports as
/// reserved names (`<profile>`, `<fuse-profile>`, `<source>`, `<suggestions>`) at
/// line 0, which is what [`Display`](fmt::Display) writes too; read them as
/// variants, never as paths.
///
/// ```
/// use latt_core::service::{Origin, Provenance};
///
/// let at = Origin::from(Provenance::new("r.md", 3));
/// assert_eq!(at.location().map(Provenance::file), Some("r.md"));
/// assert_eq!(Origin::Profile.location(), None);
/// assert_eq!(Origin::Profile.to_string(), "<profile>:0");
/// ```
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Origin {
    /// A file the adapter read or named, at a line when one is known.
    File(Provenance),
    /// The profile's configuration, not a line in it.
    Profile,
    /// A fuse run's fuse profile.
    FuseProfile,
    /// A fuse source that could not be run; the fuse finding names which.
    FuseSource,
    /// A suggestion document.
    Suggestions,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl Origin {
    /// Return the place in a file, or `None` for an input reported as a whole.
    #[must_use]
    pub fn location(&self) -> Option<&Provenance> {
        match self {
            Self::File(provenance) => Some(provenance),
            _ => None,
        }
    }

    /// Return the file the reports write: the path, or the reserved name.
    pub(crate) fn wire_file(&self) -> &str {
        match self {
            Self::File(provenance) => provenance.file(),
            Self::Profile => "<profile>",
            Self::FuseProfile => "<fuse-profile>",
            Self::FuseSource => "<source>",
            Self::Suggestions => "<suggestions>",
        }
    }

    /// Return the line the reports write, zero when there is none.
    pub(crate) fn wire_line(&self) -> u32 {
        self.location().map_or(0, Provenance::wire_line)
    }

    /// Read a finding's file and line from Lattice's own report JSON, where the
    /// reserved names stand for the inputs reported as a whole.
    pub(crate) fn from_wire(file: String, line: u32) -> Self {
        match file.as_str() {
            "<profile>" => Self::Profile,
            "<fuse-profile>" => Self::FuseProfile,
            "<source>" => Self::FuseSource,
            "<suggestions>" => Self::Suggestions,
            _ => Self::File(Provenance::new(file, line)),
        }
    }
}

impl From<Provenance> for Origin {
    fn from(provenance: Provenance) -> Self {
        Self::File(provenance)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.wire_file(), self.wire_line())
    }
}

/// A proleptic Gregorian calendar date: the unit a finding note expires in.
///
/// Only constructible as a real date, so comparison needs no validity check.
/// Ordering is the calendar's.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    year: i32,
    month: u8,
    day: u8,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl CivilDate {
    /// Return the date, or `None` when it is not on the calendar.
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let length = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => return None,
        };
        (1..=length)
            .contains(&day)
            .then_some(Self { year, month, day })
    }

    /// Parse exactly `YYYY-MM-DD`, returning `None` for anything else or for a
    /// date not on the calendar.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let shaped = bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && [0, 1, 2, 3, 5, 6, 8, 9]
                .iter()
                .all(|&i| bytes[i].is_ascii_digit());
        if !shaped {
            return None;
        }
        Self::new(
            text[..4].parse().ok()?,
            text[5..7].parse().ok()?,
            text[8..].parse().ok()?,
        )
    }

    /// Days from 1970-01-01 to the first and last dates a `CivilDate` holds.
    const MIN_DAYS: i64 = days_from_civil(i32::MIN as i64, 1, 1);
    const MAX_DAYS: i64 = days_from_civil(i32::MAX as i64, 12, 31);

    /// Return the date `days` days after 1970-01-01 (before it, when negative).
    ///
    /// Saturates: a count before the year `i32::MIN` gives its January 1st,
    /// one after the year `i32::MAX` its December 31st.
    #[must_use]
    pub fn from_unix_days(days: i64) -> Self {
        // Howard Hinnant's civil_from_days, over 400-year eras.
        let z = days.clamp(Self::MIN_DAYS, Self::MAX_DAYS) + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year: i32::try_from(year).unwrap_or(i32::MAX),
            month: u8::try_from(month).unwrap_or(1),
            day: u8::try_from(day).unwrap_or(1),
        }
    }

    /// Return today's date in UTC, from the system clock. A clock set before
    /// 1970 reads as 1970-01-01.
    #[must_use]
    pub fn today_utc() -> Self {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        Self::from_unix_days(i64::try_from(seconds / 86_400).unwrap_or(i64::MAX))
    }

    /// Return the year.
    #[must_use]
    pub fn year(self) -> i32 {
        self.year
    }

    /// Return the month, 1 to 12.
    #[must_use]
    pub fn month(self) -> u8 {
        self.month
    }

    /// Return the day of the month, from 1.
    #[must_use]
    pub fn day(self) -> u8 {
        self.day
    }
}

/// Howard Hinnant's `days_from_civil`: the inverse of `CivilDate::from_unix_days`
/// before it clamps, used to find its bounds.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl fmt::Display for CivilDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Issue severity. Only `Error` sets a non-zero exit code; `Hint` is advice
/// the tool cannot stand behind. Nothing (not `--strict`, not a profile
/// override) promotes a finding out of it.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// A gating defect.
    Error,
    /// A non-gating defect that strict validation may promote.
    Warning,
    /// Neutral diagnostic information.
    Info,
    /// Advice whose completeness the tool cannot establish.
    Hint,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl Severity {
    /// The wire spelling: what `json` output carries and how the summary line
    /// names a severity. Plain and rich findings label with [`as_upper`](Self::as_upper).
    #[must_use]
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
    #[must_use]
    pub fn as_upper(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warning => "WARNING",
            Severity::Info => "INFO",
            Severity::Hint => "HINT",
        }
    }

    /// Parse the wire spelling, or `None` for anything else.
    #[must_use]
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

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One finding, from either an adapter's parse or a validation check.
///
/// `code` is the stable machine-readable name (`PARSE_ERROR`, `VACANCY`);
/// `message` is prose for a human and is not a contract.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Issue {
    /// Resolved severity after profile overrides.
    pub severity: Severity,
    /// Stable machine-readable finding code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Where the finding came from: a place in a file, or an input reported as a
    /// whole.
    pub origin: Origin,
    /// Related node ID, when the finding belongs to a node.
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
    /// The finding-notes entry that matched this finding's code and node. It
    /// annotates and never changes severity, suppression or the exit code.
    /// `None` serializes as an absent key, as `state` does.
    pub note: Option<Note>,
}

/// A person's recorded judgement about a finding, from the profile's notes file.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Note {
    /// The note text.
    pub text: String,
    /// When the note stops holding, if the entry says.
    pub until: Option<Until>,
    /// Who recorded it, if the entry says.
    pub by: Option<String>,
    /// When it was recorded, if the entry says.
    pub at: Option<CivilDate>,
    /// True when `until` has passed. An expired note still attaches.
    pub expired: bool,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl Note {
    /// Construct an unexpired note with only its text.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            until: None,
            by: None,
            at: None,
            expired: false,
        }
    }
}

/// Where a note's validity ends.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Until {
    /// Expires after this date.
    Date(CivilDate),
    /// Expires once the register's current position on `pathway` reaches `position`.
    Pathway {
        /// A pathway the profile declares.
        pathway: String,
        /// A position in that pathway's order.
        position: String,
    },
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl Issue {
    /// Construct an unsuppressed finding with no coverage state.
    #[must_use]
    pub fn new(
        severity: Severity,
        code: impl Into<String>,
        message: impl Into<String>,
        origin: impl Into<Origin>,
        node_id: Option<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            origin: origin.into(),
            node_id,
            state: None,
            suppressed: false,
            note: None,
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
    ///
    /// The key is opaque so a tiebreaker can be added without a breaking
    /// change; compare keys, do not take them apart.
    #[must_use]
    pub fn sort_key(&self) -> impl Ord + '_ {
        (
            self.origin.wire_file(),
            self.origin.wire_line(),
            &self.code,
            self.node_id.as_deref().unwrap_or(""),
            &self.message,
        )
    }
}

/// What `validate` read, so a clean run over nothing reads differently from a
/// clean run over a populated register.
///
/// Only kinds the profile declares are counted; a kind the register names and
/// the profile does not is `UNKNOWN_KIND`'s finding. A declared kind with no
/// instance is present with 0.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ValidateScope {
    /// Declared node kind → its declared nodes.
    pub nodes: BTreeMap<String, usize>,
    /// Declared edge kind → its edges.
    pub edges: BTreeMap<String, usize>,
    /// Distinct source files the provenance of those nodes and edges names. A
    /// file that matched a glob and declared nothing is not among them.
    pub files: usize,
}

/// Validation's answer: its findings, in the order `validate` sorts them, and
/// the scope they were found over.
///
/// A struct rather than the bare list so the answer can carry more than its
/// findings without a breaking change.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ValidateReport {
    /// Profile findings, adapter issues resolved with them.
    pub findings: Vec<Issue>,
    /// What the graph held, per declared kind.
    pub scope: ValidateScope,
}

/// What `summary` has to show.
///
/// That depends on whether the profile said what a summary means. A `SUMMARY` config selects the status rollup; without one
/// the command falls back to structural statistics rather than refusing.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SummaryReport {
    /// Profile-configured status rollup.
    Configured(StatusRollup),
    /// Zero-configuration structural summary.
    Structural(StructuralSummary),
}

/// Zero-config summary payload: counts by kind and finding tallies.
///
/// Every kind the profile declares gets an entry even at zero, so an empty
/// register reads as "declared but unused" rather than as "unknown".
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StructuralSummary {
    /// Node counts keyed by declared kind.
    pub node_counts: BTreeMap<String, i64>,
    /// Edge counts keyed by declared kind.
    pub edge_counts: BTreeMap<String, i64>,
    /// Finding counts grouped by code and severity.
    pub finding_counts: Vec<FindingTally>,
}

/// How many findings one code produced at one severity.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FindingTally {
    /// Stable finding code.
    pub code: String,
    /// Resolved severity.
    pub severity: Severity,
    /// Number of matching findings.
    pub count: i64,
}

/// Status rollup payload: one row per group, counts keyed by status.
///
/// `status_keys` is carried beside the counts rather than derived from them: the
/// columns come from the profile's declared enum as well as from what the
/// register holds, so a status no node currently carries still gets a zero
/// column instead of vanishing.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatusRollup {
    /// Attribute used to group nodes into rows.
    pub group_key: String,
    /// Status values shown as columns, sorted: the declared enum values and
    /// those the register holds, with `unknown` for nodes that carry none.
    pub status_keys: Vec<String>,
    /// Group labels and their per-status counts.
    pub groups: Vec<(String, BTreeMap<String, i64>)>,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
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
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NodeRef {
    /// Stable node identifier.
    pub id: String,
    /// Profile-declared node kind.
    pub kind: String,
    /// `Some(true)` when every path the walk found to this node crosses an
    /// undeclared endpoint; `None` when the walk was not asked to tell.
    pub tainted: Option<bool>,
}

/// An edge named in a query answer, as the (src, kind, tgt) triple it is.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct EdgeRef {
    /// Source node identifier.
    pub src: String,
    /// Target node identifier.
    pub tgt: String,
    /// Profile-declared edge kind.
    pub kind: String,
}

/// Whether the endpoint node was explicitly declared.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum Endpoint {
    /// The node exists in the graph.
    Declared,
    /// The node is referenced by an edge but never declared.
    Vacancy,
}

/// How a node was encountered during traversal.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub enum Visit {
    /// First encounter of this node.
    First,
    /// The node is an ancestor on the current DFS path.
    Cycle,
    /// The node was already visited via a different path.
    Reconvergence,
}

/// One step in a depth-first traversal report.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct TraversalStep<'a> {
    /// Distance from the root (root is 0).
    pub depth: usize,
    /// The edge traversed to reach this step; `None` for the root.
    pub edge: Option<&'a crate::graph::Edge>,
    /// The endpoint node ID.
    pub node_id: &'a str,
    /// Whether the endpoint was explicitly declared.
    pub endpoint: Endpoint,
    /// How this node was encountered.
    pub visit: Visit,
    /// Position among parallel edges between the same endpoints (0-indexed).
    pub parallel_ordinal: usize,
    /// Total parallel edges between the same endpoints.
    pub parallel_count: usize,
    /// True when every path from root crosses a vacancy.
    pub tainted: bool,
}

/// Complete traversal report from a root node.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct TraversalReport<'a> {
    /// Node ID at which traversal began.
    pub root: &'a str,
    /// Which way the walk followed edges.
    pub direction: crate::query::Direction,
    /// Ordered traversal steps.
    pub steps: Vec<TraversalStep<'a>>,
}

/// An allowed triple in the schema graph: edge kind connecting two node kinds.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
#[doc(hidden)]
pub struct AllowedTriple {
    /// Profile-declared edge kind.
    pub edge_kind: String,
    /// Source node kind.
    pub src_kind: String,
    /// Target node kind.
    pub tgt_kind: String,
}

/// The profile's type structure as a graph: node kinds are vertices, allowed
/// triples are edges.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct SchemaGraph {
    /// Kind names in profile declaration order.
    pub kinds: Vec<String>,
    /// Allowed triples sorted by (edge kind, source kind, target kind).
    pub triples: Vec<AllowedTriple>,
}

/// Instance counts joined onto a schema graph from a loaded register.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct SchemaJoinCounts {
    /// Per node kind: how many nodes of that kind exist.
    pub kind_counts: Vec<(String, usize)>,
    /// Per allowed triple: how many edges of that kind connect the specified
    /// source and target kinds.
    pub triple_counts: Vec<(AllowedTriple, usize)>,
}

/// One strongly connected component.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct Component {
    /// Member node IDs in sorted order.
    pub members: Vec<String>,
    /// True when the component has more than one member or a self-loop.
    pub cyclic: bool,
    /// True when a single member has an edge to itself.
    pub self_loop: bool,
}

/// An edge that crosses from one component to another.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct InterComponentEdge<'a> {
    /// Index of the source component in the condensation's component list.
    pub src_component: usize,
    /// Index of the target component in the condensation's component list.
    pub tgt_component: usize,
    /// The edge reference.
    pub edge: &'a crate::graph::Edge,
}

/// SCC condensation: components and inter-component edges.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct Condensation<'a> {
    /// Components in topological order.
    pub components: Vec<Component>,
    /// Edges between different components.
    pub inter_edges: Vec<InterComponentEdge<'a>>,
}

/// One row in a lane layout: a node (or condensed SCC cluster) with its
/// assigned lane and the lanes of its parent edges.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct LaneRow {
    /// Node ID, or first member ID for an SCC cluster.
    pub id: String,
    /// True when this row represents a condensed multi-member SCC.
    pub cluster: bool,
    /// All member IDs when `cluster` is true; single-element otherwise.
    pub members: Vec<String>,
    /// Assigned lane index (0-based).
    pub lane: usize,
    /// Lanes of parent rows that have edges into this row.
    pub parent_lanes: Vec<usize>,
}

/// Complete lane layout for a reachability cone.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct LaneLayout {
    /// Root node ID.
    pub origin: String,
    /// Rows in topological order.
    pub rows: Vec<LaneRow>,
    /// Peak number of simultaneously open lanes.
    pub peak_lanes: usize,
}

/// Reachability answer: the transitive closure from `origin`, origin excluded.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReachReport {
    /// Node ID at which traversal began.
    pub origin: String,
    /// Which way the walk followed edges.
    pub direction: crate::query::Direction,
    /// The restriction the caller asked for; empty means every edge kind.
    pub edge_kinds: Vec<String>,
    /// Reachable declared nodes, in ID order rather than the order the walk met them.
    pub nodes: Vec<NodeRef>,
}

/// Path answer: either a validated path or a not-connected result.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathReport {
    src: String,
    tgt: String,
    outcome: PathOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PathOutcome {
    Found {
        nodes: Vec<String>,
        edges: Vec<String>,
    },
    NotFound,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl PathReport {
    /// Build a found result when nodes and intervening edges form a valid path.
    ///
    /// # Errors
    ///
    /// A message naming the broken shape when `nodes` is empty, `edges` is not
    /// one shorter than `nodes`, or the ends are not `src` and `tgt`.
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
    /// Return the requested starting node ID.
    pub fn src(&self) -> &str {
        &self.src
    }

    #[must_use]
    /// Return the requested destination node ID.
    pub fn tgt(&self) -> &str {
        &self.tgt
    }

    #[must_use]
    /// Return the path's ordered nodes and intervening edge kinds, when found.
    pub fn found_path(&self) -> Option<(&[String], &[String])> {
        match &self.outcome {
            PathOutcome::Found { nodes, edges } => Some((nodes, edges)),
            PathOutcome::NotFound => None,
        }
    }
}

/// One orphan: a declared node no edge names.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OrphanEntry {
    /// Stable node identifier.
    pub id: String,
    /// Profile-declared node kind.
    pub kind: String,
    /// Source location of the node declaration.
    pub provenance: Provenance,
}

/// Orphans answer, with the kind restriction it was computed under.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OrphansReport {
    /// Requested kind restriction, or `None` for every kind.
    pub kind_filter: Option<String>,
    /// Matching disconnected nodes.
    pub orphans: Vec<OrphanEntry>,
}

/// Counts answer: per-kind tallies, declared kinds present even at zero.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CountsReport {
    /// Node counts keyed by declared kind.
    pub nodes: BTreeMap<String, i64>,
    /// Edge counts keyed by declared kind.
    pub edges: BTreeMap<String, i64>,
}

/// Coverage answer: per-kind edge-direction statistics, ordered by kind name.
/// Declared kinds appear even at zero, on the same terms as `CountsReport`.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CoverageReport {
    /// Coverage rows in kind-name order.
    pub kinds: Vec<KindCoverage>,
}

/// How many nodes of one kind exist and how many any edge enters or leaves.
///
/// Percentages ride along rather than being left to the reader: they are the
/// figure the report exists to give, and every consumer would otherwise round
/// them differently.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct KindCoverage {
    /// Profile-declared node kind.
    pub kind: String,
    /// Number of nodes of this kind.
    pub total: i64,
    /// Nodes with at least one incoming edge.
    pub incoming: i64,
    /// Nodes with at least one outgoing edge.
    pub outgoing: i64,
    /// Incoming share expressed from zero through one hundred.
    pub incoming_pct: f64,
    /// Outgoing share expressed from zero through one hundred.
    pub outgoing_pct: f64,
}

/// Two-revision diff answer: what the register gained, lost, and changed.
///
/// The two inputs are graphs the caller supplied, under opaque labels; nothing here
/// runs an adapter. Nodes, edges and pathways compare by semantic identity: findings
/// and provenance are excluded, so a declaration that merely moved lines or files does
/// not appear.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DiffReport {
    /// Opaque label for the earlier input.
    pub rev_a: String,
    /// Opaque label for the later input.
    pub rev_b: String,
    /// Nodes present only in the later input.
    pub nodes_added: Vec<NodeRef>,
    /// Nodes present only in the earlier input.
    pub nodes_removed: Vec<NodeRef>,
    /// Present at both revisions with a different kind or attrs; the kind
    /// shown is revision B's.
    pub nodes_changed: Vec<NodeChangedRef>,
    /// Edges present only in the later input.
    pub edges_added: Vec<EdgeRef>,
    /// Edges present only in the earlier input.
    pub edges_removed: Vec<EdgeRef>,
    /// Present at both revisions under the same (src, tgt, kind) with
    /// different attrs. Parallel edges pair up in document order.
    pub edges_changed: Vec<EdgeChangedRef>,
    /// Axes added, removed, or with a different order or current position.
    pub pathways_changed: Vec<String>,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl DiffReport {
    /// True when no node, edge or pathway changed. This is semantic identity,
    /// not the ingest identity of [`LatticeGraph`](crate::graph::LatticeGraph)'s
    /// equality: two graphs that declare the same nodes in another order, or at
    /// other lines, compare unequal and still diff empty.
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
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NodeChangedRef {
    /// Stable node identifier.
    pub id: String,
    /// Node kind in the later input.
    pub kind: String,
    /// Attribute map in the earlier input.
    pub attrs_a: serde_json::Map<String, serde_json::Value>,
    /// Attribute map in the later input.
    pub attrs_b: serde_json::Map<String, serde_json::Value>,
}

/// An edge whose attrs differ between the two diffed revisions: the identity
/// tuple and both attr maps, on the same terms as [`NodeChangedRef`].
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EdgeChangedRef {
    /// Source node identifier.
    pub src: String,
    /// Target node identifier.
    pub tgt: String,
    /// Profile-declared edge kind.
    pub kind: String,
    /// Attribute map in the earlier input.
    pub attrs_a: serde_json::Map<String, serde_json::Value>,
    /// Attribute map in the later input.
    pub attrs_b: serde_json::Map<String, serde_json::Value>,
}

/// One outgoing edge in a trace report, retaining attrs and provenance.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TraceEdge {
    /// Target node identifier.
    pub tgt: String,
    /// Profile-declared edge kind.
    pub kind: String,
    /// Edge attribute map.
    pub attrs: serde_json::Map<String, serde_json::Value>,
    /// Source location of the edge declaration.
    pub provenance: Provenance,
}

/// One node in a trace report, carrying its edges and attached findings.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TraceEntry {
    /// Stable node identifier.
    pub id: String,
    /// Profile-declared node kind.
    pub kind: String,
    /// Node attribute map. Its iteration order is `serde_json::Map`'s and not
    /// part of the contract; trace JSON sorts it by name regardless.
    pub attrs: serde_json::Map<String, Value>,
    /// Source location of the node declaration.
    pub provenance: Provenance,
    /// The attr to show in the trace row's summary column, resolved from the profile.
    pub summary_attr: Option<String>,
    /// Outgoing edges, already ordered by kind, target, and provenance.
    pub edges: Vec<TraceEdge>,
    /// Findings attached to this node.
    pub findings: Vec<Issue>,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl TraceEntry {
    /// Repeated edges count once each, because the trace preserves rather than
    /// deduplicates them.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

/// The answer to `query at`: what the register holds from one source path.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PathwayEntry {
    /// Profile-declared pathway name.
    pub name: String,
    /// Valid positions in declared order.
    pub order: Vec<String>,
    /// Current position on the pathway.
    pub current: String,
}

/// Full trace-report payload: header, node entries, findings, and pathways.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TraceReport {
    /// Stable report metadata such as profile and core versions.
    pub header: BTreeMap<String, String>,
    /// Per-node trace entries.
    pub entries: Vec<TraceEntry>,
    /// Findings with no `node_id`, or one naming no explicitly added node. They
    /// belong to no entry, and dropping them would hide a finding that still
    /// moves the exit code.
    pub unattachable_findings: Vec<Issue>,
    /// Ordering pathways carried from the graph.
    pub pathways: Vec<PathwayEntry>,
}

/// Source attribution kept separately from the original file and line.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct FuseProvenance {
    /// Manifest name of the source the location is in.
    pub source: String,
    /// File and line within that source's target, unqualified.
    pub location: Provenance,
}

/// A node in the composed graph, as its source declared it.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct FuseNode {
    /// Source-qualified ID, `source:id`.
    pub id: String,
    /// Source-qualified kind, `source:kind`.
    pub kind: String,
    /// The node's attrs, unchanged.
    pub attrs: serde_json::Map<String, Value>,
    /// Where the source declares the node.
    pub provenance: FuseProvenance,
}

/// An edge in the composed graph. `src` and `tgt` are source-qualified; `kind`
/// is the edge kind as declared, unqualified.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct FuseEdge {
    /// Source-qualified ID of the edge's source node.
    pub src: String,
    /// Target ID qualified with the edge's own source, or the resolved node's
    /// ID when a join resolves it into another source.
    pub tgt: String,
    /// Edge kind as declared, unqualified.
    pub kind: String,
    /// The edge's attrs, unchanged.
    pub attrs: serde_json::Map<String, Value>,
    /// Where the source declares the edge.
    pub provenance: FuseProvenance,
    /// Source-qualified kind of the `src` node.
    pub source_kind: String,
    /// Qualified kind of the target node, once composition resolves it.
    pub target_kind: Option<String>,
    /// Manifest source of the resolved target node; set with `target_kind`.
    pub target_source: Option<String>,
}

/// A finding in the composed report, with the source it came from.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct FuseFinding {
    /// The finding, its node ID source-qualified where it names one.
    pub issue: Issue,
    /// Manifest source the finding belongs to; `None` for a cross-source finding.
    pub source: Option<String>,
    /// Every location of a cross-source finding, such as a raw ID repeated
    /// across sources; empty otherwise.
    pub locations: Vec<FuseProvenance>,
}

/// `describe`'s answer: what the profile declares and what the adapter read
/// under it, with nothing validated. The CLI's orientation view; a host reads
/// the graph and profile directly.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct DescribeReport {
    /// The profile's `name` and `profile_version`.
    pub profile: DescribeProfile,
    /// The adapter that ran, and its `adapter:` block as configured.
    pub adapter: DescribeAdapter,
    /// Every declared node kind, in name order.
    pub node_kinds: Vec<DescribeNodeKind>,
    /// Every declared edge kind, in name order.
    pub edge_kinds: Vec<DescribeEdgeKind>,
    /// Every `validations:` entry, by code.
    pub validations: Vec<DescribeValidation>,
    /// The adapter's own findings, as `validate` reports them.
    pub findings: Vec<Issue>,
}

/// The profile `describe` ran under.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct DescribeProfile {
    /// The profile's `name`.
    pub name: String,
    /// Its `profile_version`.
    pub version: String,
}

/// The adapter `describe` ran.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct DescribeAdapter {
    /// A native adapter's name, or the external program as named.
    pub name: String,
    /// The profile's `adapter:` block as the adapter receives it in the
    /// resolved profile; `None` when the profile has none. The JSON form reads
    /// this.
    pub config: Option<serde_json::Value>,
    /// The same block as YAML text, keys in the profile's order, for a reader;
    /// `None` when the profile has none. Plain and rich read this.
    pub config_yaml: Option<String>,
}

/// One declared node kind and what the register holds of it.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct DescribeNodeKind {
    /// The kind name.
    pub kind: String,
    /// Its `id_pattern` as the profile wrote it.
    pub id_pattern: String,
    /// Declared nodes of this kind.
    pub count: usize,
    /// The distinct source files those nodes came from, in path order.
    pub files: Vec<String>,
}

/// One declared edge kind and what the register holds of it.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct DescribeEdgeKind {
    /// The kind name.
    pub kind: String,
    /// Its `allowed` `(source kind, target kind)` pairs, as declared.
    pub allowed: Vec<(String, String)>,
    /// Edges of this kind.
    pub count: usize,
}

/// One `validations:` entry.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct DescribeValidation {
    /// The entry's code.
    pub code: String,
    /// The entry's own `severity`, when it sets one.
    pub severity: Option<Severity>,
}

/// Composed graph and findings, in manifest source order. A failed source leaves
/// the graph empty; findings from healthy sources still remain visible.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct FuseReport {
    /// The manifest's `name`, `version` and `manifest_version`, and the
    /// report's `fuse_version`.
    pub header: BTreeMap<String, String>,
    /// Composed nodes, source by source.
    pub nodes: Vec<FuseNode>,
    /// Composed edges, source by source.
    pub edges: Vec<FuseEdge>,
    /// Every source's ordering pathways, names source-qualified.
    pub pathways: Vec<PathwayEntry>,
    /// Every source's findings, then the composition's own.
    pub findings: Vec<FuseFinding>,
    /// False when a source failed, so there is no composed graph to report;
    /// [`exit_code`](Self::exit_code) maps it to 2.
    pub could_run: bool,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl FuseReport {
    /// Return the report of a composition that could not start: no graph, a
    /// header carrying only `fuse_version`, and `issue` as its one finding, from
    /// no source.
    #[must_use]
    pub fn failed(issue: Issue) -> Self {
        Self {
            header: BTreeMap::from([(
                "fuse_version".to_owned(),
                crate::fuse::FUSE_VERSION.to_owned(),
            )]),
            findings: vec![FuseFinding {
                issue,
                source: None,
                locations: Vec::new(),
            }],
            ..Self::default()
        }
    }

    /// The process exit code: 2 when the composition could not run, 1 when a
    /// finding gates, 0 otherwise.
    #[must_use]
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

/// A node in a Sugiyama layered graph layout.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[doc(hidden)]
pub enum LayoutNode {
    /// Index into `GraphLayout.components`.
    Real(usize),
    /// Virtual node for a long edge; index into `GraphLayout.edges`.
    Virtual(usize),
}

/// An edge in the Sugiyama graph layout (between component indices).
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct LayoutEdge {
    /// Index into `GraphLayout.components`.
    pub src_component: usize,
    /// Index into `GraphLayout.components`.
    pub tgt_component: usize,
    /// Kind of the first edge met between the two components; parallel edges
    /// of other kinds share this one layout edge.
    pub kind: String,
}

/// Sugiyama layered DAG layout for a rooted subgraph.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[doc(hidden)]
pub struct GraphLayout {
    /// Condensed components (SCCs collapsed).
    pub components: Vec<Component>,
    /// Nodes per layer, ordered after crossing minimization.
    pub layers: Vec<Vec<LayoutNode>>,
    /// Edges between components.
    pub edges: Vec<LayoutEdge>,
    /// Per-layer, per-node x position from the PR horizontal placement.
    pub x_positions: Vec<Vec<usize>>,
    /// Component index of the center node.
    pub center_component: usize,
}

#[cfg(test)]
mod civil_date_tests {
    use super::CivilDate;

    #[test]
    fn civil_date_parses_iso_dates() {
        let date = CivilDate::parse("2026-12-31").unwrap();
        assert_eq!((date.year(), date.month(), date.day()), (2026, 12, 31));
        assert_eq!(date.to_string(), "2026-12-31");
    }

    #[test]
    fn civil_date_rejects_impossible_and_malformed_dates() {
        for text in [
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "2026-04-31",
            "2025-02-29",
            "2026-1-01",
            "2026-01-01T00:00",
            "20260101",
            "",
            "２026-01-01",
        ] {
            assert_eq!(CivilDate::parse(text), None, "{text}");
        }
    }

    #[test]
    fn civil_date_knows_leap_years() {
        assert!(CivilDate::parse("2024-02-29").is_some());
        assert!(CivilDate::parse("2000-02-29").is_some());
        assert!(CivilDate::parse("1900-02-29").is_none());
    }

    #[test]
    fn civil_date_orders_by_calendar() {
        let earlier = CivilDate::parse("2026-09-30").unwrap();
        let later = CivilDate::parse("2026-10-01").unwrap();
        assert!(earlier < later);
        assert!(CivilDate::parse("2025-12-31").unwrap() < earlier);
    }

    #[test]
    fn civil_date_from_unix_days() {
        assert_eq!(CivilDate::from_unix_days(0).to_string(), "1970-01-01");
        assert_eq!(CivilDate::from_unix_days(59).to_string(), "1970-03-01");
        assert_eq!(CivilDate::from_unix_days(11_016).to_string(), "2000-02-29");
        assert_eq!(CivilDate::from_unix_days(20_727).to_string(), "2026-10-01");
        assert_eq!(CivilDate::from_unix_days(-1).to_string(), "1969-12-31");
    }

    #[test]
    fn civil_date_from_unix_days_saturates_at_the_representable_range() {
        let first = CivilDate::new(i32::MIN, 1, 1).unwrap();
        let last = CivilDate::new(i32::MAX, 12, 31).unwrap();
        assert_eq!(CivilDate::from_unix_days(CivilDate::MIN_DAYS), first);
        assert_eq!(CivilDate::from_unix_days(CivilDate::MAX_DAYS), last);
        assert_eq!(CivilDate::from_unix_days(i64::MIN), first);
        assert_eq!(CivilDate::from_unix_days(i64::MAX), last);
        assert_eq!(CivilDate::from_unix_days(CivilDate::MIN_DAYS - 1), first);
        assert_eq!(CivilDate::from_unix_days(CivilDate::MAX_DAYS + 1), last);
    }

    /// The calendar day after `date`, worked out from `new`'s own validation
    /// rather than from day arithmetic.
    fn next_day(date: CivilDate) -> CivilDate {
        let (y, m, d) = (date.year(), date.month(), date.day());
        CivilDate::new(y, m, d + 1)
            .or_else(|| CivilDate::new(y, m + 1, 1))
            .or_else(|| CivilDate::new(y + 1, 1, 1))
            .unwrap()
    }

    #[test]
    fn civil_date_from_unix_days_steps_one_calendar_day() {
        let span = CivilDate::MAX_DAYS - CivilDate::MIN_DAYS;
        let strided = (0..20_000).map(|i| CivilDate::MIN_DAYS + i * (span / 20_000));
        let near_epoch = -800..800;
        let near_min = CivilDate::MIN_DAYS..CivilDate::MIN_DAYS + 800;
        let near_max = CivilDate::MAX_DAYS - 800..CivilDate::MAX_DAYS;
        for days in strided.chain(near_epoch).chain(near_min).chain(near_max) {
            let date = CivilDate::from_unix_days(days);
            assert_eq!(
                CivilDate::from_unix_days(days + 1),
                next_day(date),
                "{days}"
            );
        }
    }

    #[test]
    fn civil_date_today_is_a_real_date() {
        let today = CivilDate::today_utc();
        assert_eq!(CivilDate::parse(&today.to_string()), Some(today));
    }

    #[test]
    fn civil_date_unquoted_yaml_dates_arrive_as_strings() {
        let value: serde_norway::Value = serde_norway::from_str("until: 2026-12-31").unwrap();
        assert_eq!(
            value.get("until"),
            Some(&serde_norway::Value::String("2026-12-31".into()))
        );
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::Provenance;

    #[test]
    fn line_zero_reads_as_no_line() {
        let whole_file = Provenance::new("r.md", 0);
        assert_eq!(whole_file.file(), "r.md");
        assert_eq!(whole_file.line(), None);
        assert_eq!(
            Provenance::new("r.md", 3).line().map(|line| line.get()),
            Some(3)
        );
    }
}
