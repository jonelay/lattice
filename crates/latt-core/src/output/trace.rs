use std::collections::BTreeMap;

use serde::ser::{Serialize, SerializeSeq, Serializer};

use super::sorted::{SortedMap, scalar_text};
use super::{
    PathwaysJson, ProvenanceJson, RESET, color_for, ljust, render_json, rjust, truncate, visible,
};
use crate::types::{Issue, Severity, TraceEdge, TraceEntry, TraceReport};

/// The profile's `summary_attr` for this entry's kind, with its whole value;
/// `None` when the kind declares none or the node leaves it out.
fn summary_field(entry: &TraceEntry) -> Option<(&str, String)> {
    let attr = entry.summary_attr.as_deref()?;
    let value = entry.attrs.get(attr).map(scalar_text)?;
    Some((attr, value))
}

/// A trace row's key-attr cell in `rich`: the summary value cut to the column.
pub(super) fn key_value(entry: &TraceEntry) -> String {
    summary_field(entry).map_or(String::new(), |(_, value)| truncate(&value, 30))
}

/// An entry's `plain` line: every field as `name:value`, so it reads in any
/// order. The summary attr is free text, so it goes last and whole, with each
/// whitespace run folded to one space so an entry never spans lines.
pub(super) fn plain_entry_line(entry: &TraceEntry) -> String {
    let mut line = format!(
        "id:{} kind:{} edges:{} findings:{}",
        entry.id,
        entry.kind,
        entry.edge_count(),
        visible(&entry.findings).count()
    );
    if let Some((attr, value)) = summary_field(entry) {
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        line.push_str(&format!(" {attr}:{value}"));
    }
    line
}

pub(super) fn finding_line(issue: &Issue) -> String {
    let line = format!(
        "  {} {} {} {}",
        issue.severity.as_upper(),
        issue.code,
        issue.origin,
        issue.message
    );
    super::with_plain_note(line, issue)
}

/// A finding under its entry in `rich`, where the severity is coloured and padded.
pub(super) fn rich_finding_line(issue: &Issue) -> String {
    let line = format!(
        "  {}{}{} {} {} {}",
        color_for(issue.severity),
        ljust(issue.severity.as_upper(), 7),
        RESET,
        issue.code,
        issue.origin,
        issue.message
    );
    match &issue.note {
        Some(note) => format!("{line}\n    {}", super::note_text(note)),
        None => line,
    }
}

#[derive(serde::Serialize)]
pub(super) struct TraceFindingJson<'a> {
    pub code: &'a str,
    pub file: &'a str,
    pub line: u32,
    pub message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<super::NoteJson<'a>>,
    pub severity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub suppressed: bool,
}

impl<'a> From<&'a Issue> for TraceFindingJson<'a> {
    fn from(issue: &'a Issue) -> Self {
        Self {
            code: &issue.code,
            file: issue.origin.wire_file(),
            line: issue.origin.wire_line(),
            message: &issue.message,
            note: issue.note.as_ref().map(super::NoteJson::from),
            severity: issue.severity.as_str(),
            state: issue.state.as_deref(),
            suppressed: issue.suppressed,
        }
    }
}

pub(super) struct TraceFindingsJson<'a>(pub &'a [Issue]);

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

#[derive(serde::Serialize)]
struct TraceEdgeJson<'a> {
    tgt: &'a str,
    kind: &'a str,
    attrs: SortedMap<'a>,
    provenance: ProvenanceJson<'a>,
}

impl<'a> From<&'a TraceEdge> for TraceEdgeJson<'a> {
    fn from(edge: &'a TraceEdge) -> Self {
        Self {
            tgt: &edge.tgt,
            kind: &edge.kind,
            attrs: SortedMap(&edge.attrs),
            provenance: ProvenanceJson::from(&edge.provenance),
        }
    }
}

struct TraceEdgesJson<'a>(&'a [TraceEdge]);

impl Serialize for TraceEdgesJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for edge in self.0 {
            sequence.serialize_element(&TraceEdgeJson::from(edge))?;
        }
        sequence.end()
    }
}

#[derive(serde::Serialize)]
struct TraceEntryJson<'a> {
    attrs: SortedMap<'a>,
    edges: TraceEdgesJson<'a>,
    findings: TraceFindingsJson<'a>,
    id: &'a str,
    kind: &'a str,
    provenance: ProvenanceJson<'a>,
}

pub(super) struct TraceEntriesJson<'a>(pub &'a [TraceEntry]);

impl Serialize for TraceEntriesJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            sequence.serialize_element(&TraceEntryJson {
                attrs: SortedMap(&entry.attrs),
                edges: TraceEdgesJson(&entry.edges),
                findings: TraceFindingsJson(&entry.findings),
                id: &entry.id,
                kind: &entry.kind,
                provenance: ProvenanceJson::from(&entry.provenance),
            })?;
        }
        sequence.end()
    }
}

#[derive(serde::Serialize)]
struct TraceJson<'a> {
    entries: TraceEntriesJson<'a>,
    header: &'a BTreeMap<String, String>,
    pathways: PathwaysJson<'a>,
    unattachable_findings: TraceFindingsJson<'a>,
}

pub(super) fn format_trace_plain(report: &TraceReport) -> String {
    let mut lines: Vec<String> = Vec::new();
    for entry in &report.entries {
        lines.push(plain_entry_line(entry));
        lines.extend(visible(&entry.findings).map(finding_line));
    }
    if visible(&report.unattachable_findings).next().is_some() {
        lines.push(String::new());
        lines.push("Unattachable findings:".to_string());
        lines.extend(visible(&report.unattachable_findings).map(finding_line));
    }
    lines.join("\n")
}

pub(super) fn format_trace_json(report: &TraceReport) -> String {
    render_json(&TraceJson {
        entries: TraceEntriesJson(&report.entries),
        header: &report.header,
        pathways: PathwaysJson(&report.pathways),
        unattachable_findings: TraceFindingsJson(&report.unattachable_findings),
    })
}

pub(super) fn format_trace_rich(report: &TraceReport) -> String {
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
            rjust(&visible(&entry.findings).count().to_string(), 5)
        ));
        lines.extend(visible(&entry.findings).map(rich_finding_line));
    }

    if visible(&report.unattachable_findings).next().is_some() {
        lines.push(String::new());
        lines.push("Unattachable findings:".to_string());
        lines.extend(visible(&report.unattachable_findings).map(rich_finding_line));
    }

    let all_findings = || {
        report
            .entries
            .iter()
            .flat_map(|e| visible(&e.findings))
            .chain(visible(&report.unattachable_findings))
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
