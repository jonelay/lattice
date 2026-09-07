//! Rendering findings in the three output formats.
//!
//! One dispatch point, so a format is defined once for every command. Formatting
//! at a call site is how the three drift apart.

use serde_json::{Map, Value, json};

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
fn escape_non_ascii(json: &str) -> String {
    if json.is_ascii() {
        return json.to_string();
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
    out
}

fn format_json(issues: &[&Issue]) -> String {
    let findings: Vec<Value> = issues
        .iter()
        .map(|i| {
            let mut entry = Map::new();
            entry.insert("code".into(), json!(i.code));
            entry.insert("file".into(), json!(i.provenance.file));
            entry.insert("line".into(), json!(i.provenance.line));
            entry.insert("message".into(), json!(i.message));
            entry.insert("node_id".into(), json!(i.node_id));
            entry.insert("severity".into(), json!(i.severity.as_str()));
            if let Some(state) = &i.state {
                entry.insert("state".into(), json!(state));
            }
            Value::Object(entry)
        })
        .collect();
    let mut root = Map::new();
    root.insert("findings".into(), Value::Array(findings));
    render_json(&Value::Object(root))
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

fn format_summary_json(report: &SummaryReport) -> String {
    let files: Vec<Value> = report
        .groups
        .iter()
        .map(|(group, counts)| {
            let mut row = Map::new();
            row.insert(report.group_key.clone(), json!(group));
            // Python spreads the counts over the group key, so a status named
            // like the group column wins the collision.
            for (key, value) in counts {
                row.insert(key.clone(), json!(value));
            }
            Value::Object(row)
        })
        .collect();
    let mut root = Map::new();
    root.insert("files".into(), Value::Array(files));
    root.insert("totals".into(), json!(report.totals()));
    render_json(&Value::Object(root))
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

fn trace_entry_json(e: &TraceEntry) -> Value {
    let mut entry = Map::new();
    entry.insert("attrs".into(), json!(e.attrs));
    entry.insert("edges".into(), json!(e.edges));
    entry.insert(
        "findings".into(),
        Value::Array(e.findings.iter().map(trace_finding_json).collect()),
    );
    entry.insert("id".into(), json!(e.id));
    entry.insert("kind".into(), json!(e.kind));
    entry.insert(
        "provenance".into(),
        json!({"file": e.provenance.file, "line": e.provenance.line}),
    );
    Value::Object(entry)
}

fn format_trace_json(report: &TraceReport) -> String {
    let entries: Vec<Value> = report.entries.iter().map(trace_entry_json).collect();

    let mut root = Map::new();
    root.insert("header".into(), json!(report.header));
    root.insert("entries".into(), Value::Array(entries));
    root.insert(
        "unattachable_findings".into(),
        Value::Array(
            report
                .unattachable_findings
                .iter()
                .map(trace_finding_json)
                .collect(),
        ),
    );
    render_json(&Value::Object(root))
}

/// A finding inside a trace report, which unlike a `validate` finding carries no
/// `node_id` — the entry it sits under already answers that.
fn trace_finding_json(issue: &Issue) -> Value {
    let mut entry = Map::new();
    entry.insert("code".into(), json!(issue.code));
    entry.insert("file".into(), json!(issue.provenance.file));
    entry.insert("line".into(), json!(issue.provenance.line));
    entry.insert("message".into(), json!(issue.message));
    entry.insert("severity".into(), json!(issue.severity.as_str()));
    if let Some(state) = &issue.state {
        entry.insert("state".into(), json!(state));
    }
    Value::Object(entry)
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
    let mut root = Map::new();
    root.insert(
        "entries".into(),
        Value::Array(report.entries.iter().map(trace_entry_json).collect()),
    );
    root.insert(
        "findings".into(),
        Value::Array(report.findings.iter().map(trace_finding_json).collect()),
    );
    root.insert("path".into(), json!(report.path));
    render_json(&Value::Object(root))
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

fn format_reach_json(report: &ReachReport) -> String {
    let nodes: Vec<Value> = report
        .nodes
        .iter()
        .map(|n| json!({"id": n.id, "kind": n.kind}))
        .collect();
    render_json(&json!({
        "direction": report.direction,
        "edge_kinds": report.edge_kinds,
        "nodes": nodes,
        "origin": report.origin,
    }))
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
fn path_chain(report: &PathReport) -> String {
    let mut out = report.nodes[0].clone();
    for (kind, node) in report.edges.iter().zip(&report.nodes[1..]) {
        out.push_str(&format!(" -[{kind}]-> {node}"));
    }
    out
}

fn format_path_plain(report: &PathReport) -> String {
    if report.found {
        path_chain(report)
    } else {
        format!("No path from {} to {}.", report.src, report.tgt)
    }
}

fn format_path_json(report: &PathReport) -> String {
    render_json(&json!({
        "edges": report.edges,
        "found": report.found,
        "nodes": report.nodes,
        "src": report.src,
        "tgt": report.tgt,
    }))
}

fn format_path_rich(report: &PathReport) -> String {
    if report.found {
        format!("{}\n\n{} edge(s)", path_chain(report), report.edges.len())
    } else {
        format!("No path from {} to {}.", report.src, report.tgt)
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
    let orphans: Vec<Value> = report
        .orphans
        .iter()
        .map(|o| {
            json!({
                "file": o.provenance.file,
                "id": o.id,
                "kind": o.kind,
                "line": o.provenance.line,
            })
        })
        .collect();
    render_json(&json!({
        "kind_filter": report.kind_filter,
        "orphans": orphans,
    }))
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
    render_json(&json!({
        "edges": report.edges,
        "nodes": report.nodes,
    }))
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

fn format_diff_json(report: &DiffReport) -> String {
    let nodes = |list: &[crate::types::NodeRef]| -> Vec<Value> {
        list.iter()
            .map(|n| json!({"id": n.id, "kind": n.kind}))
            .collect()
    };
    let edges = |list: &[crate::types::EdgeRef]| -> Vec<Value> {
        list.iter()
            .map(|e| json!({"kind": e.kind, "src": e.src, "tgt": e.tgt}))
            .collect()
    };
    render_json(&json!({
        "axes_changed": report.axes_changed,
        "edges_added": edges(&report.edges_added),
        "edges_removed": edges(&report.edges_removed),
        "nodes_added": nodes(&report.nodes_added),
        "nodes_changed": nodes(&report.nodes_changed),
        "nodes_removed": nodes(&report.nodes_removed),
        "rev_a": report.rev_a,
        "rev_b": report.rev_b,
    }))
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
/// Sorted keys come from `serde_json::Map` being a `BTreeMap` here, which sorts
/// at every level as `sort_keys` does — enabling its `preserve_order` feature
/// would silently break that.
fn render_json(value: &Value) -> String {
    let rendered = serde_json::to_string_pretty(value).expect("payloads are serializable");
    escape_non_ascii(&rendered)
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
