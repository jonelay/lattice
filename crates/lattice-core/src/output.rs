//! Rendering findings in the three output formats.
//!
//! One dispatch point, so a format is defined once for every command. Formatting
//! at a call site is how the three drift apart.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::Serialize as DeriveSerialize;
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::Value;

use crate::types::{
    AtReport, CountsReport, DiffReport, Issue, OrphansReport, PathReport, ReachReport, Severity,
    SummaryReport, TraceEntry, TraceReport,
};

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

/// Pad to `width` on the right, leaving anything longer untouched — Python's
/// `ljust` and `{:<Ns}` both no-op on an oversized field, and absolute paths in
/// provenance regularly overflow the location column.
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

fn format_plain(issues: &[&Issue]) -> String {
    issues
        .iter()
        .map(|i| {
            format!(
                "{} {} {} {}",
                i.severity.as_upper(),
                i.code,
                i.provenance,
                i.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Escape every non-ASCII character as `\uXXXX`, the way Python's `json.dumps`
/// does by default.
///
/// `serde_json` emits raw UTF-8, so a finding quoting non-ASCII source text — a
/// minus sign, an en dash, a Greek letter, all of which this register's
/// requirement text carries — would serialize differently from the reference
/// core. Applied to the rendered document rather than through a custom
/// `Formatter`: non-ASCII can only occur inside JSON string literals, so a pass
/// over the finished text is equivalent and far smaller.
fn escape_non_ascii(json: &str) -> Cow<'_, str> {
    if json.is_ascii() {
        return Cow::Borrowed(json);
    }
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            // Python emits a surrogate pair for anything outside the BMP.
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
        }
    }
}

struct FindingsJson<'a>(&'a [&'a Issue]);

impl Serialize for FindingsJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for issue in self.0 {
            sequence.serialize_element(&FindingJson::from(*issue))?;
        }
        sequence.end()
    }
}

#[derive(DeriveSerialize)]
struct FindingsRoot<'a> {
    findings: FindingsJson<'a>,
}

fn format_json(issues: &[&Issue]) -> String {
    render_json(&FindingsRoot {
        findings: FindingsJson(issues),
    })
}

fn format_rich(issues: &[&Issue]) -> String {
    if issues.is_empty() {
        return "No findings.".to_string();
    }

    let mut lines: Vec<String> = issues
        .iter()
        .map(|i| {
            format!(
                "{}{}{} {} {} {}",
                color_for(i.severity),
                ljust(i.severity.as_upper(), 7),
                RESET,
                ljust(&i.code, 20),
                ljust(&i.provenance.to_string(), 30),
                i.message
            )
        })
        .collect();

    let mut parts = Vec::new();
    for severity in [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Hint,
    ] {
        let count = issues.iter().filter(|i| i.severity == severity).count();
        if count > 0 {
            parts.push(format!("{count} {}(s)", severity.as_str()));
        }
    }
    lines.push(format!("\n{}", parts.join(", ")));
    lines.join("\n")
}

fn format_summary_plain(report: &SummaryReport) -> String {
    report
        .groups
        .iter()
        .map(|(group, counts)| {
            let parts: Vec<String> = report
                .status_keys
                .iter()
                .map(|k| format!("{k}={}", count(counts, k)))
                .collect();
            format!(
                "{group}: {} total={}",
                parts.join(" "),
                count(counts, "total")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct SummaryRowsJson<'a>(&'a SummaryReport);

struct SummaryRowJson<'a> {
    group_key: &'a str,
    group: &'a str,
    counts: &'a BTreeMap<String, i64>,
}

impl Serialize for SummaryRowJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let has_collision = self.counts.contains_key(self.group_key);
        let mut mapping =
            serializer.serialize_map(Some(self.counts.len() + usize::from(!has_collision)))?;
        let mut inserted_group = has_collision;
        for (key, value) in self.counts {
            if !inserted_group && self.group_key < key.as_str() {
                mapping.serialize_entry(self.group_key, self.group)?;
                inserted_group = true;
            }
            mapping.serialize_entry(key, value)?;
        }
        if !inserted_group {
            mapping.serialize_entry(self.group_key, self.group)?;
        }
        mapping.end()
    }
}

impl Serialize for SummaryRowsJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.groups.len()))?;
        for (group, counts) in &self.0.groups {
            sequence.serialize_element(&SummaryRowJson {
                group_key: &self.0.group_key,
                group,
                counts,
            })?;
        }
        sequence.end()
    }
}

#[derive(DeriveSerialize)]
struct SummaryJson<'a> {
    files: SummaryRowsJson<'a>,
    totals: &'a BTreeMap<String, i64>,
}

fn format_summary_json(report: &SummaryReport) -> String {
    let totals = report.totals();
    render_json(&SummaryJson {
        files: SummaryRowsJson(report),
        totals: &totals,
    })
}

fn format_summary_rich(report: &SummaryReport) -> String {
    let mut labels: Vec<String> = report
        .status_keys
        .iter()
        .map(|k| truncate(&capitalize(k), 5))
        .collect();
    labels.push("Total".to_string());

    let header = format!(
        "{} {}",
        ljust(&capitalize(&report.group_key), 30),
        labels
            .iter()
            .map(|l| rjust(l, 5))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let rule = "-".repeat(header.chars().count());

    let mut lines = vec![header, rule.clone()];
    for (group, counts) in &report.groups {
        lines.push(format!(
            "{} {} {}",
            ljust(group, 30),
            columns(&report.status_keys, counts),
            rjust(&count(counts, "total").to_string(), 5)
        ));
    }

    let totals = report.totals();
    lines.push(rule);
    lines.push(format!(
        "{} {} {}",
        ljust("TOTAL", 30),
        columns(&report.status_keys, &totals),
        rjust(&count(&totals, "total").to_string(), 5)
    ));
    lines.join("\n")
}

/// One count, or zero — a group carries a column for every status key, but
/// reading through a map keeps the formatters total.
fn count(counts: &std::collections::BTreeMap<String, i64>, key: &str) -> i64 {
    counts.get(key).copied().unwrap_or(0)
}

fn columns(status_keys: &[String], counts: &std::collections::BTreeMap<String, i64>) -> String {
    status_keys
        .iter()
        .map(|k| rjust(&count(counts, k).to_string(), 5))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A trace row's key-attr cell: the profile's `summary_attr` for this kind,
/// stringified and cut to the column width. Empty when the kind declares none.
fn key_value(entry: &TraceEntry) -> String {
    let Some(attr) = &entry.summary_attr else {
        return String::new();
    };
    let value = entry
        .attrs
        .get(attr.as_str())
        .map_or(String::new(), scalar_text);
    truncate(&value, 30)
}

/// A value for a trace row's cell: a string bare, anything else in its JSON
/// form (`true`, `false`, `null`, a number's JSON rendering).
fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn finding_line(issue: &Issue) -> String {
    format!(
        "  {} {} {} {}",
        issue.severity.as_upper(),
        issue.code,
        issue.provenance,
        issue.message
    )
}

/// A finding under its entry in `rich`, where the severity is coloured and padded.
fn rich_finding_line(issue: &Issue) -> String {
    format!(
        "  {}{}{} {} {} {}",
        color_for(issue.severity),
        ljust(issue.severity.as_upper(), 7),
        RESET,
        issue.code,
        issue.provenance,
        issue.message
    )
}

fn format_trace_plain(report: &TraceReport) -> String {
    let mut lines: Vec<String> = Vec::new();
    for entry in &report.entries {
        lines.push(format!(
            "{} {} {} edges:{} findings:{}",
            ljust(&entry.id, 20),
            ljust(&entry.kind, 10),
            ljust(&key_value(entry), 30),
            entry.edge_count(),
            entry.findings.len()
        ));
        lines.extend(entry.findings.iter().map(finding_line));
    }
    if !report.unattachable_findings.is_empty() {
        lines.push(String::new());
        lines.push("Unattachable findings:".to_string());
        lines.extend(report.unattachable_findings.iter().map(finding_line));
    }
    lines.join("\n")
}

#[derive(DeriveSerialize)]
struct TraceFindingJson<'a> {
    code: &'a str,
    file: &'a str,
    line: i64,
    message: &'a str,
    severity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a str>,
}

impl<'a> From<&'a Issue> for TraceFindingJson<'a> {
    fn from(issue: &'a Issue) -> Self {
        Self {
            code: &issue.code,
            file: &issue.provenance.file,
            line: issue.provenance.line,
            message: &issue.message,
            severity: issue.severity.as_str(),
            state: issue.state.as_deref(),
        }
    }
}

struct TraceFindingsJson<'a>(&'a [Issue]);

impl Serialize for TraceFindingsJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for issue in self.0 {
            sequence.serialize_element(&TraceFindingJson::from(issue))?;
        }
        sequence.end()
    }
}

#[derive(DeriveSerialize)]
struct TraceEntryJson<'a> {
    attrs: &'a BTreeMap<String, Value>,
    edges: &'a BTreeMap<String, Vec<String>>,
    findings: TraceFindingsJson<'a>,
    id: &'a str,
    kind: &'a str,
    provenance: ProvenanceJson<'a>,
}

struct TraceEntriesJson<'a>(&'a [TraceEntry]);

impl Serialize for TraceEntriesJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            sequence.serialize_element(&TraceEntryJson {
                attrs: &entry.attrs,
                edges: &entry.edges,
                findings: TraceFindingsJson(&entry.findings),
                id: &entry.id,
                kind: &entry.kind,
                provenance: ProvenanceJson::from(&entry.provenance),
            })?;
        }
        sequence.end()
    }
}

#[derive(DeriveSerialize)]
struct TraceJson<'a> {
    entries: TraceEntriesJson<'a>,
    header: &'a BTreeMap<String, String>,
    unattachable_findings: TraceFindingsJson<'a>,
}

fn format_trace_json(report: &TraceReport) -> String {
    render_json(&TraceJson {
        entries: TraceEntriesJson(&report.entries),
        header: &report.header,
        unattachable_findings: TraceFindingsJson(&report.unattachable_findings),
    })
}

fn format_trace_rich(report: &TraceReport) -> String {
    if report.entries.is_empty() && report.unattachable_findings.is_empty() {
        return "No trace entries.".to_string();
    }

    let header = format!(
        "{} {} {} {} {}",
        ljust("ID", 20),
        ljust("Kind", 10),
        ljust("Key Attr", 30),
        rjust("Edges", 5),
        rjust("Finds", 5)
    );
    let mut lines = vec!["-".repeat(header.chars().count())];
    lines.insert(0, header);

    for entry in &report.entries {
        lines.push(format!(
            "{} {} {} {} {}",
            ljust(&entry.id, 20),
            ljust(&entry.kind, 10),
            ljust(&key_value(entry), 30),
            rjust(&entry.edge_count().to_string(), 5),
            rjust(&entry.findings.len().to_string(), 5)
        ));
        lines.extend(entry.findings.iter().map(rich_finding_line));
    }

    if !report.unattachable_findings.is_empty() {
        lines.push(String::new());
        lines.push("Unattachable findings:".to_string());
        lines.extend(report.unattachable_findings.iter().map(rich_finding_line));
    }

    let all_findings = || {
        report
            .entries
            .iter()
            .flat_map(|e| &e.findings)
            .chain(&report.unattachable_findings)
    };
    let mut parts = Vec::new();
    for severity in [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Hint,
    ] {
        let count = all_findings().filter(|f| f.severity == severity).count();
        if count > 0 {
            parts.push(format!("{count} {}(s)", severity.as_str()));
        }
    }
    let entries = report.entries.len();
    lines.push(if parts.is_empty() {
        format!("\n{entries} entries, no findings")
    } else {
        format!("\n{entries} entries, {}", parts.join(", "))
    });

    lines.join("\n")
}

fn format_at_plain(report: &AtReport) -> String {
    if report.entries.is_empty() && report.findings.is_empty() {
        return format!("No entries or findings at {}.", report.path);
    }
    let mut lines: Vec<String> = Vec::new();
    for entry in &report.entries {
        lines.push(format!(
            "{} {} {} edges:{} findings:{}",
            ljust(&entry.id, 20),
            ljust(&entry.kind, 10),
            ljust(&key_value(entry), 30),
            entry.edge_count(),
            entry.findings.len()
        ));
        lines.extend(entry.findings.iter().map(finding_line));
    }
    if !report.findings.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("Findings at this path:".to_string());
        lines.extend(report.findings.iter().map(finding_line));
    }
    lines.join("\n")
}

fn format_at_json(report: &AtReport) -> String {
    #[derive(DeriveSerialize)]
    struct AtJson<'a> {
        entries: TraceEntriesJson<'a>,
        findings: TraceFindingsJson<'a>,
        path: &'a str,
    }

    render_json(&AtJson {
        entries: TraceEntriesJson(&report.entries),
        findings: TraceFindingsJson(&report.findings),
        path: &report.path,
    })
}

fn format_at_rich(report: &AtReport) -> String {
    if report.entries.is_empty() && report.findings.is_empty() {
        return format!("No entries or findings at {}.", report.path);
    }
    let mut lines = Vec::new();
    if !report.entries.is_empty() {
        let header = format!(
            "{} {} {} {} {}",
            ljust("ID", 20),
            ljust("Kind", 10),
            ljust("Key Attr", 30),
            rjust("Edges", 5),
            rjust("Finds", 5)
        );
        lines.push(header.clone());
        lines.push("-".repeat(header.chars().count()));
        for entry in &report.entries {
            lines.push(format!(
                "{} {} {} {} {}",
                ljust(&entry.id, 20),
                ljust(&entry.kind, 10),
                ljust(&key_value(entry), 30),
                rjust(&entry.edge_count().to_string(), 5),
                rjust(&entry.findings.len().to_string(), 5)
            ));
            lines.extend(entry.findings.iter().map(rich_finding_line));
        }
    }
    if !report.findings.is_empty() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("Findings at this path:".to_string());
        lines.extend(report.findings.iter().map(rich_finding_line));
    }
    let findings = report
        .entries
        .iter()
        .map(|e| e.findings.len())
        .sum::<usize>()
        + report.findings.len();
    lines.push(format!(
        "\n{} entries, {} finding(s) at {}",
        report.entries.len(),
        findings,
        report.path
    ));
    lines.join("\n")
}

fn format_reach_plain(report: &ReachReport) -> String {
    if report.nodes.is_empty() {
        return "No nodes.".to_string();
    }
    report
        .nodes
        .iter()
        .map(|n| format!("{} {}", n.id, n.kind))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(DeriveSerialize)]
struct NodeRefJson<'a> {
    id: &'a str,
    kind: &'a str,
}

struct NodeRefsJson<'a>(&'a [crate::types::NodeRef]);

impl Serialize for NodeRefsJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for node in self.0 {
            sequence.serialize_element(&NodeRefJson {
                id: &node.id,
                kind: &node.kind,
            })?;
        }
        sequence.end()
    }
}

fn format_reach_json(report: &ReachReport) -> String {
    #[derive(DeriveSerialize)]
    struct ReachJson<'a> {
        direction: &'a str,
        edge_kinds: &'a [String],
        nodes: NodeRefsJson<'a>,
        origin: &'a str,
    }

    render_json(&ReachJson {
        direction: &report.direction,
        edge_kinds: &report.edge_kinds,
        nodes: NodeRefsJson(&report.nodes),
        origin: &report.origin,
    })
}

fn format_reach_rich(report: &ReachReport) -> String {
    let restriction = if report.edge_kinds.is_empty() {
        "all edge kinds".to_string()
    } else {
        report.edge_kinds.join(", ")
    };
    let mut lines = vec![format!(
        "{} {} ({restriction})",
        report.direction, report.origin
    )];
    if report.nodes.is_empty() {
        lines.push("No nodes.".to_string());
    } else {
        for n in &report.nodes {
            lines.push(format!("{} {}", ljust(&n.id, 20), n.kind));
        }
        lines.push(format!("\n{} node(s)", report.nodes.len()));
    }
    lines.join("\n")
}

/// The plain path chain: `A -[kind]-> B -[kind]-> C`.
fn path_chain(nodes: &[String], edges: &[String]) -> String {
    let Some((first, remaining)) = nodes.split_first() else {
        return String::new();
    };
    let mut out = first.clone();
    for (kind, node) in edges.iter().zip(remaining) {
        out.push_str(&format!(" -[{kind}]-> {node}"));
    }
    out
}

fn format_path_plain(report: &PathReport) -> String {
    if let Some((nodes, edges)) = report.found_path() {
        path_chain(nodes, edges)
    } else {
        format!("No path from {} to {}.", report.src(), report.tgt())
    }
}

fn format_path_json(report: &PathReport) -> String {
    let (found, nodes, edges) = match report.found_path() {
        Some((nodes, edges)) => (true, nodes, edges),
        None => (false, &[][..], &[][..]),
    };
    #[derive(DeriveSerialize)]
    struct PathJson<'a> {
        edges: &'a [String],
        found: bool,
        nodes: &'a [String],
        src: &'a str,
        tgt: &'a str,
    }

    render_json(&PathJson {
        edges,
        found,
        nodes,
        src: report.src(),
        tgt: report.tgt(),
    })
}

fn format_path_rich(report: &PathReport) -> String {
    if let Some((nodes, edges)) = report.found_path() {
        format!("{}\n\n{} edge(s)", path_chain(nodes, edges), edges.len())
    } else {
        format!("No path from {} to {}.", report.src(), report.tgt())
    }
}

fn format_orphans_plain(report: &OrphansReport) -> String {
    if report.orphans.is_empty() {
        return "No orphans.".to_string();
    }
    report
        .orphans
        .iter()
        .map(|o| format!("{} {} {}", o.id, o.kind, o.provenance))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_orphans_json(report: &OrphansReport) -> String {
    #[derive(DeriveSerialize)]
    struct OrphanJson<'a> {
        file: &'a str,
        id: &'a str,
        kind: &'a str,
        line: i64,
    }

    struct OrphansJson<'a>(&'a [crate::types::OrphanEntry]);

    impl Serialize for OrphansJson<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
            for orphan in self.0 {
                sequence.serialize_element(&OrphanJson {
                    file: &orphan.provenance.file,
                    id: &orphan.id,
                    kind: &orphan.kind,
                    line: orphan.provenance.line,
                })?;
            }
            sequence.end()
        }
    }

    #[derive(DeriveSerialize)]
    struct OrphansRoot<'a> {
        kind_filter: &'a Option<String>,
        orphans: OrphansJson<'a>,
    }

    render_json(&OrphansRoot {
        kind_filter: &report.kind_filter,
        orphans: OrphansJson(&report.orphans),
    })
}

fn format_orphans_rich(report: &OrphansReport) -> String {
    if report.orphans.is_empty() {
        return "No orphans.".to_string();
    }
    let mut lines: Vec<String> = report
        .orphans
        .iter()
        .map(|o| {
            format!(
                "{} {} {}",
                ljust(&o.id, 20),
                ljust(&o.kind, 10),
                o.provenance
            )
        })
        .collect();
    lines.push(format!("\n{} orphan(s)", report.orphans.len()));
    lines.join("\n")
}

fn format_counts_plain(report: &CountsReport) -> String {
    let mut lines = Vec::new();
    for (kind, n) in &report.nodes {
        lines.push(format!("node {kind} {n}"));
    }
    for (kind, n) in &report.edges {
        lines.push(format!("edge {kind} {n}"));
    }
    lines.join("\n")
}

fn format_counts_json(report: &CountsReport) -> String {
    #[derive(DeriveSerialize)]
    struct CountsJson<'a> {
        edges: &'a BTreeMap<String, i64>,
        nodes: &'a BTreeMap<String, i64>,
    }

    render_json(&CountsJson {
        edges: &report.edges,
        nodes: &report.nodes,
    })
}

fn format_counts_rich(report: &CountsReport) -> String {
    let mut lines = vec!["Nodes".to_string()];
    for (kind, n) in &report.nodes {
        lines.push(format!(
            "  {} {}",
            ljust(kind, 20),
            rjust(&n.to_string(), 5)
        ));
    }
    lines.push("Edges".to_string());
    for (kind, n) in &report.edges {
        lines.push(format!(
            "  {} {}",
            ljust(kind, 20),
            rjust(&n.to_string(), 5)
        ));
    }
    lines.join("\n")
}

/// The diff's shared line list: one line per difference, in a fixed section
/// order (nodes added/removed/changed, edges added/removed, axes).
fn diff_lines(report: &DiffReport) -> Vec<String> {
    let mut lines = Vec::new();
    for n in &report.nodes_added {
        lines.push(format!("node added {} {}", n.id, n.kind));
    }
    for n in &report.nodes_removed {
        lines.push(format!("node removed {} {}", n.id, n.kind));
    }
    for n in &report.nodes_changed {
        lines.push(format!("node changed {} {}", n.id, n.kind));
    }
    for e in &report.edges_added {
        lines.push(format!("edge added {} {} {}", e.src, e.kind, e.tgt));
    }
    for e in &report.edges_removed {
        lines.push(format!("edge removed {} {} {}", e.src, e.kind, e.tgt));
    }
    for name in &report.axes_changed {
        lines.push(format!("axis changed {name}"));
    }
    lines
}

fn format_diff_plain(report: &DiffReport) -> String {
    if report.is_empty() {
        return "No differences.".to_string();
    }
    diff_lines(report).join("\n")
}

#[derive(DeriveSerialize)]
struct EdgeRefJson<'a> {
    kind: &'a str,
    src: &'a str,
    tgt: &'a str,
}

struct EdgeRefsJson<'a>(&'a [crate::types::EdgeRef]);

impl Serialize for EdgeRefsJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for edge in self.0 {
            sequence.serialize_element(&EdgeRefJson {
                kind: &edge.kind,
                src: &edge.src,
                tgt: &edge.tgt,
            })?;
        }
        sequence.end()
    }
}

fn format_diff_json(report: &DiffReport) -> String {
    #[derive(DeriveSerialize)]
    struct DiffJson<'a> {
        axes_changed: &'a [String],
        edges_added: EdgeRefsJson<'a>,
        edges_removed: EdgeRefsJson<'a>,
        nodes_added: NodeRefsJson<'a>,
        nodes_changed: NodeRefsJson<'a>,
        nodes_removed: NodeRefsJson<'a>,
        rev_a: &'a str,
        rev_b: &'a str,
    }

    render_json(&DiffJson {
        axes_changed: &report.axes_changed,
        edges_added: EdgeRefsJson(&report.edges_added),
        edges_removed: EdgeRefsJson(&report.edges_removed),
        nodes_added: NodeRefsJson(&report.nodes_added),
        nodes_changed: NodeRefsJson(&report.nodes_changed),
        nodes_removed: NodeRefsJson(&report.nodes_removed),
        rev_a: &report.rev_a,
        rev_b: &report.rev_b,
    })
}

fn format_diff_rich(report: &DiffReport) -> String {
    let header = format!("{} -> {}", report.rev_a, report.rev_b);
    if report.is_empty() {
        return format!("{header}\nNo differences.");
    }
    let lines = diff_lines(report);
    let count = lines.len();
    format!("{header}\n{}\n\n{count} difference(s)", lines.join("\n"))
}

/// Serialize a payload the way `json.dumps(indent=2, sort_keys=True)` would.
///
/// Serializable views declare fields in sorted order, and nested maps are
/// `BTreeMap`s, matching `sort_keys` at every level.
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
/// removes escapes when its stream is not a terminal. The rendered text is
/// therefore not the whole contract — the baselines were captured to files and
/// carry no colour, so a port that reproduced only `output.py` would differ on
/// every `rich` line.
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
        // click's pattern is \033\[[;?0-9]*[a-zA-Z]: parameters, then one letter.
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
    At(&'a AtReport),
    Reach(&'a ReachReport),
    Path(&'a PathReport),
    Orphans(&'a OrphansReport),
    Counts(&'a CountsReport),
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

impl<'a> From<&'a DiffReport> for Payload<'a> {
    fn from(report: &'a DiffReport) -> Self {
        Payload::Diff(report)
    }
}

/// Render a payload in the named format.
///
/// The single rendering path, so a format is defined once for every command.
/// Findings are sorted first, making output deterministic whatever order
/// collection produced; the other payloads carry their own total order already.
pub fn output_result<'a>(
    payload: impl Into<Payload<'a>>,
    format: &str,
) -> Result<String, UnknownFormat> {
    match payload.into() {
        Payload::Findings(issues) => {
            // Sorted borrowed, not cloned. `validate` has already sorted what it
            // returns, but the sort stays because a caller may arrive with
            // findings in collection order.
            let mut sorted: Vec<&Issue> = issues.iter().collect();
            sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
            match format {
                "plain" => Ok(format_plain(&sorted)),
                "json" => Ok(format_json(&sorted)),
                "rich" => Ok(format_rich(&sorted)),
                other => Err(UnknownFormat(other.to_string())),
            }
        }
        Payload::Summary(report) => match format {
            "plain" => Ok(format_summary_plain(report)),
            "json" => Ok(format_summary_json(report)),
            "rich" => Ok(format_summary_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Trace(report) => match format {
            "plain" => Ok(format_trace_plain(report)),
            "json" => Ok(format_trace_json(report)),
            "rich" => Ok(format_trace_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::At(report) => match format {
            "plain" => Ok(format_at_plain(report)),
            "json" => Ok(format_at_json(report)),
            "rich" => Ok(format_at_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Reach(report) => match format {
            "plain" => Ok(format_reach_plain(report)),
            "json" => Ok(format_reach_json(report)),
            "rich" => Ok(format_reach_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Path(report) => match format {
            "plain" => Ok(format_path_plain(report)),
            "json" => Ok(format_path_json(report)),
            "rich" => Ok(format_path_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Orphans(report) => match format {
            "plain" => Ok(format_orphans_plain(report)),
            "json" => Ok(format_orphans_json(report)),
            "rich" => Ok(format_orphans_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Counts(report) => match format {
            "plain" => Ok(format_counts_plain(report)),
            "json" => Ok(format_counts_json(report)),
            "rich" => Ok(format_counts_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
        Payload::Diff(report) => match format {
            "plain" => Ok(format_diff_plain(report)),
            "json" => Ok(format_diff_json(report)),
            "rich" => Ok(format_diff_rich(report)),
            other => Err(UnknownFormat(other.to_string())),
        },
    }
}
