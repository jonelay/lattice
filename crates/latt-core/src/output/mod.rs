//! Rendering findings in the three output formats.
//!
//! One dispatch point, so a format is defined once for every command. Formatting
//! at a call site is how the three drift apart.

// A raw `serde_json` map or value here would serialize in whatever order a
// host's feature unification chose; `sorted` holds the one exception.
#![deny(clippy::disallowed_types)]

mod describe;
mod fuse;
mod query;
mod sorted;
mod summary;
mod trace;
mod validate;
#[doc(hidden)]
pub use validate::scope_line;

use std::borrow::Cow;
use std::fmt::Write as _;
use std::io;

use serde_json::ser::{Formatter as JsonFormatter, PrettyFormatter};

use crate::types::{
    AtReport, CountsReport, CoverageReport, DiffReport, FuseReport, Issue, Note, OrphansReport,
    PathReport, PathwayEntry, ReachReport, RuleCoverageReport, Severity, SummaryReport,
    TraceReport, Until,
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
        .map_or("", |(_, c)| *c)
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

/// `serde_json`'s pretty layout, with every non-ASCII character in a string
/// escaped as `\uXXXX` the way Python's `json.dumps` does by default.
///
/// `serde_json` emits raw UTF-8, so a finding quoting non-ASCII source text
/// would serialize differently from the reference core. Escaping as the
/// serializer writes keeps the document in one buffer: non-ASCII can only occur
/// inside JSON string literals, which reach `write_string_fragment`.
struct AsciiPretty(PrettyFormatter<'static>);

impl JsonFormatter for AsciiPretty {
    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut rest = fragment;
        while let Some(at) = rest.find(|c: char| !c.is_ascii()) {
            writer.write_all(&rest.as_bytes()[..at])?;
            let c = rest[at..].chars().next().expect("found at this offset");
            let mut units = [0u16; 2];
            for &unit in c.encode_utf16(&mut units).iter() {
                let digits = [12, 8, 4, 0].map(|shift| HEX[usize::from((unit >> shift) & 0xf)]);
                writer.write_all(b"\\u")?;
                writer.write_all(&digits)?;
            }
            rest = &rest[at + c.len_utf8()..];
        }
        writer.write_all(rest.as_bytes())
    }

    fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_array(w)
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_array(w)
    }
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.0.begin_array_value(w, first)
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_array_value(w)
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_object(w)
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_object(w)
    }
    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.0.begin_object_key(w, first)
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.begin_object_value(w)
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.0.end_object_value(w)
    }
}

#[derive(DeriveSerialize)]
struct ProvenanceJson<'a> {
    file: &'a str,
    line: u32,
}

impl<'a> From<&'a crate::types::Provenance> for ProvenanceJson<'a> {
    fn from(provenance: &'a crate::types::Provenance) -> Self {
        Self {
            file: provenance.file(),
            line: provenance.wire_line(),
        }
    }
}

/// `node_id` is always present, as `null` for a finding about no node. A wire
/// decision: a consumer keys on the field without first testing for it.
#[derive(DeriveSerialize)]
struct FindingJson<'a> {
    code: &'a str,
    file: &'a str,
    line: u32,
    message: &'a str,
    node_id: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<NoteJson<'a>>,
    severity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    suppressed: bool,
}

/// A finding's note on the wire: absent keys for what the entry left out, and
/// `expired` only when true, so a note reads as the entry was written.
#[derive(DeriveSerialize)]
pub(crate) struct NoteJson<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    by: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    expired: bool,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    until: Option<UntilJson<'a>>,
}

/// A note's `until`: a bare date, or the pathway position it waits for.
#[derive(DeriveSerialize)]
#[serde(untagged)]
enum UntilJson<'a> {
    Date(String),
    Pathway { pathway: &'a str, position: &'a str },
}

impl<'a> From<&'a Note> for NoteJson<'a> {
    fn from(note: &'a Note) -> Self {
        Self {
            at: note.at.map(|date| date.to_string()),
            by: note.by.as_deref(),
            expired: note.expired,
            text: &note.text,
            until: note.until.as_ref().map(|until| match until {
                Until::Date(date) => UntilJson::Date(date.to_string()),
                Until::Pathway { pathway, position } => UntilJson::Pathway { pathway, position },
            }),
        }
    }
}

/// The note as the human formats print it: `note: `, `(expired) ` when it has
/// expired, the text, then `by` and `until` in brackets when declared.
pub(crate) fn note_text(note: &Note) -> String {
    let mut text = String::from("note: ");
    if note.expired {
        text.push_str("(expired) ");
    }
    text.push_str(&sanitize_text(&note.text));
    let mut qualifiers: Vec<String> = note
        .by
        .iter()
        .map(|by| sanitize_text(by).into_owned())
        .collect();
    if let Some(until) = &note.until {
        let until = crate::notes::until_text(until);
        qualifiers.push(format!("until {}", sanitize_text(&until)));
    }
    if !qualifiers.is_empty() {
        let _ = write!(text, " [{}]", qualifiers.join(", "));
    }
    text
}

/// A plain finding line with its note, if any, appended: plain keeps one line
/// per finding.
pub(crate) fn with_plain_note(line: String, issue: &Issue) -> String {
    match &issue.note {
        Some(note) => format!("{line} | {}", note_text(note)),
        None => line,
    }
}

impl<'a> From<&'a Issue> for FindingJson<'a> {
    fn from(issue: &'a Issue) -> Self {
        Self {
            code: &issue.code,
            file: issue.origin.wire_file(),
            line: issue.origin.wire_line(),
            message: &issue.message,
            node_id: &issue.node_id,
            note: issue.note.as_ref().map(NoteJson::from),
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
    current: &'a str,
    name: &'a str,
    order: &'a [String],
}

/// Serialize a payload as the pinned wire form.
///
/// Struct-shaped objects emit their fields in declaration order: that order
/// is the contract, and the byte baselines pin it. Map-shaped objects, the
/// attrs and anything nested in them, go through `SortedMap`, which emits
/// keys in byte order whatever `serde_json` features a host unified. This is
/// not `json.dumps(sort_keys=True)`: some structs are not alphabetical.
fn render_json(value: &(impl Serialize + ?Sized)) -> String {
    let mut out = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut out, AsciiPretty(PrettyFormatter::new()));
    value
        .serialize(&mut serializer)
        .expect("payloads are serializable");
    String::from_utf8(out).expect("the serializer emits UTF-8")
}

/// Drop ANSI escape sequences from rendered output.
///
/// [`Format::Rich`] colors unconditionally, so a caller writing its output
/// anywhere but a terminal strips it first; the CLI does so for any stream
/// that is not a terminal.
///
/// ```
/// use latt_core::output::strip_ansi;
/// assert_eq!(strip_ansi("\u{1b}[31mERROR\u{1b}[0m x"), "ERROR x");
/// ```
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[must_use]
#[doc(hidden)]
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

/// Write terminal control text in `text` as a visible escape, for `plain` and
/// `rich` output and stderr notices.
///
/// Register text (a node ID, a message, a path) reaches the terminal through
/// the human formats; an escape sequence or a newline in it could recolour the
/// screen, plant a link or forge a finding line. Escaped: C0 controls except
/// tab, DEL, C1 controls, the bidi controls and U+2028/U+2029. A newline is
/// written `\n`, a carriage return `\r`, anything else `\u{..}` in lower-case
/// hex. Text with none of them comes back borrowed. `json` stays raw: it is
/// the lossless channel.
///
/// ```
/// use latt_core::output::sanitize_text;
/// assert_eq!(sanitize_text("a\u{1b}[2Jb\nc\td"), "a\\u{1b}[2Jb\\nc\td");
/// ```
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[must_use]
#[doc(hidden)]
pub fn sanitize_text(text: &str) -> Cow<'_, str> {
    let Some(first) = text.find(is_terminal_control) else {
        return Cow::Borrowed(text);
    };
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str(&text[..first]);
    for c in text[first..].chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if is_terminal_control(c) => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// The characters [`sanitize_text`] escapes.
fn is_terminal_control(c: char) -> bool {
    matches!(
        c,
        '\u{0}'..='\u{8}'
            | '\u{a}'..='\u{1f}'
            | '\u{7f}'..='\u{9f}'
            | '\u{61c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// The renderings every payload has. `--format` parses straight into it, so an
/// unknown format is a usage error at the parser and never reaches here.
///
/// A format can be added in a minor release, so a `match` on one needs a
/// wildcard arm.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Format {
    /// Uncoloured text.
    Plain,
    /// The JSON payload.
    Json,
    /// Text coloured with ANSI escapes.
    Rich,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl Format {
    /// Every format, in the order the CLI lists them.
    pub const ALL: &'static [Format] = &[Format::Plain, Format::Json, Format::Rich];

    /// The `--format` spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Plain => "plain",
            Format::Json => "json",
            Format::Rich => "rich",
        }
    }
}

/// A spelling that names no [`Format`].
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown format '{value}': expected plain, json or rich")]
#[non_exhaustive]
pub struct ParseFormatError {
    value: String,
}

#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
impl ParseFormatError {
    /// Return the spelling that was refused.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Parse the `--format` spelling that [`Format::as_str`] returns.
///
/// ```
/// use latt_core::output::Format;
///
/// assert_eq!("json".parse::<Format>()?, Format::Json);
/// assert!("xml".parse::<Format>().is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
impl std::str::FromStr for Format {
    type Err = ParseFormatError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Format::ALL
            .iter()
            .copied()
            .find(|format| format.as_str() == value)
            .ok_or_else(|| ParseFormatError {
                value: value.to_owned(),
            })
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a command has to show. The dispatcher picks its formatters from this,
/// so a new payload adds three formatters and no new printing path.
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[non_exhaustive]
#[derive(Debug)]
pub enum Payload<'a> {
    /// Findings alone: adapter issues on stderr, a suggestion overlay.
    Findings(&'a [Issue]),
    /// `validate`'s answer: its findings and the scope they were found over.
    Validate(&'a crate::types::ValidateReport),
    /// `summary`'s rollup or structural report.
    Summary(&'a SummaryReport),
    /// `trace`'s report.
    Trace(&'a TraceReport),
    /// `fuse`'s composed report.
    #[doc(hidden)]
    Fuse(&'a FuseReport),
    /// `describe`'s orientation view.
    #[doc(hidden)]
    Describe(&'a crate::types::DescribeReport),
    /// `query at`'s answer: what the register holds from one source path.
    At(&'a AtReport),
    /// A reach query's answer.
    Reach(&'a ReachReport),
    /// A path query's answer.
    Path(&'a PathReport),
    /// An orphans query's answer.
    Orphans(&'a OrphansReport),
    /// A counts query's answer.
    Counts(&'a CountsReport),
    /// A coverage query's answer.
    Coverage(&'a CoverageReport),
    /// A direct coverage rule with its input uncertainty.
    RuleCoverage(&'a RuleCoverageReport),
    /// A comparison of two registers.
    Diff(&'a DiffReport),
}

// Service reports select the same payload; no transport envelope enters the rendered output.
impl<'a> From<&'a crate::service::Report> for Payload<'a> {
    fn from(report: &'a crate::service::Report) -> Self {
        use crate::service::Report;
        match report {
            Report::Validate(value) => Self::Validate(value),
            Report::Summary(value) => Self::Summary(value),
            Report::Trace(value) => Self::Trace(value),
            Report::At(value) => Self::At(value),
            Report::Reach(value) => Self::Reach(value),
            Report::Path(value) => Self::Path(value),
            Report::Orphans(value) => Self::Orphans(value),
            Report::Counts(value) => Self::Counts(value),
            Report::Coverage(value) => Self::Coverage(value),
            Report::RuleCoverage(value) => Self::RuleCoverage(value),
        }
    }
}

impl<'a> From<&'a [Issue]> for Payload<'a> {
    fn from(issues: &'a [Issue]) -> Self {
        Payload::Findings(issues)
    }
}

impl<'a> From<&'a crate::types::ValidateReport> for Payload<'a> {
    fn from(report: &'a crate::types::ValidateReport) -> Self {
        Payload::Validate(report)
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

#[doc(hidden)]
impl<'a> From<&'a crate::types::DescribeReport> for Payload<'a> {
    fn from(report: &'a crate::types::DescribeReport) -> Self {
        Payload::Describe(report)
    }
}

#[doc(hidden)]
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

impl<'a> From<&'a RuleCoverageReport> for Payload<'a> {
    fn from(report: &'a RuleCoverageReport) -> Self {
        Payload::RuleCoverage(report)
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

type Formatter<T> = fn(&T) -> String;

fn pick<T: ?Sized>(format: Format, value: &T, formatters: [Formatter<T>; 3]) -> String {
    let [plain, json, rich] = formatters;
    match format {
        Format::Plain => plain(value),
        Format::Json => json(value),
        Format::Rich => rich(value),
    }
}

/// Render a payload in the given format.
///
/// Findings are sorted before rendering. `Json` is pretty-printed with attr keys
/// sorted; `Rich` carries ANSI color, which [`strip_ansi`] removes for a
/// non-terminal. Only the payload is rendered, never a service
/// [`Response`](crate::service::Response) envelope around it.
///
/// ```
/// use latt_core::document::ingest_document;
/// use latt_core::output::{Format, output_result};
/// use latt_core::types::Issue;
///
/// let graph = ingest_document(serde_json::json!({
///     "interface_version": "1.2",
///     "findings": [{"severity": "warning", "code": "PARSE_ERROR", "message": "row",
///                   "provenance": {"file": "r.md", "line": 3}, "node_id": null}],
/// }))?;
/// let issues: Vec<Issue> = graph.adapter_issues().to_vec();
/// let text = output_result(&issues, Format::Json);
/// let parsed: serde_json::Value = serde_json::from_str(&text)?;
/// assert_eq!(parsed["findings"][0]["code"], "PARSE_ERROR");
/// assert!(output_result(&issues, Format::Plain).contains("WARNING PARSE_ERROR"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]
#[must_use]
pub fn output_result<'a>(payload: impl Into<Payload<'a>>, format: Format) -> String {
    match payload.into() {
        Payload::Findings(issues) => {
            // Sorted borrowed, not cloned. `validate` has already sorted what
            // it returns, but the sort stays because a caller may arrive with
            // findings in collection order.
            let mut sorted: Vec<&Issue> = issues.iter().collect();
            sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
            match format {
                Format::Plain => validate::format_plain(&sorted),
                Format::Json => validate::format_json(&sorted),
                Format::Rich => validate::format_rich(&sorted),
            }
        }
        Payload::Validate(report) => pick(
            format,
            report,
            [
                validate::format_report_plain,
                validate::format_report_json,
                validate::format_report_rich,
            ],
        ),
        Payload::Summary(SummaryReport::Configured(report)) => pick(
            format,
            report,
            [
                summary::format_summary_plain,
                summary::format_summary_json,
                summary::format_summary_rich,
            ],
        ),
        Payload::Summary(SummaryReport::Structural(report)) => pick(
            format,
            report,
            [
                summary::format_structural_plain,
                summary::format_structural_json,
                summary::format_structural_rich,
            ],
        ),
        Payload::Describe(report) => pick(
            format,
            report,
            [
                describe::format_describe_plain,
                describe::format_describe_json,
                describe::format_describe_rich,
            ],
        ),
        Payload::Fuse(report) => pick(
            format,
            report,
            [
                fuse::format_fuse_plain,
                fuse::format_fuse_json,
                fuse::format_fuse_rich,
            ],
        ),
        Payload::Trace(report) => pick(
            format,
            report,
            [
                trace::format_trace_plain,
                trace::format_trace_json,
                trace::format_trace_rich,
            ],
        ),
        Payload::At(report) => pick(
            format,
            report,
            [
                query::format_at_plain,
                query::format_at_json,
                query::format_at_rich,
            ],
        ),
        Payload::Reach(report) => pick(
            format,
            report,
            [
                query::format_reach_plain,
                query::format_reach_json,
                query::format_reach_rich,
            ],
        ),
        Payload::Path(report) => pick(
            format,
            report,
            [
                query::format_path_plain,
                query::format_path_json,
                query::format_path_rich,
            ],
        ),
        Payload::Orphans(report) => pick(
            format,
            report,
            [
                query::format_orphans_plain,
                query::format_orphans_json,
                query::format_orphans_rich,
            ],
        ),
        Payload::Counts(report) => pick(
            format,
            report,
            [
                query::format_counts_plain,
                query::format_counts_json,
                query::format_counts_rich,
            ],
        ),
        Payload::RuleCoverage(report) => pick(
            format,
            report,
            [
                query::format_rule_coverage_plain,
                query::format_rule_coverage_json,
                query::format_rule_coverage_rich,
            ],
        ),
        Payload::Coverage(report) => pick(
            format,
            report,
            [
                query::format_coverage_plain,
                query::format_coverage_json,
                query::format_coverage_rich,
            ],
        ),
        Payload::Diff(report) => pick(
            format,
            report,
            [
                query::format_diff_plain,
                query::format_diff_json,
                query::format_diff_rich,
            ],
        ),
    }
}

#[cfg(test)]
mod sanitize_tests {
    use std::borrow::Cow;

    use proptest::prelude::*;

    use super::sanitize_text;

    /// Each escaped character with the text it must become, written out by
    /// hand so the expectation does not come from the function under test.
    const ESCAPES: &[(char, &str)] = &[
        ('\u{0}', r"\u{0}"),
        ('\u{7}', r"\u{7}"),
        ('\u{8}', r"\u{8}"),
        ('\n', r"\n"),
        ('\u{b}', r"\u{b}"),
        ('\r', r"\r"),
        ('\u{1b}', r"\u{1b}"),
        ('\u{1f}', r"\u{1f}"),
        ('\u{7f}', r"\u{7f}"),
        ('\u{80}', r"\u{80}"),
        ('\u{85}', r"\u{85}"),
        ('\u{9f}', r"\u{9f}"),
        ('\u{61c}', r"\u{61c}"),
        ('\u{200e}', r"\u{200e}"),
        ('\u{200f}', r"\u{200f}"),
        ('\u{202a}', r"\u{202a}"),
        ('\u{202e}', r"\u{202e}"),
        ('\u{2028}', r"\u{2028}"),
        ('\u{2029}', r"\u{2029}"),
        ('\u{2066}', r"\u{2066}"),
        ('\u{2069}', r"\u{2069}"),
    ];

    /// The escaped set, from the rule rather than from `ESCAPES`.
    fn escaped(c: char) -> bool {
        matches!(c, '\u{0}'..='\u{8}' | '\u{a}'..='\u{1f}' | '\u{7f}'..='\u{9f}')
            || matches!(
                c,
                '\u{61c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{2028}'
                    | '\u{2029}'
            )
    }

    #[test]
    fn sanitize_text_escapes_controls_and_bidi_and_borrows_clean_text() {
        assert_eq!(
            sanitize_text("a\u{1b}]8;;http://x\u{7}b\nc\rd\u{202e}e\u{85}f\u{7f}\tg"),
            r"a\u{1b}]8;;http://x\u{7}b\nc\rd\u{202e}e\u{85}f\u{7f}".to_owned() + "\tg"
        );
        assert!(matches!(
            sanitize_text("clean ünïcode\tand a \\ backslash"),
            Cow::Borrowed("clean ünïcode\tand a \\ backslash")
        ));
    }

    /// Text with no escaped character: letters, digits, punctuation, a tab, a
    /// backslash and non-ASCII outside the escaped ranges.
    fn clean_part() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 \t\\\\{}\\[\\];'éü€\u{a0}\u{2010}\u{202f}]{0,8}"
    }

    /// Any characters, the escaped ones well represented: `any::<String>()`
    /// draws from `\PC` and would never produce a control.
    fn hostile_text() -> impl Strategy<Value = String> {
        let controls: Vec<char> = ESCAPES.iter().map(|(c, _)| *c).collect();
        prop::collection::vec(
            prop_oneof![any::<char>(), prop::sample::select(controls)],
            0..16,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    proptest! {
        /// Parts joined by one chosen control come back joined by its escape.
        #[test]
        fn a_control_between_clean_parts_becomes_its_escape(
            parts in prop::collection::vec(clean_part(), 1..5),
            pick in 0..ESCAPES.len(),
        ) {
            let (control, escape) = ESCAPES[pick];
            let input = parts.join(&control.to_string());
            prop_assert_eq!(sanitize_text(&input).into_owned(), parts.join(escape));
        }

        /// Clean text comes back borrowed and unchanged.
        #[test]
        fn clean_text_is_borrowed(text in clean_part()) {
            prop_assert!(matches!(sanitize_text(&text), Cow::Borrowed(t) if t == text));
        }

        /// No escaped character survives, and a second pass changes nothing.
        #[test]
        fn the_output_is_free_of_controls_and_a_fixed_point(text in hostile_text()) {
            let once = sanitize_text(&text).into_owned();
            prop_assert!(!once.chars().any(escaped), "{:?}", once);
            prop_assert_eq!(sanitize_text(&once).into_owned(), once);
        }
    }
}
