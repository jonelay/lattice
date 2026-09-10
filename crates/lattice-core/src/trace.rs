//! Assembling the trace-report payload: every node with its edges and findings.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::LatticeGraph;
use crate::profile::Profile;
use crate::types::{Issue, PathwayEntry, TraceEdge, TraceEntry, TraceReport};

/// The rank a kind the profile does not declare sorts at — after every declared
/// kind, and mirroring the reference core's fixed sentinel rather than inventing
/// a wider one.
const UNDECLARED_KIND_RANK: usize = 999;

/// Outgoing edges by source node, then by edge kind.
type Adjacency<'a> = BTreeMap<&'a str, BTreeMap<&'a str, Vec<&'a crate::graph::Edge>>>;

/// Build the trace report for a validated graph.
///
/// `lattice_version` is the caller's, so the library never has to name the
/// binary it was linked into.
#[must_use]
pub fn build_trace_report(
    profile: &Profile,
    graph: &LatticeGraph,
    issues: Vec<Issue>,
    lattice_version: &str,
) -> TraceReport {
    build_trace_report_for_nodes(profile, graph, issues, lattice_version, None)
}

/// Build trace entries only for `node_ids`, while retaining omitted-node
/// findings as unattachable so callers can still select them by provenance.
#[must_use]
pub(crate) fn build_trace_report_for_nodes(
    profile: &Profile,
    graph: &LatticeGraph,
    issues: Vec<Issue>,
    lattice_version: &str,
    node_ids: Option<&BTreeSet<&str>>,
) -> TraceReport {
    let mut header = BTreeMap::from([
        ("lattice_version".to_string(), lattice_version.to_string()),
        ("profile".to_string(), profile.name().to_owned()),
        (
            "profile_version".to_string(),
            profile.profile_version().to_owned(),
        ),
    ]);
    header.insert("trace_version".to_owned(), "2".to_owned());

    let mut findings_by_node: BTreeMap<&str, Vec<Issue>> = BTreeMap::new();
    let mut unattachable: Vec<Issue> = Vec::new();
    for issue in issues {
        // A node_id naming no graph node — a VACANCY on a ghost source, say
        // — must not vanish: no entry would ever carry it.
        match issue.node_id.as_deref() {
            Some(node_id)
                if graph.has_node(node_id) && node_ids.is_none_or(|ids| ids.contains(node_id)) =>
            {
                let key = graph.node(node_id).expect("just checked").id.as_str();
                findings_by_node.entry(key).or_default().push(issue);
            }
            _ => unattachable.push(issue),
        }
    }

    // One pass over the edges rather than a rescan per node. The sort key carries
    // the referencing edge's provenance, so repeated targets order by where they
    // were written rather than by the order the graph was built in.
    let mut adjacency: Adjacency<'_> = BTreeMap::new();
    for edge in graph.iter_edges() {
        if node_ids.is_some_and(|ids| !ids.contains(edge.src.as_str())) {
            continue;
        }
        adjacency
            .entry(&edge.src)
            .or_default()
            .entry(&edge.kind)
            .or_default()
            .push(edge);
    }
    // Sorted in place, once, rather than per node: the entry pass reads these
    // lists behind a shared borrow and would otherwise have to copy each one to
    // sort it.
    for by_kind in adjacency.values_mut() {
        for edges in by_kind.values_mut() {
            edges.sort_unstable_by(|a, b| {
                (&a.tgt, &a.provenance.file, a.provenance.line).cmp(&(
                    &b.tgt,
                    &b.provenance.file,
                    b.provenance.line,
                ))
            });
        }
    }

    let mut entries: Vec<TraceEntry> = graph
        .iter_nodes()
        .filter(|node| node_ids.is_none_or(|ids| ids.contains(node.id.as_str())))
        .map(|node| {
            let edges = adjacency
                .get(node.id.as_str())
                .map(|by_kind| {
                    by_kind
                        .iter()
                        .flat_map(|(kind, edges)| {
                            edges.iter().map(|edge| TraceEdge {
                                tgt: edge.tgt.clone(),
                                kind: (*kind).to_string(),
                                attrs: edge.attrs.clone(),
                                provenance: edge.provenance.clone(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();

            let summary_attr = profile
                .node_kinds()
                .get(&node.kind)
                .and_then(|k| k.summary_attr.clone());

            TraceEntry {
                id: node.id.clone(),
                kind: node.kind.clone(),
                attrs: node.attrs.clone().into_iter().collect(),
                provenance: node.provenance.clone(),
                summary_attr,
                edges,
                findings: findings_by_node
                    .remove(node.id.as_str())
                    .unwrap_or_default(),
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        (kind_rank(profile, &a.kind), &a.id).cmp(&(kind_rank(profile, &b.kind), &b.id))
    });

    TraceReport {
        header,
        entries,
        unattachable_findings: unattachable,
        pathways: graph
            .iter_pathways()
            .map(|pathway| PathwayEntry {
                name: pathway.name.clone(),
                order: pathway.order.clone(),
                current: pathway.current.clone(),
            })
            .collect(),
    }
}

fn kind_rank(profile: &Profile, kind: &str) -> usize {
    profile
        .node_kinds()
        .get(kind)
        .map_or(UNDECLARED_KIND_RANK, |k| k.declared_index)
}
