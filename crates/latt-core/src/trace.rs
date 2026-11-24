//! Assembling the trace-report payload: every node with its edges and findings.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::graph::{Edge, LatticeGraph, Node};
use crate::profile::Profile;
use crate::types::{Issue, PathwayEntry, TraceEdge, TraceEntry, TraceReport};

/// Undeclared kinds get rank 999, sorting after up to 999 declared kinds.
/// The fixed value mirrors the reference core's sentinel.
const UNDECLARED_KIND_RANK: usize = 999;

/// Everything the entry pass needs that comes from reading the graph, held
/// without borrowing it so the owning path can then move the graph's nodes and
/// edges into the report.
struct Plan {
    header: BTreeMap<String, String>,
    /// Findings attached to a node, by node slot.
    findings: HashMap<usize, Vec<Issue>>,
    unattachable: Vec<Issue>,
    /// Edge indices grouped by source node slot, each group in report order.
    order: Vec<usize>,
    /// `order[starts[slot]..starts[slot + 1]]` is the group for `slot`.
    starts: Vec<usize>,
    /// Slots of the nodes the report carries.
    selected: Vec<bool>,
    pathways: Vec<PathwayEntry>,
}

impl Plan {
    fn read(
        profile: &Profile,
        graph: &LatticeGraph,
        issues: Vec<Issue>,
        lattice_version: &str,
        node_ids: Option<&BTreeSet<&str>>,
    ) -> Self {
        let mut header = BTreeMap::from([
            ("lattice_version".to_string(), lattice_version.to_string()),
            ("profile".to_string(), profile.name().to_owned()),
            (
                "profile_version".to_string(),
                profile.profile_version().to_owned(),
            ),
        ]);
        header.insert("trace_version".to_owned(), "2".to_owned());

        let selected: Vec<bool> = graph
            .iter_nodes()
            .map(|node| node_ids.is_none_or(|ids| ids.contains(node.id.as_str())))
            .collect();

        let mut findings: HashMap<usize, Vec<Issue>> = HashMap::new();
        let mut unattachable: Vec<Issue> = Vec::new();
        for issue in issues {
            // A finding whose node_id names no graph node (e.g. VACANCY on a ghost
            // source) must not vanish; no entry would ever carry it.
            match issue
                .node_id
                .as_deref()
                .and_then(|id| graph.slot(id))
                .filter(|&slot| selected[slot])
            {
                Some(slot) => findings.entry(slot).or_default().push(issue),
                None => unattachable.push(issue),
            }
        }

        // One sort over edge indices rather than a rescan per node. The key
        // carries the referencing edge's provenance, so repeated targets order by
        // where they were written rather than by the order the graph was built
        // in. A source naming no declared node has no entry to carry its edges.
        let edges = graph.edges();
        let source_slot: Vec<Option<usize>> = edges
            .iter()
            .map(|edge| graph.slot(&edge.src).filter(|&slot| selected[slot]))
            .collect();
        let mut order: Vec<usize> = (0..edges.len())
            .filter(|&index| source_slot[index].is_some())
            .collect();
        order.sort_unstable_by(|&a, &b| {
            let (ea, eb) = (&edges[a], &edges[b]);
            (
                source_slot[a],
                &ea.kind,
                &ea.tgt,
                ea.provenance.file(),
                ea.provenance.line(),
                a,
            )
                .cmp(&(
                    source_slot[b],
                    &eb.kind,
                    &eb.tgt,
                    eb.provenance.file(),
                    eb.provenance.line(),
                    b,
                ))
        });
        let mut starts = vec![0usize; selected.len() + 1];
        for &index in &order {
            if let Some(slot) = source_slot[index] {
                starts[slot + 1] += 1;
            }
        }
        for slot in 0..selected.len() {
            starts[slot + 1] += starts[slot];
        }

        Self {
            header,
            findings,
            unattachable,
            order,
            starts,
            selected,
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

    /// Assemble the report from nodes and edges the caller either borrowed
    /// (cloned here) or owns (moved here).
    fn finish<'a>(
        mut self,
        profile: &Profile,
        nodes: Vec<Cow<'a, Node>>,
        edges: Vec<Cow<'a, Edge>>,
    ) -> TraceReport {
        let mut edges: Vec<Option<Cow<'a, Edge>>> = edges.into_iter().map(Some).collect();
        // The rank is computed once per entry here, not once per comparison in
        // the sort: the order is the same (rank, id) total order either way.
        let mut ranked: Vec<(usize, TraceEntry)> = Vec::new();
        for (slot, node) in nodes.into_iter().enumerate() {
            if !self.selected[slot] {
                continue;
            }
            let node = node.into_owned();
            let entry_edges = self.order[self.starts[slot]..self.starts[slot + 1]]
                .iter()
                .filter_map(|&index| edges[index].take())
                .map(|edge| {
                    let edge = edge.into_owned();
                    TraceEdge {
                        tgt: edge.tgt,
                        kind: edge.kind,
                        attrs: edge.attrs,
                        provenance: edge.provenance,
                    }
                })
                .collect();
            let summary_attr = profile
                .node_kinds()
                .get(&node.kind)
                .and_then(|k| k.summary_attr.clone());
            let rank = kind_rank(profile, &node.kind);
            let entry = TraceEntry {
                summary_attr,
                edges: entry_edges,
                findings: self.findings.remove(&slot).unwrap_or_default(),
                id: node.id,
                kind: node.kind,
                attrs: node.attrs,
                provenance: node.provenance,
            };
            ranked.push((rank, entry));
        }
        ranked.sort_by(|(rank_a, a), (rank_b, b)| (rank_a, &a.id).cmp(&(rank_b, &b.id)));
        TraceReport {
            header: self.header,
            entries: ranked.into_iter().map(|(_, entry)| entry).collect(),
            unattachable_findings: self.unattachable,
            pathways: self.pathways,
        }
    }
}

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

/// Build the trace report, consuming the graph: node and edge attributes move
/// into the report instead of being copied, so peak memory holds the register once.
#[must_use]
pub fn build_trace_report_owned(
    profile: &Profile,
    graph: LatticeGraph,
    issues: Vec<Issue>,
    lattice_version: &str,
) -> TraceReport {
    let plan = Plan::read(profile, &graph, issues, lattice_version, None);
    let (nodes, edges) = graph.into_nodes_and_edges();
    plan.finish(
        profile,
        nodes.into_iter().map(Cow::Owned).collect(),
        edges.into_iter().map(Cow::Owned).collect(),
    )
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
    Plan::read(profile, graph, issues, lattice_version, node_ids).finish(
        profile,
        graph.nodes().iter().map(Cow::Borrowed).collect(),
        graph.edges().iter().map(Cow::Borrowed).collect(),
    )
}

fn kind_rank(profile: &Profile, kind: &str) -> usize {
    profile
        .node_kinds()
        .get(kind)
        .map_or(UNDECLARED_KIND_RANK, |k| k.declared_index)
}
