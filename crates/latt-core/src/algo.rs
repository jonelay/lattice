//! Graph algorithms over any representation that can enumerate neighbors.
//!
//! [`crate::query::AdjacencyIndex`] is one implementation. An embedding host
//! can implement [`Neighbors`] over its own graph and run these algorithms
//! without building a `LatticeGraph`. Algorithms that need edge data stay in
//! `query`.
#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hash;

use crate::query::Direction;

/// A directed graph seen only through node identity and adjacency.
///
/// A neighbor may repeat when parallel edges exist; algorithms dedupe. The
/// order `neighbors` yields is part of the implementor's contract: results
/// are deterministic only if it is.
///
/// Hosts implement this trait, so it only grows compatibly: a method added in
/// a later release carries a default body, and an existing implementation
/// keeps compiling.
///
/// A host graph held as adjacency lists, run through [`scc`]:
///
/// ```
/// use std::collections::HashSet;
/// use latt_core::algo::{Neighbors, scc};
/// use latt_core::query::Direction;
///
/// struct Lists(Vec<Vec<usize>>);
///
/// impl Neighbors for Lists {
///     type NodeId = usize;
///     fn neighbors(&self, id: usize, direction: Direction) -> impl Iterator<Item = usize> + '_ {
///         let forward = matches!(direction, Direction::Forward);
///         (0..self.0.len()).filter(move |&other| {
///             if forward { self.0[id].contains(&other) } else { self.0[other].contains(&id) }
///         })
///     }
///     fn node_ids(&self) -> impl Iterator<Item = usize> + '_ {
///         0..self.0.len()
///     }
/// }
///
/// // 0 ⇄ 1 is a cycle; 2 hangs off it.
/// let graph = Lists(vec![vec![1], vec![0, 2], vec![]]);
/// let every: HashSet<usize> = graph.node_ids().collect();
/// let components = scc(&graph, &every);
/// assert_eq!(components, vec![vec![0, 1], vec![2]]);
/// ```
pub trait Neighbors {
    /// Ordered so that results are deterministic regardless of hash seed.
    type NodeId: Copy + Eq + Hash + Ord;
    /// Nodes one edge away from `id`, following edges in `direction`.
    fn neighbors(
        &self,
        id: Self::NodeId,
        direction: Direction,
    ) -> impl Iterator<Item = Self::NodeId> + '_;
    /// Every node the graph knows, in no particular order.
    fn node_ids(&self) -> impl Iterator<Item = Self::NodeId> + '_;
}

/// Every node a breadth-first walk from `origin` reaches, excluding the
/// origin. The walk enters a node only when `enter` admits it, so a refused
/// node neither appears nor is expanded through.
#[must_use]
pub fn bfs_reachable<G: Neighbors>(
    graph: &G,
    origin: G::NodeId,
    direction: Direction,
    enter: impl Fn(G::NodeId) -> bool,
) -> BTreeSet<G::NodeId> {
    let mut seen = BTreeSet::from([origin]);
    let mut queue = VecDeque::from([origin]);
    while let Some(id) = queue.pop_front() {
        for far in graph.neighbors(id, direction) {
            if enter(far) && seen.insert(far) {
                queue.push_back(far);
            }
        }
    }
    seen.remove(&origin);
    seen
}

/// Nodes a breadth-first walk from `origin` reaches, grouped by hop distance.
///
/// Index 0 holds the depth-1 nodes, and the walk stops at `max_depth`. The origin
/// is excluded, each layer is sorted, and the walk enters a node only when
/// `enter` admits it.
#[must_use]
pub fn bfs_layers<G: Neighbors>(
    graph: &G,
    origin: G::NodeId,
    direction: Direction,
    max_depth: usize,
    enter: impl Fn(G::NodeId) -> bool,
) -> Vec<Vec<G::NodeId>> {
    let mut layers: Vec<Vec<G::NodeId>> = Vec::new();
    let mut seen = HashSet::from([origin]);
    for _ in 0..max_depth {
        let frontier = layers
            .last()
            .map_or(std::slice::from_ref(&origin), Vec::as_slice);
        let mut next: Vec<G::NodeId> = frontier
            .iter()
            .flat_map(|&id| graph.neighbors(id, direction))
            .filter(|&far| enter(far) && seen.insert(far))
            .collect();
        if next.is_empty() {
            break;
        }
        next.sort_unstable();
        layers.push(next);
    }
    layers
}

/// Betweenness centrality via Brandes' algorithm, normalized to [0, 1].
///
/// Runs from every node the graph enumerates, following edges forward.
/// Returns a score per enumerated node; the maximum score is scaled to 1.0,
/// and a graph with no path through any node returns all zeros. A neighbor the
/// graph does not enumerate is walked through, never from, and is scored when
/// an enumerated node reaches it; the core index enumerates every node with an
/// edge, so there the result is exactly those nodes. Parallel edges count as one.
#[must_use]
pub fn betweenness_centrality<G: Neighbors>(graph: &G) -> HashMap<G::NodeId, f64> {
    let mut ids: Vec<G::NodeId> = graph.node_ids().collect();
    ids.sort_unstable();
    ids.dedup();
    let sources = ids.len();
    // Dense slots, resolved once: the walk from every source then indexes vectors
    // instead of hashing IDs. A neighbor the graph does not enumerate takes a slot
    // after the sources when first met; it is walked through, never walked from.
    let mut slot: HashMap<G::NodeId, usize> =
        ids.iter().enumerate().map(|(at, &id)| (id, at)).collect();
    let mut successors: Vec<Vec<usize>> = Vec::with_capacity(ids.len());
    while successors.len() < ids.len() {
        let mut far: Vec<G::NodeId> = graph
            .neighbors(ids[successors.len()], Direction::Forward)
            .collect();
        // Ascending and once each, so every sum below accumulates in the same
        // order whatever order the graph yields neighbors in.
        far.sort_unstable();
        far.dedup();
        let row = far
            .into_iter()
            .map(|id| {
                *slot.entry(id).or_insert_with(|| {
                    ids.push(id);
                    ids.len() - 1
                })
            })
            .collect();
        successors.push(row);
    }

    let count = ids.len();
    let mut centrality = vec![0.0_f64; count];
    let mut dist = vec![usize::MAX; count];
    let mut sigma = vec![0.0_f64; count];
    let mut delta = vec![0.0_f64; count];
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut stack = Vec::new();
    let mut queue = VecDeque::new();
    for source in 0..sources {
        dist[source] = 0;
        sigma[source] = 1.0;
        queue.push_back(source);
        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let (d_v, sigma_v) = (dist[v], sigma[v]);
            for &w in &successors[v] {
                if dist[w] == usize::MAX {
                    dist[w] = d_v + 1;
                    queue.push_back(w);
                }
                if dist[w] == d_v + 1 {
                    sigma[w] += sigma_v;
                    predecessors[w].push(v);
                }
            }
        }
        // Popped in non-increasing distance, so no node still on the stack
        // reads a popped one: each slot is reset for the next source as it goes.
        while let Some(w) = stack.pop() {
            if w != source {
                let delta_w = delta[w];
                let coeff = (1.0 + delta_w) / sigma[w];
                for &v in &predecessors[w] {
                    delta[v] += sigma[v] * coeff;
                }
                centrality[w] += delta_w;
            }
            dist[w] = usize::MAX;
            sigma[w] = 0.0;
            delta[w] = 0.0;
            predecessors[w].clear();
        }
    }

    let max = centrality.iter().copied().fold(0.0_f64, f64::max);
    if max > 0.0 {
        for val in &mut centrality {
            *val /= max;
        }
    }
    ids.into_iter().zip(centrality).collect()
}

/// Strongly connected components over `admit`, by iterative Kosaraju.
///
/// Nodes outside `admit` are invisible: edges into or through them are not
/// followed. Components come in Kosaraju topological order — a component
/// precedes every component it reaches — each with sorted members, so a
/// caller may index them while the graph and `admit` are unchanged.
#[must_use]
pub fn scc<G: Neighbors>(graph: &G, admit: &HashSet<G::NodeId>) -> Vec<Vec<G::NodeId>> {
    let mut starts: Vec<G::NodeId> = admit.iter().copied().collect();
    starts.sort_unstable();
    let finish_order = finish_order(graph, admit, &starts);

    let mut seen = HashSet::new();
    let mut components = Vec::new();
    for &start in finish_order.iter().rev() {
        if !seen.insert(start) {
            continue;
        }
        let mut members = Vec::new();
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            members.push(node);
            for pred in graph.neighbors(node, Direction::Reverse) {
                if admit.contains(&pred) && seen.insert(pred) {
                    stack.push(pred);
                }
            }
        }
        members.sort_unstable();
        components.push(members);
    }
    components
}

/// Forward depth-first finish order from each start in turn, never revisiting.
fn finish_order<G: Neighbors>(
    graph: &G,
    admit: &HashSet<G::NodeId>,
    starts: &[G::NodeId],
) -> Vec<G::NodeId> {
    let mut seen = HashSet::new();
    let mut order = Vec::with_capacity(starts.len());
    for &start in starts {
        if !seen.insert(start) {
            continue;
        }
        let mut stack = vec![(start, graph.neighbors(start, Direction::Forward))];
        while let Some((node, pending)) = stack.last_mut() {
            match pending.find(|n| admit.contains(n) && seen.insert(*n)) {
                Some(next) => stack.push((next, graph.neighbors(next, Direction::Forward))),
                None => {
                    order.push(*node);
                    stack.pop();
                }
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Digraph {
        succ: BTreeMap<u32, Vec<u32>>,
        pred: BTreeMap<u32, Vec<u32>>,
    }

    impl Digraph {
        fn new(edges: &[(u32, u32)]) -> Self {
            let mut succ: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            let mut pred: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for &(s, t) in edges {
                succ.entry(s).or_default().push(t);
                pred.entry(t).or_default().push(s);
                succ.entry(t).or_default();
                pred.entry(s).or_default();
            }
            Self { succ, pred }
        }
    }

    impl Neighbors for Digraph {
        type NodeId = u32;
        fn neighbors(&self, id: u32, direction: Direction) -> impl Iterator<Item = u32> + '_ {
            let side = match direction {
                Direction::Forward => &self.succ,
                Direction::Reverse => &self.pred,
            };
            side.get(&id).into_iter().flatten().copied()
        }
        fn node_ids(&self) -> impl Iterator<Item = u32> + '_ {
            self.succ.keys().copied()
        }
    }

    fn all(g: &Digraph) -> HashSet<u32> {
        g.node_ids().collect()
    }

    #[test]
    fn reachable_excludes_origin_and_stops_at_a_refused_node() {
        let g = Digraph::new(&[(1, 2), (2, 9), (9, 3), (2, 4), (4, 1)]);
        let all = bfs_reachable(&g, 1, Direction::Forward, |_| true);
        assert_eq!(all, BTreeSet::from([2, 3, 4, 9]));
        let admitted = bfs_reachable(&g, 1, Direction::Forward, |n| n != 9);
        assert_eq!(admitted, BTreeSet::from([2, 4]));
        let upstream = bfs_reachable(&g, 3, Direction::Reverse, |_| true);
        assert_eq!(upstream, BTreeSet::from([1, 2, 4, 9]));
    }

    #[test]
    fn layers_group_by_first_hop_distance_and_cap_at_max_depth() {
        let g = Digraph::new(&[(1, 2), (1, 3), (2, 4), (3, 4), (4, 5), (4, 5)]);
        let layers = bfs_layers(&g, 1, Direction::Forward, 10, |_| true);
        assert_eq!(layers, vec![vec![2, 3], vec![4], vec![5]]);
        let capped = bfs_layers(&g, 1, Direction::Forward, 2, |_| true);
        assert_eq!(capped, vec![vec![2, 3], vec![4]]);
        let refused = bfs_layers(&g, 1, Direction::Forward, 10, |n| n != 4);
        assert_eq!(refused, vec![vec![2, 3]]);
    }

    #[test]
    fn centrality_peaks_at_the_interior_of_a_chain_and_ignores_parallel_edges() {
        let g = Digraph::new(&[(1, 2), (2, 3), (2, 4)]);
        let c = betweenness_centrality(&g);
        assert_eq!(c[&2], 1.0);
        assert_eq!(c[&1], 0.0);
        assert_eq!(c[&3], 0.0);
        assert_eq!(c[&4], 0.0);

        let doubled = Digraph::new(&[(1, 2), (1, 2), (2, 3), (2, 4), (2, 4)]);
        assert_eq!(betweenness_centrality(&doubled), c);
    }

    #[test]
    fn centrality_splits_evenly_across_equal_shortest_paths() {
        let g = Digraph::new(&[(1, 2), (1, 3), (2, 4), (3, 4)]);
        let c = betweenness_centrality(&g);
        assert_eq!(c[&2], 1.0);
        assert_eq!(c[&3], 1.0);
        assert_eq!(c[&1], 0.0);
        assert_eq!(c[&4], 0.0);
        assert!(betweenness_centrality(&Digraph::new(&[])).is_empty());
    }

    #[test]
    fn cycle_is_one_component_with_sorted_members() {
        let g = Digraph::new(&[(3, 1), (1, 2), (2, 3)]);
        assert_eq!(scc(&g, &all(&g)), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn chain_yields_singletons_in_topological_order() {
        let g = Digraph::new(&[(1, 2), (2, 3)]);
        assert_eq!(scc(&g, &all(&g)), vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn components_precede_the_components_they_reach() {
        let g = Digraph::new(&[(2, 1), (1, 2), (2, 3), (3, 4), (4, 3)]);
        assert_eq!(scc(&g, &all(&g)), vec![vec![1, 2], vec![3, 4]]);
    }

    #[test]
    fn unadmitted_node_breaks_the_path_through_it() {
        let g = Digraph::new(&[(1, 9), (9, 2), (2, 1)]);
        let admit = HashSet::from([1, 2]);
        assert_eq!(scc(&g, &admit), vec![vec![2], vec![1]]);
    }

    #[test]
    fn adjacency_index_agrees_with_a_host_graph_and_with_components() {
        use crate::graph::{EdgeSpec, LatticeGraph};
        use crate::query::{AdjacencyIndex, components};
        use crate::types::Provenance;
        use serde_json::Map;

        let names = ["a", "b", "c"];
        let edges = [(1, 2), (2, 1), (2, 3), (3, 3)];
        let mut g = LatticeGraph::new();
        for id in names {
            g.add_node(id, "k", Map::new(), Provenance::new("t", 1))
                .unwrap();
        }
        for (s, t) in edges {
            let spec = EdgeSpec {
                src: names[s - 1].into(),
                tgt: names[t - 1].into(),
                kind: "e".into(),
                attrs: Map::new(),
            };
            g.add_edge(spec, Provenance::new("t", 1));
        }
        let index = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = names.into();
        let from_index = scc(&index, &admit);

        let host = Digraph::new(&edges.map(|(s, t)| (s as u32, t as u32)));
        let from_host: Vec<Vec<&str>> = scc(&host, &all(&host))
            .into_iter()
            .map(|c| c.into_iter().map(|n| names[n as usize - 1]).collect())
            .collect();
        assert_eq!(from_index, from_host);

        let condensation = components(&index, &admit);
        let from_components: Vec<Vec<&str>> = condensation
            .components
            .iter()
            .map(|c| c.members.iter().map(String::as_str).collect())
            .collect();
        assert_eq!(from_index, from_components);
    }

    #[test]
    fn parallel_edges_and_self_loops_do_not_duplicate_members() {
        let g = Digraph::new(&[(1, 2), (1, 2), (2, 1), (1, 1)]);
        assert_eq!(scc(&g, &all(&g)), vec![vec![1, 2]]);
    }

    mod properties {
        use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

        use proptest::prelude::*;

        use super::{Digraph, all};
        use crate::algo::{Neighbors, betweenness_centrality, bfs_reachable, scc};
        use crate::query::Direction;

        /// A small digraph with self-loops, parallel edges and cycles all possible.
        fn edges() -> impl Strategy<Value = Vec<(u32, u32)>> {
            prop::collection::vec((0u32..7, 0u32..7), 1..14)
        }

        /// Transitive closure by repeated relaxation: `reach[u][v]` when a path of one or
        /// more edges joins them. Nodes are `0..7`, present or not.
        fn closure(edges: &[(u32, u32)]) -> [[bool; 7]; 7] {
            let mut reach = [[false; 7]; 7];
            for &(s, t) in edges {
                reach[s as usize][t as usize] = true;
            }
            for k in 0..7 {
                for i in 0..7 {
                    for j in 0..7 {
                        if reach[i][k] && reach[k][j] {
                            reach[i][j] = true;
                        }
                    }
                }
            }
            reach
        }

        /// `inner` with one node withheld from `node_ids` but still a neighbor.
        struct Hiding(Digraph, u32);

        impl Neighbors for Hiding {
            type NodeId = u32;
            fn neighbors(&self, id: u32, direction: Direction) -> impl Iterator<Item = u32> + '_ {
                self.0.neighbors(id, direction)
            }
            fn node_ids(&self) -> impl Iterator<Item = u32> + '_ {
                self.0.node_ids().filter(|&id| id != self.1)
            }
        }

        /// Each score's bits, ordered: `==` on `f64` would let `-0.0` pass for `0.0`.
        fn bits(scores: HashMap<u32, f64>) -> BTreeMap<u32, u64> {
            scores
                .into_iter()
                .map(|(id, score)| (id, score.to_bits()))
                .collect()
        }

        /// Brandes over hashed maps, as it stood before the dense slots: the oracle
        /// the dense walk must match to the bit.
        fn hashed_centrality<G: Neighbors>(graph: &G) -> HashMap<G::NodeId, f64> {
            let mut nodes: Vec<G::NodeId> = graph.node_ids().collect();
            nodes.sort_unstable();
            nodes.dedup();
            let mut centrality: HashMap<G::NodeId, f64> =
                nodes.iter().map(|&id| (id, 0.0)).collect();
            for &source in &nodes {
                let mut stack = Vec::new();
                let mut predecessors: HashMap<G::NodeId, Vec<G::NodeId>> = HashMap::new();
                let mut sigma: HashMap<G::NodeId, f64> = HashMap::from([(source, 1.0)]);
                let mut dist: HashMap<G::NodeId, usize> = HashMap::from([(source, 0)]);
                let mut queue = VecDeque::from([source]);
                while let Some(v) = queue.pop_front() {
                    stack.push(v);
                    let d_v = dist[&v];
                    let sigma_v = sigma[&v];
                    let successors: BTreeSet<G::NodeId> =
                        graph.neighbors(v, Direction::Forward).collect();
                    for w in successors {
                        let d_w = *dist.entry(w).or_insert_with(|| {
                            queue.push_back(w);
                            d_v + 1
                        });
                        if d_w == d_v + 1 {
                            *sigma.entry(w).or_insert(0.0) += sigma_v;
                            predecessors.entry(w).or_default().push(v);
                        }
                    }
                }
                let mut delta: HashMap<G::NodeId, f64> = HashMap::new();
                while let Some(w) = stack.pop() {
                    if w == source {
                        continue;
                    }
                    let delta_w = delta.get(&w).copied().unwrap_or(0.0);
                    let coeff = (1.0 + delta_w) / sigma[&w];
                    for &v in predecessors.get(&w).into_iter().flatten() {
                        *delta.entry(v).or_insert(0.0) += sigma[&v] * coeff;
                    }
                    *centrality.entry(w).or_insert(0.0) += delta_w;
                }
            }
            let max = centrality.values().copied().fold(0.0_f64, f64::max);
            if max > 0.0 {
                for val in centrality.values_mut() {
                    *val /= max;
                }
            }
            centrality
        }

        proptest! {
            /// Score every node to the same bits as the hashed walk, a withheld
            /// neighbor included: the dense slots change where values live, not
            /// the order any of them is summed in.
            #[test]
            fn centrality_matches_the_hashed_walk(edges in edges(), hide in 0u32..8) {
                let g = Digraph::new(&edges);
                prop_assert_eq!(bits(betweenness_centrality(&g)), bits(hashed_centrality(&g)));
                let hiding = Hiding(Digraph::new(&edges), hide);
                prop_assert_eq!(
                    bits(betweenness_centrality(&hiding)),
                    bits(hashed_centrality(&hiding))
                );
            }

            /// Reach, in either direction, exactly the nodes the transitive closure joins
            /// to the origin, never the origin itself: the walk agrees with a reference
            /// that does no walking.
            #[test]
            fn reachable_matches_the_transitive_closure(edges in edges(), pick in 0usize..7) {
                let g = Digraph::new(&edges);
                let nodes: Vec<u32> = g.node_ids().collect();
                let origin = nodes[pick % nodes.len()];
                let reach = closure(&edges);
                let forward: BTreeSet<u32> = nodes
                    .iter()
                    .copied()
                    .filter(|&v| v != origin && reach[origin as usize][v as usize])
                    .collect();
                let reverse: BTreeSet<u32> = nodes
                    .iter()
                    .copied()
                    .filter(|&v| v != origin && reach[v as usize][origin as usize])
                    .collect();
                prop_assert_eq!(bfs_reachable(&g, origin, Direction::Forward, |_| true), forward);
                prop_assert_eq!(bfs_reachable(&g, origin, Direction::Reverse, |_| true), reverse);
            }

            /// Partition the nodes exactly as mutual reachability does: two nodes share
            /// a component when each reaches the other, and every node lands in one.
            #[test]
            fn scc_matches_mutual_reachability(edges in edges()) {
                let g = Digraph::new(&edges);
                let reach = closure(&edges);
                let nodes: Vec<u32> = g.node_ids().collect();
                let expected: BTreeSet<Vec<u32>> = nodes
                    .iter()
                    .map(|&u| {
                        nodes
                            .iter()
                            .copied()
                            .filter(|&v| {
                                u == v || (reach[u as usize][v as usize] && reach[v as usize][u as usize])
                            })
                            .collect()
                    })
                    .collect();
                let found = scc(&g, &all(&g));
                let members: usize = found.iter().map(Vec::len).sum();
                prop_assert_eq!(members, nodes.len());
                let found: BTreeSet<Vec<u32>> = found
                    .into_iter()
                    .map(|mut component| {
                        component.sort_unstable();
                        component
                    })
                    .collect();
                prop_assert_eq!(found, expected);
            }

            /// Keep a node out of the walk when `enter` refuses it, and out of every path
            /// through it: the reachable set equals the closure of the subgraph without
            /// that node. The origin is walked from even when it is the refused node.
            #[test]
            fn refused_nodes_block_paths(edges in edges(), pick in 0usize..7, refused in 0u32..7) {
                let g = Digraph::new(&edges);
                let nodes: Vec<u32> = g.node_ids().collect();
                let origin = nodes[pick % nodes.len()];
                let admitted: Vec<(u32, u32)> = edges
                    .iter()
                    .copied()
                    .filter(|&(s, t)| (s != refused || s == origin) && t != refused)
                    .collect();
                let reach = closure(&admitted);
                let expected: BTreeSet<u32> = nodes
                    .iter()
                    .copied()
                    .filter(|&v| v != origin && v != refused && reach[origin as usize][v as usize])
                    .collect();
                let walked = bfs_reachable(&g, origin, Direction::Forward, |n| n != refused);
                prop_assert_eq!(walked, expected);
            }
        }
    }
}
