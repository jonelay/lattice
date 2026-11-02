//! The adjacency and slot indexes, two per-invocation views over a borrowed graph.

use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::algo::Neighbors;
use crate::graph::{Edge, LatticeGraph};
use crate::profile::Profile;

use super::{Direction, QueryError};

/// Bidirectional adjacency index over a borrowed graph.
///
/// Built once per session outside the graph (which has no finalize step).
/// Both directions are indexed at construction; `edges_from` and `edges_to`
/// return sorted slices of `&Edge` in deterministic order: (far endpoint,
/// edge kind), with ties broken by ingestion order.
#[derive(Clone, Debug)]
pub struct AdjacencyIndex<'a> {
    outgoing: HashMap<&'a str, Vec<&'a Edge>>,
    incoming: HashMap<&'a str, Vec<&'a Edge>>,
}

impl<'a> AdjacencyIndex<'a> {
    /// Build the index over every edge, walking them once.
    #[must_use]
    pub fn build(graph: &'a LatticeGraph) -> Self {
        Self::from_edges(graph.iter_edges())
    }

    /// Build the index over edges of the given kinds only, walking all edges once.
    ///
    /// The same index as [`build`](Self::build) then [`filtered`](Self::filtered),
    /// without the intermediate full index. An empty `kinds` slice means no
    /// edges (use [`build`](Self::build) for all).
    #[must_use]
    pub fn build_filtered(graph: &'a LatticeGraph, kinds: &[&str]) -> Self {
        let kind_set: BTreeSet<&str> = kinds.iter().copied().collect();
        Self::from_edges(
            graph
                .iter_edges()
                .filter(|edge| kind_set.contains(edge.kind.as_str())),
        )
    }

    fn from_edges(edges: impl Iterator<Item = &'a Edge>) -> Self {
        let mut outgoing: HashMap<&str, Vec<&Edge>> = HashMap::new();
        let mut incoming: HashMap<&str, Vec<&Edge>> = HashMap::new();
        for edge in edges {
            outgoing.entry(edge.src.as_str()).or_default().push(edge);
            incoming.entry(edge.tgt.as_str()).or_default().push(edge);
        }
        for list in outgoing.values_mut() {
            list.sort_by_key(|e| (&e.tgt, &e.kind));
        }
        for list in incoming.values_mut() {
            list.sort_by_key(|e| (&e.src, &e.kind));
        }
        Self { outgoing, incoming }
    }

    /// Build a filtered index containing only edges of the specified kinds.
    ///
    /// An empty `kinds` slice means no edges (use the unfiltered index for all).
    #[must_use]
    pub fn filtered(&self, kinds: &[&str]) -> AdjacencyIndex<'a> {
        let kind_set: BTreeSet<&str> = kinds.iter().copied().collect();
        let filter_map =
            |source: &HashMap<&'a str, Vec<&'a Edge>>| -> HashMap<&'a str, Vec<&'a Edge>> {
                let mut result: HashMap<&str, Vec<&Edge>> = HashMap::new();
                for (&node, edges) in source {
                    let filtered: Vec<&Edge> = edges
                        .iter()
                        .filter(|e| kind_set.contains(e.kind.as_str()))
                        .copied()
                        .collect();
                    if !filtered.is_empty() {
                        result.insert(node, filtered);
                    }
                }
                result
            };
        AdjacencyIndex {
            outgoing: filter_map(&self.outgoing),
            incoming: filter_map(&self.incoming),
        }
    }

    /// Outgoing edges from `id`, sorted by (target, edge kind).
    #[doc(hidden)]
    pub fn edges_from(&self, id: &str) -> &[&'a Edge] {
        self.outgoing.get(id).map(Vec::as_slice).unwrap_or_default()
    }

    /// Incoming edges to `id`, sorted by (source, edge kind).
    #[doc(hidden)]
    pub fn edges_to(&self, id: &str) -> &[&'a Edge] {
        self.incoming.get(id).map(Vec::as_slice).unwrap_or_default()
    }

    /// True when `id` has at least one incoming edge.
    #[doc(hidden)]
    pub fn has_incoming(&self, id: &str) -> bool {
        self.incoming.get(id).is_some_and(|edges| !edges.is_empty())
    }

    /// True when `id` has at least one outgoing edge.
    #[doc(hidden)]
    pub fn has_outgoing(&self, id: &str) -> bool {
        self.outgoing.get(id).is_some_and(|edges| !edges.is_empty())
    }

    /// True when `id` appears as either endpoint in any edge.
    #[doc(hidden)]
    pub fn connected(&self, id: &str) -> bool {
        self.has_incoming(id) || self.has_outgoing(id)
    }

    /// Edges in the walk direction from `id`.
    pub(crate) fn edges_in_direction(&self, id: &str, direction: Direction) -> &[&'a Edge] {
        match direction {
            Direction::Forward => self.edges_from(id),
            Direction::Reverse => self.edges_to(id),
        }
    }

    /// The far endpoint of an edge in the walk direction.
    pub(crate) fn far_endpoint(edge: &Edge, direction: Direction) -> &str {
        match direction {
            Direction::Forward => &edge.tgt,
            Direction::Reverse => &edge.src,
        }
    }
}

impl<'a> Neighbors for AdjacencyIndex<'a> {
    type NodeId = &'a str;

    fn neighbors(&self, id: &'a str, direction: Direction) -> impl Iterator<Item = &'a str> + '_ {
        self.edges_in_direction(id, direction)
            .iter()
            .map(move |&edge| Self::far_endpoint(edge, direction))
    }

    fn node_ids(&self) -> impl Iterator<Item = &'a str> + '_ {
        let incoming_only = self
            .incoming
            .keys()
            .filter(|id| !self.outgoing.contains_key(*id));
        self.outgoing.keys().chain(incoming_only).copied()
    }
}

/// Refuse an edge kind the profile does not declare, before any traversal.
///
/// Of several unknown kinds the smallest is named, so the message does not
/// depend on the order the caller listed them in.
pub(super) fn check_edge_kinds(profile: &Profile, kinds: &[&str]) -> Result<(), QueryError> {
    match kinds
        .iter()
        .filter(|kind| !profile.edge_kinds().contains_key(**kind))
        .min()
    {
        Some(kind) => Err(QueryError::new(format!(
            "unknown edge kind '{kind}': the profile declares {:?}",
            profile.edge_kinds().keys().collect::<Vec<_>>()
        ))),
        None => Ok(()),
    }
}

/// A one-direction adjacency over node slots, built for one walk and dropped
/// with it.
///
/// Declared nodes keep their graph slot; an endpoint no adapter declared gets a
/// slot from `declared` up, so a walk still passes through a vacancy. Each row
/// holds a node's edges in the walk direction ordered by far endpoint, then
/// kind, with ties in ingestion order: the order [`AdjacencyIndex`] gives, which
/// the path search's tie-break depends on. Walking slots instead of hashing ID
/// strings at every step removes the hash probe per step that dominated both.
pub(super) struct SlotIndex<'a> {
    /// Every slot's ID: the declared nodes, then the vacancies.
    pub(super) ids: Vec<&'a str>,
    pub(super) declared: usize,
    /// Row `slot` is `edges[offsets[slot]..offsets[slot + 1]]`.
    offsets: Vec<usize>,
    /// Far endpoint slot and edge kind.
    edges: Vec<(usize, &'a str)>,
}

impl<'a> SlotIndex<'a> {
    /// Index the edges of `edge_kinds`, or every edge when none are named.
    pub(super) fn build(
        graph: &'a LatticeGraph,
        edge_kinds: &[&str],
        direction: Direction,
    ) -> Self {
        let kinds: BTreeSet<&str> = edge_kinds.iter().copied().collect();
        let declared = graph.node_count();
        let mut ids: Vec<&str> = graph.iter_nodes().map(|node| node.id.as_str()).collect();
        let mut vacancies: HashMap<&str, usize> = HashMap::new();
        let mut slot_of = |id: &'a str, ids: &mut Vec<&'a str>| {
            graph.slot(id).unwrap_or_else(|| {
                *vacancies.entry(id).or_insert_with(|| {
                    ids.push(id);
                    ids.len() - 1
                })
            })
        };
        let mut pairs: Vec<(usize, usize, &str)> = Vec::new();
        for edge in graph.iter_edges() {
            if !kinds.is_empty() && !kinds.contains(edge.kind.as_str()) {
                continue;
            }
            let (near, far) = match direction {
                Direction::Forward => (edge.src.as_str(), edge.tgt.as_str()),
                Direction::Reverse => (edge.tgt.as_str(), edge.src.as_str()),
            };
            let near = slot_of(near, &mut ids);
            let far = slot_of(far, &mut ids);
            pairs.push((near, far, edge.kind.as_str()));
        }

        // Bucket by near slot, keeping ingestion order within each row.
        let mut offsets = vec![0; ids.len() + 1];
        for &(near, _, _) in &pairs {
            offsets[near + 1] += 1;
        }
        for slot in 0..ids.len() {
            offsets[slot + 1] += offsets[slot];
        }
        let mut cursor = offsets.clone();
        let mut edges = vec![(0, ""); pairs.len()];
        for (near, far, kind) in pairs {
            edges[cursor[near]] = (far, kind);
            cursor[near] += 1;
        }
        for slot in 0..ids.len() {
            edges[offsets[slot]..offsets[slot + 1]]
                .sort_by(|a, b| ids[a.0].cmp(ids[b.0]).then_with(|| a.1.cmp(b.1)));
        }
        Self {
            ids,
            declared,
            offsets,
            edges,
        }
    }

    pub(super) fn row(&self, slot: usize) -> &[(usize, &'a str)] {
        &self.edges[self.offsets[slot]..self.offsets[slot + 1]]
    }

    /// Which slots a breadth-first walk from `origin` reaches, excluding the
    /// origin, entering a slot only when `enter` admits it.
    pub(super) fn reached(&self, origin: usize, enter: impl Fn(usize) -> bool) -> Vec<bool> {
        let mut seen = vec![false; self.ids.len()];
        seen[origin] = true;
        let mut queue = VecDeque::from([origin]);
        while let Some(slot) = queue.pop_front() {
            for &(far, _) in self.row(slot) {
                if !seen[far] && enter(far) {
                    seen[far] = true;
                    queue.push_back(far);
                }
            }
        }
        seen[origin] = false;
        seen
    }
}

/// The slot of a declared node, or the error for one the graph does not hold.
pub(super) fn node_slot(graph: &LatticeGraph, id: &str) -> Result<usize, QueryError> {
    graph
        .slot(id)
        .ok_or_else(|| QueryError::new(format!("unknown node '{id}'")))
}

#[cfg(test)]
mod adjacency_tests {
    use super::AdjacencyIndex;
    use crate::graph::LatticeGraph;
    use crate::types::Provenance;
    use serde_json::Map;

    fn prov() -> Provenance {
        Provenance::new("test.md", 1)
    }

    fn build_graph() -> LatticeGraph {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "test", Map::new(), prov()).unwrap();
        g.add_node("C", "spec", Map::new(), prov()).unwrap();
        g.add_edge(
            crate::graph::EdgeSpec {
                src: "A".into(),
                tgt: "B".into(),
                kind: "verifies".into(),
                attrs: Map::new(),
            },
            prov(),
        );
        g.add_edge(
            crate::graph::EdgeSpec {
                src: "A".into(),
                tgt: "C".into(),
                kind: "derives".into(),
                attrs: Map::new(),
            },
            prov(),
        );
        g.add_edge(
            crate::graph::EdgeSpec {
                src: "C".into(),
                tgt: "B".into(),
                kind: "verifies".into(),
                attrs: Map::new(),
            },
            prov(),
        );
        g
    }

    #[test]
    fn node_ids_yields_each_endpoint_once_including_incoming_only() {
        use crate::algo::Neighbors;
        use crate::graph::EdgeSpec;
        let mut g = LatticeGraph::new();
        for id in ["A", "B", "C"] {
            g.add_node(id, "req", Map::new(), prov()).unwrap();
        }
        let edge = |src: &str, tgt: &str| EdgeSpec {
            src: src.into(),
            tgt: tgt.into(),
            kind: "k".into(),
            attrs: Map::new(),
        };
        g.add_edge(edge("A", "B"), prov());
        g.add_edge(edge("A", "C"), prov());
        g.add_edge(edge("B", "C"), prov());
        let idx = AdjacencyIndex::build(&g);
        let mut ids: Vec<&str> = idx.node_ids().collect();
        ids.sort_unstable();
        assert_eq!(ids, ["A", "B", "C"]);
        assert_eq!(idx.filtered(&[]).node_ids().count(), 0);
    }

    #[test]
    fn edges_from_returns_sorted_by_far_endpoint_then_kind() {
        let g = build_graph();
        let idx = AdjacencyIndex::build(&g);
        let from_a: Vec<(&str, &str)> = idx
            .edges_from("A")
            .iter()
            .map(|e| (e.tgt.as_str(), e.kind.as_str()))
            .collect();
        assert_eq!(from_a, vec![("B", "verifies"), ("C", "derives")]);
    }

    #[test]
    fn edges_to_returns_sorted_by_far_endpoint_then_kind() {
        let g = build_graph();
        let idx = AdjacencyIndex::build(&g);
        let to_b: Vec<(&str, &str)> = idx
            .edges_to("B")
            .iter()
            .map(|e| (e.src.as_str(), e.kind.as_str()))
            .collect();
        assert_eq!(to_b, vec![("A", "verifies"), ("C", "verifies")]);
    }

    #[test]
    fn unknown_id_returns_empty() {
        let g = build_graph();
        let idx = AdjacencyIndex::build(&g);
        assert!(idx.edges_from("nonexistent").is_empty());
        assert!(idx.edges_to("nonexistent").is_empty());
    }

    #[test]
    fn undeclared_endpoints_are_indexed() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_edge(
            crate::graph::EdgeSpec {
                src: "A".into(),
                tgt: "X".into(),
                kind: "verifies".into(),
                attrs: Map::new(),
            },
            prov(),
        );
        let idx = AdjacencyIndex::build(&g);
        assert_eq!(idx.edges_from("A").len(), 1);
        assert_eq!(idx.edges_to("X").len(), 1);
    }

    #[test]
    fn parallel_edges_preserved() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "test", Map::new(), prov()).unwrap();
        g.add_edge(
            crate::graph::EdgeSpec {
                src: "A".into(),
                tgt: "B".into(),
                kind: "verifies".into(),
                attrs: Map::new(),
            },
            Provenance::new("a.md", 1),
        );
        g.add_edge(
            crate::graph::EdgeSpec {
                src: "A".into(),
                tgt: "B".into(),
                kind: "verifies".into(),
                attrs: Map::new(),
            },
            Provenance::new("b.md", 2),
        );
        let idx = AdjacencyIndex::build(&g);
        let from_a = idx.edges_from("A");
        assert_eq!(from_a.len(), 2);
        assert_eq!(from_a[0].provenance.file(), "a.md");
        assert_eq!(from_a[1].provenance.file(), "b.md");
    }

    #[test]
    fn filtered_retains_only_matching_kinds() {
        let g = build_graph();
        let idx = AdjacencyIndex::build(&g);
        let filtered = idx.filtered(&["verifies"]);
        let from_a: Vec<&str> = filtered
            .edges_from("A")
            .iter()
            .map(|e| e.tgt.as_str())
            .collect();
        assert_eq!(from_a, vec!["B"]);
    }

    #[test]
    fn build_filtered_equals_build_then_filtered() {
        use crate::algo::Neighbors;
        use crate::graph::Edge;
        let g = build_graph();
        let full = AdjacencyIndex::build(&g);
        for kinds in [&[][..], &["verifies"][..], &["verifies", "depends"][..]] {
            let copied = full.filtered(kinds);
            let direct = AdjacencyIndex::build_filtered(&g, kinds);
            let mut copied_ids: Vec<&str> = copied.node_ids().collect();
            let mut direct_ids: Vec<&str> = direct.node_ids().collect();
            copied_ids.sort_unstable();
            direct_ids.sort_unstable();
            assert_eq!(direct_ids, copied_ids, "{kinds:?}");
            for node in g.iter_nodes() {
                let id = node.id.as_str();
                let key = |e: &&Edge| (e.src.clone(), e.tgt.clone(), e.kind.clone());
                let out_copied: Vec<_> = copied.edges_from(id).iter().map(key).collect();
                let out_direct: Vec<_> = direct.edges_from(id).iter().map(key).collect();
                assert_eq!(out_direct, out_copied, "{kinds:?} from {id}");
                let in_copied: Vec<_> = copied.edges_to(id).iter().map(key).collect();
                let in_direct: Vec<_> = direct.edges_to(id).iter().map(key).collect();
                assert_eq!(in_direct, in_copied, "{kinds:?} to {id}");
            }
        }
    }

    #[test]
    fn has_incoming_outgoing_connected() {
        let g = build_graph();
        let idx = AdjacencyIndex::build(&g);
        assert!(idx.has_outgoing("A"));
        assert!(!idx.has_incoming("A"));
        assert!(idx.has_incoming("B"));
        assert!(!idx.has_outgoing("B"));
        assert!(idx.connected("A"));
        assert!(idx.connected("B"));
    }
}
