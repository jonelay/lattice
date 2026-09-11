use std::collections::BTreeMap;

use serde::ser::{Serialize, SerializeSeq, Serializer};

use super::trace::{
    TraceEntriesJson, TraceFindingsJson, finding_line, key_value, rich_finding_line,
};
use super::{ljust, render_json, rjust, visible};
use crate::types::{
    AtReport, CountsReport, CoverageReport, DiffReport, EdgeChangedRef, KindCoverage,
    NodeChangedRef, NodeRef, OrphansReport, PathReport, ReachReport,
};
use serde_json::{Map, Value};

#[derive(serde::Serialize)]
struct NodeRefJson<'a> {
    id: &'a str,
    kind: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tainted: Option<bool>,
}

struct NodeRefsJson<'a>(&'a [NodeRef]);

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
                tainted: node.tainted,
            })?;
        }
        sequence.end()
    }
}

/// The text-format suffix for a node whose every path crosses a vacancy.
fn tainted_marker(node: &NodeRef) -> &'static str {
    if node.tainted == Some(true) {
        " (tainted)"
    } else {
        ""
    }
}

#[derive(serde::Serialize)]
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

// --- at ---

pub(super) fn format_at_plain(report: &AtReport) -> String {
    if report.entries.is_empty() && visible(&report.findings).next().is_none() {
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
            visible(&entry.findings).count()
        ));
        lines.extend(visible(&entry.findings).map(finding_line));
    }
    if visible(&report.findings).next().is_some() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("Findings at this path:".to_string());
        lines.extend(visible(&report.findings).map(finding_line));
    }
    lines.join("\n")
}

pub(super) fn format_at_json(report: &AtReport) -> String {
    #[derive(serde::Serialize)]
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

pub(super) fn format_at_rich(report: &AtReport) -> String {
    if report.entries.is_empty() && visible(&report.findings).next().is_none() {
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
                rjust(&visible(&entry.findings).count().to_string(), 5)
            ));
            lines.extend(visible(&entry.findings).map(rich_finding_line));
        }
    }
    if visible(&report.findings).next().is_some() {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push("Findings at this path:".to_string());
        lines.extend(visible(&report.findings).map(rich_finding_line));
    }
    let findings = report
        .entries
        .iter()
        .map(|e| visible(&e.findings).count())
        .sum::<usize>()
        + visible(&report.findings).count();
    lines.push(format!(
        "\n{} entries, {} finding(s) at {}",
        report.entries.len(),
        findings,
        report.path
    ));
    lines.join("\n")
}

// --- reach ---

pub(super) fn format_reach_plain(report: &ReachReport) -> String {
    if report.nodes.is_empty() {
        return "No nodes.".to_string();
    }
    report
        .nodes
        .iter()
        .map(|n| format!("{} {}{}", n.id, n.kind, tainted_marker(n)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn format_reach_json(report: &ReachReport) -> String {
    #[derive(serde::Serialize)]
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

pub(super) fn format_reach_rich(report: &ReachReport) -> String {
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
            lines.push(format!(
                "{} {}{}",
                ljust(&n.id, 20),
                n.kind,
                tainted_marker(n)
            ));
        }
        // `tainted` is set on every node or on none, so the first node tells
        // whether the walk was asked; the unflagged path does no scan.
        let tally = report
            .nodes
            .first()
            .and_then(|n| n.tainted)
            .map(|_| {
                let tainted = report
                    .nodes
                    .iter()
                    .filter(|n| n.tainted == Some(true))
                    .count();
                format!(", {tainted} tainted")
            })
            .unwrap_or_default();
        lines.push(format!("\n{} node(s){tally}", report.nodes.len()));
    }
    lines.join("\n")
}

// --- path ---

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

pub(super) fn format_path_plain(report: &PathReport) -> String {
    if let Some((nodes, edges)) = report.found_path() {
        path_chain(nodes, edges)
    } else {
        format!("No path from {} to {}.", report.src(), report.tgt())
    }
}

pub(super) fn format_path_json(report: &PathReport) -> String {
    let (found, nodes, edges) = match report.found_path() {
        Some((nodes, edges)) => (true, nodes, edges),
        None => (false, &[][..], &[][..]),
    };
    #[derive(serde::Serialize)]
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

pub(super) fn format_path_rich(report: &PathReport) -> String {
    if let Some((nodes, edges)) = report.found_path() {
        format!("{}\n\n{} edge(s)", path_chain(nodes, edges), edges.len())
    } else {
        format!("No path from {} to {}.", report.src(), report.tgt())
    }
}

// --- orphans ---

pub(super) fn format_orphans_plain(report: &OrphansReport) -> String {
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

pub(super) fn format_orphans_json(report: &OrphansReport) -> String {
    #[derive(serde::Serialize)]
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

    #[derive(serde::Serialize)]
    struct OrphansRoot<'a> {
        kind_filter: &'a Option<String>,
        orphans: OrphansJson<'a>,
    }

    render_json(&OrphansRoot {
        kind_filter: &report.kind_filter,
        orphans: OrphansJson(&report.orphans),
    })
}

pub(super) fn format_orphans_rich(report: &OrphansReport) -> String {
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

// --- counts ---

pub(super) fn format_counts_plain(report: &CountsReport) -> String {
    let mut lines = Vec::new();
    for (kind, n) in &report.nodes {
        lines.push(format!("node {kind} {n}"));
    }
    for (kind, n) in &report.edges {
        lines.push(format!("edge {kind} {n}"));
    }
    lines.join("\n")
}

pub(super) fn format_counts_json(report: &CountsReport) -> String {
    #[derive(serde::Serialize)]
    struct CountsJson<'a> {
        edges: &'a BTreeMap<String, i64>,
        nodes: &'a BTreeMap<String, i64>,
    }

    render_json(&CountsJson {
        edges: &report.edges,
        nodes: &report.nodes,
    })
}

pub(super) fn format_counts_rich(report: &CountsReport) -> String {
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

// --- coverage ---

pub(super) fn format_coverage_plain(report: &CoverageReport) -> String {
    let mut lines = vec!["kind total incoming incoming_pct outgoing outgoing_pct".to_string()];
    for k in &report.kinds {
        lines.push(format!(
            "{} {} {} {:.1} {} {:.1}",
            k.kind, k.total, k.incoming, k.incoming_pct, k.outgoing, k.outgoing_pct
        ));
    }
    lines.join("\n")
}

pub(super) fn format_coverage_json(report: &CoverageReport) -> String {
    #[derive(serde::Serialize)]
    struct KindJson<'a> {
        incoming: i64,
        incoming_pct: f64,
        kind: &'a str,
        outgoing: i64,
        outgoing_pct: f64,
        total: i64,
    }

    struct KindsJson<'a>(&'a [KindCoverage]);

    impl Serialize for KindsJson<'_> {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
            for k in self.0 {
                sequence.serialize_element(&KindJson {
                    incoming: k.incoming,
                    incoming_pct: k.incoming_pct,
                    kind: &k.kind,
                    outgoing: k.outgoing,
                    outgoing_pct: k.outgoing_pct,
                    total: k.total,
                })?;
            }
            sequence.end()
        }
    }

    #[derive(serde::Serialize)]
    struct CoverageJson<'a> {
        kinds: KindsJson<'a>,
    }

    render_json(&CoverageJson {
        kinds: KindsJson(&report.kinds),
    })
}

pub(super) fn format_coverage_rich(report: &CoverageReport) -> String {
    let header = format!(
        "{} {} {} {}",
        ljust("Kind", 20),
        rjust("Total", 5),
        rjust("Incoming", 14),
        rjust("Outgoing", 14)
    );
    let mut lines = vec![header.clone(), "-".repeat(header.chars().count())];
    for k in &report.kinds {
        lines.push(format!(
            "{} {} {} {}",
            ljust(&k.kind, 20),
            rjust(&k.total.to_string(), 5),
            rjust(&format!("{} ({:.1}%)", k.incoming, k.incoming_pct), 14),
            rjust(&format!("{} ({:.1}%)", k.outgoing, k.outgoing_pct), 14)
        ));
    }
    lines.push(format!("\n{} kind(s)", report.kinds.len()));
    lines.join("\n")
}

// --- diff ---

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
    for e in &report.edges_changed {
        lines.push(format!("edge changed {} {} {}", e.src, e.kind, e.tgt));
    }
    for name in &report.pathways_changed {
        lines.push(format!("pathway changed {name}"));
    }
    lines
}

pub(super) fn format_diff_plain(report: &DiffReport) -> String {
    if report.is_empty() {
        return "No differences.".to_string();
    }
    diff_lines(report).join("\n")
}

#[derive(serde::Serialize)]
struct NodeChangedJson<'a> {
    attrs_a: &'a Map<String, Value>,
    attrs_b: &'a Map<String, Value>,
    id: &'a str,
    kind: &'a str,
}

struct NodesChangedJson<'a>(&'a [NodeChangedRef]);

impl Serialize for NodesChangedJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for node in self.0 {
            sequence.serialize_element(&NodeChangedJson {
                attrs_a: &node.attrs_a,
                attrs_b: &node.attrs_b,
                id: &node.id,
                kind: &node.kind,
            })?;
        }
        sequence.end()
    }
}

#[derive(serde::Serialize)]
struct EdgeChangedJson<'a> {
    attrs_a: &'a Map<String, Value>,
    attrs_b: &'a Map<String, Value>,
    kind: &'a str,
    src: &'a str,
    tgt: &'a str,
}

struct EdgesChangedJson<'a>(&'a [EdgeChangedRef]);

impl Serialize for EdgesChangedJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for edge in self.0 {
            sequence.serialize_element(&EdgeChangedJson {
                attrs_a: &edge.attrs_a,
                attrs_b: &edge.attrs_b,
                kind: &edge.kind,
                src: &edge.src,
                tgt: &edge.tgt,
            })?;
        }
        sequence.end()
    }
}

pub(super) fn format_diff_json(report: &DiffReport) -> String {
    #[derive(serde::Serialize)]
    struct DiffJson<'a> {
        pathways_changed: &'a [String],
        edges_added: EdgeRefsJson<'a>,
        edges_changed: EdgesChangedJson<'a>,
        edges_removed: EdgeRefsJson<'a>,
        nodes_added: NodeRefsJson<'a>,
        nodes_changed: NodesChangedJson<'a>,
        nodes_removed: NodeRefsJson<'a>,
        rev_a: &'a str,
        rev_b: &'a str,
    }

    render_json(&DiffJson {
        pathways_changed: &report.pathways_changed,
        edges_added: EdgeRefsJson(&report.edges_added),
        edges_changed: EdgesChangedJson(&report.edges_changed),
        edges_removed: EdgeRefsJson(&report.edges_removed),
        nodes_added: NodeRefsJson(&report.nodes_added),
        nodes_changed: NodesChangedJson(&report.nodes_changed),
        nodes_removed: NodeRefsJson(&report.nodes_removed),
        rev_a: &report.rev_a,
        rev_b: &report.rev_b,
    })
}

pub(super) fn format_diff_rich(report: &DiffReport) -> String {
    let header = format!("{} -> {}", report.rev_a, report.rev_b);
    if report.is_empty() {
        return format!("{header}\nNo differences.");
    }
    let lines = diff_lines(report);
    let count = lines.len();
    format!("{header}\n{}\n\n{count} difference(s)", lines.join("\n"))
}
