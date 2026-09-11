use serde_json::Value;

use super::{FindingJson, PathwaysJson, RESET, color_for, render_json};
use crate::types::FuseReport;

fn fuse_provenance_json(provenance: &crate::types::FuseProvenance) -> Value {
    serde_json::json!({"source": provenance.source, "file": provenance.location.file, "line": provenance.location.line})
}

pub(super) fn format_fuse_json(report: &FuseReport) -> String {
    let nodes: Vec<_> = report
        .nodes
        .iter()
        .map(|node| {
            serde_json::json!({
                "id": node.id, "kind": node.kind, "attrs": node.attrs,
                "provenance": fuse_provenance_json(&node.provenance),
            })
        })
        .collect();
    let edges: Vec<_> = report
        .edges
        .iter()
        .map(|edge| {
            let mut value = serde_json::json!({
                "src": edge.src, "tgt": edge.tgt, "kind": edge.kind, "attrs": edge.attrs,
                "source": edge.provenance.source, "source_kind": edge.source_kind,
                "provenance": fuse_provenance_json(&edge.provenance),
            });
            if let Some(kind) = &edge.target_kind {
                value["target_kind"] = kind.clone().into();
            }
            if let Some(source) = &edge.target_source {
                value["target_source"] = source.clone().into();
            }
            value
        })
        .collect();
    let findings: Vec<_> = report
        .findings
        .iter()
        .map(|finding| {
            let mut value = serde_json::to_value(FindingJson::from(&finding.issue))
                .expect("finding serializes");
            if let Some(source) = &finding.source {
                value["source"] = source.clone().into();
            }
            if !finding.locations.is_empty() {
                value["sources"] = serde_json::json!(
                    finding
                        .locations
                        .iter()
                        .map(|p| &p.source)
                        .collect::<Vec<_>>()
                );
                value["locations"] =
                    Value::Array(finding.locations.iter().map(fuse_provenance_json).collect());
            }
            value
        })
        .collect();
    render_json(&serde_json::json!({
        "header": report.header, "nodes": nodes, "edges": edges,
        "pathways": PathwaysJson(&report.pathways), "findings": findings,
    }))
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
            .map(String::as_str)
            .unwrap_or("<unknown>"),
        report
            .header
            .get("version")
            .map(String::as_str)
            .unwrap_or(""),
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
        lines.push(format!(
            "{severity} {} [{}] {} {}",
            issue.code, source, issue.provenance, issue.message
        ));
    }
    if shown == 0 {
        lines.push("No findings.".into());
    }
    lines.join("\n")
}
