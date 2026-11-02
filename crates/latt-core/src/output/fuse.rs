use std::collections::BTreeMap;

use serde::Serialize;

use super::sorted::SortedMap;
use super::{PathwaysJson, RESET, color_for, render_json};
use crate::types::{FuseFinding, FuseProvenance, FuseReport};

// Every struct below emits its fields in declaration order; see `render_json`.

#[derive(Serialize)]
struct FuseProvenanceJson<'a> {
    file: &'a str,
    line: u32,
    source: &'a str,
}

impl<'a> From<&'a FuseProvenance> for FuseProvenanceJson<'a> {
    fn from(provenance: &'a FuseProvenance) -> Self {
        Self {
            file: provenance.location.file(),
            line: provenance.location.wire_line(),
            source: &provenance.source,
        }
    }
}

#[derive(Serialize)]
struct FuseNodeJson<'a> {
    attrs: SortedMap<'a>,
    id: &'a str,
    kind: &'a str,
    provenance: FuseProvenanceJson<'a>,
}

#[derive(Serialize)]
struct FuseEdgeJson<'a> {
    attrs: SortedMap<'a>,
    kind: &'a str,
    provenance: FuseProvenanceJson<'a>,
    source: &'a str,
    source_kind: &'a str,
    src: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_source: Option<&'a str>,
    tgt: &'a str,
}

/// `FindingJson` plus the fuse-only keys, spelled out rather than flattened
/// because `locations` and `source` sort between the shared fields.
#[derive(Serialize)]
struct FuseFindingJson<'a> {
    code: &'a str,
    file: &'a str,
    line: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    locations: Vec<FuseProvenanceJson<'a>>,
    message: &'a str,
    node_id: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<super::NoteJson<'a>>,
    severity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sources: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    suppressed: bool,
}

impl<'a> From<&'a FuseFinding> for FuseFindingJson<'a> {
    fn from(finding: &'a FuseFinding) -> Self {
        let issue = &finding.issue;
        Self {
            code: &issue.code,
            file: issue.origin.wire_file(),
            line: issue.origin.wire_line(),
            locations: finding
                .locations
                .iter()
                .map(FuseProvenanceJson::from)
                .collect(),
            message: &issue.message,
            node_id: &issue.node_id,
            note: issue.note.as_ref().map(super::NoteJson::from),
            severity: issue.severity.as_str(),
            source: finding.source.as_deref(),
            sources: finding
                .locations
                .iter()
                .map(|p| p.source.as_str())
                .collect(),
            state: issue.state.as_deref(),
            suppressed: issue.suppressed,
        }
    }
}

#[derive(Serialize)]
struct FuseJson<'a> {
    edges: Vec<FuseEdgeJson<'a>>,
    findings: Vec<FuseFindingJson<'a>>,
    header: &'a BTreeMap<String, String>,
    nodes: Vec<FuseNodeJson<'a>>,
    pathways: PathwaysJson<'a>,
}

pub(super) fn format_fuse_json(report: &FuseReport) -> String {
    render_json(&FuseJson {
        edges: report
            .edges
            .iter()
            .map(|edge| FuseEdgeJson {
                attrs: SortedMap(&edge.attrs),
                kind: &edge.kind,
                provenance: FuseProvenanceJson::from(&edge.provenance),
                source: &edge.provenance.source,
                source_kind: &edge.source_kind,
                src: &edge.src,
                target_kind: edge.target_kind.as_deref(),
                target_source: edge.target_source.as_deref(),
                tgt: &edge.tgt,
            })
            .collect(),
        findings: report.findings.iter().map(FuseFindingJson::from).collect(),
        header: &report.header,
        nodes: report
            .nodes
            .iter()
            .map(|node| FuseNodeJson {
                attrs: SortedMap(&node.attrs),
                id: &node.id,
                kind: &node.kind,
                provenance: FuseProvenanceJson::from(&node.provenance),
            })
            .collect(),
        pathways: PathwaysJson(&report.pathways),
    })
}

pub(super) fn format_fuse_plain(report: &FuseReport) -> String {
    format_fuse_text(report, false)
}
pub(super) fn format_fuse_rich(report: &FuseReport) -> String {
    format_fuse_text(report, true)
}

fn format_fuse_text(report: &FuseReport, rich: bool) -> String {
    let mut lines = vec![format!(
        "Fuse {} {}: {} nodes, {} edges, {} pathways",
        report
            .header
            .get("name")
            .map_or("<unknown>", String::as_str),
        report.header.get("version").map_or("", String::as_str),
        report.nodes.len(),
        report.edges.len(),
        report.pathways.len()
    )];
    for node in &report.nodes {
        lines.push(format!(
            "{} [{}] {}",
            node.id, node.kind, node.provenance.location
        ));
    }
    for edge in &report.edges {
        lines.push(format!(
            "{} --{}--> {}{}",
            edge.src,
            edge.kind,
            edge.tgt,
            if edge.target_kind.is_some() {
                ""
            } else {
                " (unresolved)"
            }
        ));
    }
    for pathway in &report.pathways {
        lines.push(format!(
            "{}: {} (current: {})",
            pathway.name,
            pathway.order.join(" -> "),
            pathway.current
        ));
    }
    let mut shown = 0;
    for finding in &report.findings {
        let issue = &finding.issue;
        if issue.suppressed {
            continue;
        }
        shown += 1;
        let severity = if rich {
            format!(
                "{}{}{}",
                color_for(issue.severity),
                issue.severity.as_upper(),
                RESET
            )
        } else {
            issue.severity.as_upper().into()
        };
        let source = finding
            .source
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                finding
                    .locations
                    .iter()
                    .map(|p| p.source.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            });
        let line = format!(
            "{severity} {} [{}] {} {}",
            issue.code, source, issue.origin, issue.message
        );
        match (&issue.note, rich) {
            (Some(note), true) => {
                lines.push(line);
                lines.push(format!("  {}", super::note_text(note)));
            }
            _ => lines.push(super::with_plain_note(line, issue)),
        }
    }
    if shown == 0 {
        lines.push("No findings.".into());
    }
    lines.join("\n")
}
