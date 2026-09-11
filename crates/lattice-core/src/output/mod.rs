//! Rendering findings in the three output formats.
//!
//! One dispatch point, so a format is defined once for every command. Formatting
//! at a call site is how the three drift apart.

mod fuse;
mod query;
mod summary;
mod trace;
mod validate;

use std::borrow::Cow;

use crate::types::{
    AtReport, CountsReport, CoverageReport, DiffReport, FuseReport, Issue, OrphansReport,
    PathReport, PathwayEntry, ReachReport, Severity, SummaryReport, TraceReport,
};
use serde::Serialize as DeriveSerialize;
use serde::ser::{Serialize, SerializeSeq, Serializer};

const SEVERITY_COLORS: [(Severity, &str); 4] = [
    (Severity::Error, "\u{1b}[31m"),
    (Severity::Warning, "\u{1b}[33m"),
    (Severity::Info, "\u{1b}[36m"),
    (Severity::Hint, "\u{1b}[35m"),
];
const RESET: &str = "\u{1b}[0m";

fn color_for(severity: Severity) -> &'static str {
    SEVERITY_COLORS
        .iter()
        .find(|(s, _)| *s == severity)
        .map(|(_, c)| *c)
        .unwrap_or("")
}

/// Pad to `width` on the right, leaving anything longer untouched.
fn ljust(text: &str, width: usize) -> String {
    let mut out = text.to_string();
    for _ in text.chars().count()..width {
        out.push(' ');
    }
    out
}

/// `ljust`'s mirror. Counts codepoints for the same reason.
fn rjust(text: &str, width: usize) -> String {
    let mut out = String::new();
    for _ in text.chars().count()..width {
        out.push(' ');
    }
    out.push_str(text);
    out
}

/// The first `width` characters, counted as codepoints the way Python slices.
fn truncate(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

/// Python's `str.capitalize`: upper-case the first character, lower-case the rest.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
    }
}

/// Escape every non-ASCII character as `\uXXXX`, the way Python's `json.dumps`
/// does by default.
///
/// `serde_json` emits raw UTF-8, so a finding quoting non-ASCII source text
/// would serialize differently from the reference core. Applied to the
/// rendered document rather than through a custom `Formatter`: non-ASCII can
/// only occur inside JSON string literals.
fn escape_non_ascii(json: &str) -> Cow<'_, str> {
    if json.is_ascii() {
        return Cow::Borrowed(json);
    }
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    Cow::Owned(out)
}

#[derive(DeriveSerialize)]
struct ProvenanceJson<'a> {
    file: &'a str,
    line: i64,
}

impl<'a> From<&'a crate::types::Provenance> for ProvenanceJson<'a> {
    fn from(provenance: &'a crate::types::Provenance) -> Self {
        Self {
            file: &provenance.file,
            line: provenance.line,
        }
    }
}

#[derive(DeriveSerialize)]
struct FindingJson<'a> {
    code: &'a str,
    file: &'a str,
    line: i64,
    message: &'a str,
    node_id: &'a Option<String>,
    severity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    suppressed: bool,
}

impl<'a> From<&'a Issue> for FindingJson<'a> {
    fn from(issue: &'a Issue) -> Self {
        Self {
            code: &issue.code,
            file: &issue.provenance.file,
            line: issue.provenance.line,
            message: &issue.message,
            node_id: &issue.node_id,
            severity: issue.severity.as_str(),
            state: issue.state.as_deref(),
            suppressed: issue.suppressed,
        }
    }
}

/// The findings a human-facing format shows: `plain` and `rich` leave out what
/// the profile suppressed, the one declared divergence from `json`.
pub(crate) fn visible(issues: &[Issue]) -> impl Iterator<Item = &Issue> {
    issues.iter().filter(|issue| !issue.suppressed)
}

struct PathwaysJson<'a>(&'a [PathwayEntry]);

impl Serialize for PathwaysJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for pathway in self.0 {
            sequence.serialize_element(&PathwayEntryJson {
                name: &pathway.name,
                order: &pathway.order,
                current: &pathway.current,
            })?;
        }
        sequence.end()
    }
}

#[derive(DeriveSerialize)]
struct PathwayEntryJson<'a> {
    name: &'a str,
    order: &'a [String],
    current: &'a str,
}

/// Serialize a payload the way `json.dumps(indent=2, sort_keys=True)` would.
fn render_json(value: &(impl Serialize + ?Sized)) -> String {
    let rendered = serde_json::to_string_pretty(value).expect("payloads are serializable");
    match escape_non_ascii(&rendered) {
        Cow::Borrowed(_) => rendered,
        Cow::Owned(escaped) => escaped,
    }
}

/// Drop ANSI escape sequences from rendered output.
///
/// `rich` colours unconditionally; the stripping lives in `click.echo`, which
/// removes escapes when its stream is not a terminal.
#[must_use]
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' || chars.peek() != Some(&'[') {
            out.push(c);
            continue;
        }
        chars.next();
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
            if !matches!(c, ';' | '?' | '0'..='9') {
                out.push(c);
                break;
            }
        }
    }
    out
}

/// An unrecognised `--format`, which is a caller error rather than a finding.
#[derive(Debug)]
pub struct UnknownFormat(pub String);

impl std::fmt::Display for UnknownFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown format: {}", self.0)
    }
}

impl std::error::Error for UnknownFormat {}

/// What a command has to show. The dispatcher picks its formatters from this,
/// so a new payload adds three formatters and no new printing path.
#[non_exhaustive]
#[derive(Debug)]
pub enum Payload<'a> {
    Findings(&'a [Issue]),
    Summary(&'a SummaryReport),
    Trace(&'a TraceReport),
    Fuse(&'a FuseReport),
    At(&'a AtReport),
    Reach(&'a ReachReport),
    Path(&'a PathReport),
    Orphans(&'a OrphansReport),
    Counts(&'a CountsReport),
    Coverage(&'a CoverageReport),
    Diff(&'a DiffReport),
}

impl<'a> From<&'a [Issue]> for Payload<'a> {
    fn from(issues: &'a [Issue]) -> Self {
        Payload::Findings(issues)
    }
}

impl<'a> From<&'a Vec<Issue>> for Payload<'a> {
    fn from(issues: &'a Vec<Issue>) -> Self {
        Payload::Findings(issues)
    }
}

impl<'a> From<&'a SummaryReport> for Payload<'a> {
    fn from(report: &'a SummaryReport) -> Self {
        Payload::Summary(report)
    }
}

impl<'a> From<&'a FuseReport> for Payload<'a> {
    fn from(report: &'a FuseReport) -> Self {
        Payload::Fuse(report)
    }
}

impl<'a> From<&'a TraceReport> for Payload<'a> {
    fn from(report: &'a TraceReport) -> Self {
        Payload::Trace(report)
    }
}

impl<'a> From<&'a AtReport> for Payload<'a> {
    fn from(report: &'a AtReport) -> Self {
        Payload::At(report)
    }
}

impl<'a> From<&'a ReachReport> for Payload<'a> {
    fn from(report: &'a ReachReport) -> Self {
        Payload::Reach(report)
    }
}

impl<'a> From<&'a PathReport> for Payload<'a> {
    fn from(report: &'a PathReport) -> Self {
        Payload::Path(report)
    }
}

impl<'a> From<&'a OrphansReport> for Payload<'a> {
    fn from(report: &'a OrphansReport) -> Self {
        Payload::Orphans(report)
    }
}

impl<'a> From<&'a CountsReport> for Payload<'a> {
    fn from(report: &'a CountsReport) -> Self {
        Payload::Counts(report)
    }
}

impl<'a> From<&'a CoverageReport> for Payload<'a> {
    fn from(report: &'a CoverageReport) -> Self {
        Payload::Coverage(report)
    }
}

impl<'a> From<&'a DiffReport> for Payload<'a> {
    fn from(report: &'a DiffReport) -> Self {
        Payload::Diff(report)
    }
}

/// Render a payload in the named format.
pub fn output_result<'a>(
    payload: impl Into<Payload<'a>>,
    format: &str,
) -> Result<String, UnknownFormat> {
    match payload.into() {
        Payload::Findings(issues) => {
            // Sorted borrowed, not cloned. `validate` has already sorted what
            // it returns, but the sort stays because a caller may arrive with
            // findings in collection order.
            let mut sorted: Vec<&Issue> = issues.iter().collect();
            sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
            match format {
                "plain" => Ok(validate::format_plain(&sorted)),
                "json" => Ok(validate::format_json(&sorted)),
                "rich" => Ok(validate::format_rich(&sorted)),
                other => Err(UnknownFormat(other.to_string())),
            }
        }
        Payload::Summary(SummaryReport::Configured(report)) => match format {
            "plain" => Ok(summary::format_summary_plain(report)),
            "json" => Ok(summary::format_summary_json(report)),
            "rich" => Ok(summary::format_summary_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Summary(SummaryReport::Structural(report)) => match format {
            "plain" => Ok(summary::format_structural_plain(report)),
            "json" => Ok(summary::format_structural_json(report)),
            "rich" => Ok(summary::format_structural_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Fuse(report) => match format {
            "plain" => Ok(fuse::format_fuse_plain(report)),
            "json" => Ok(fuse::format_fuse_json(report)),
            "rich" => Ok(fuse::format_fuse_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Trace(report) => match format {
            "plain" => Ok(trace::format_trace_plain(report)),
            "json" => Ok(trace::format_trace_json(report)),
            "rich" => Ok(trace::format_trace_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::At(report) => match format {
            "plain" => Ok(query::format_at_plain(report)),
            "json" => Ok(query::format_at_json(report)),
            "rich" => Ok(query::format_at_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Reach(report) => match format {
            "plain" => Ok(query::format_reach_plain(report)),
            "json" => Ok(query::format_reach_json(report)),
            "rich" => Ok(query::format_reach_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Path(report) => match format {
            "plain" => Ok(query::format_path_plain(report)),
            "json" => Ok(query::format_path_json(report)),
            "rich" => Ok(query::format_path_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Orphans(report) => match format {
            "plain" => Ok(query::format_orphans_plain(report)),
            "json" => Ok(query::format_orphans_json(report)),
            "rich" => Ok(query::format_orphans_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Counts(report) => match format {
            "plain" => Ok(query::format_counts_plain(report)),
            "json" => Ok(query::format_counts_json(report)),
            "rich" => Ok(query::format_counts_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Coverage(report) => match format {
            "plain" => Ok(query::format_coverage_plain(report)),
            "json" => Ok(query::format_coverage_json(report)),
            "rich" => Ok(query::format_coverage_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Diff(report) => match format {
            "plain" => Ok(query::format_diff_plain(report)),
            "json" => Ok(query::format_diff_json(report)),
            "rich" => Ok(query::format_diff_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
    }
}
