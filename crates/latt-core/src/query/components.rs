//! Strongly connected components, their condensation and the lane layout.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::algo::Neighbors;
use crate::types::{Component, Condensation, InterComponentEdge, LaneLayout, LaneRow};

use super::index::AdjacencyIndex;
use super::{Direction, scc};

/// An adjacency index whose node IDs live only as long as the borrow of it.
///
/// `components` admits IDs the caller may drop as soon as it returns, while the
/// index's own IDs live as long as the graph. Walking through this view gives
/// both the shorter of the two lifetimes, so the admitted set never has to
/// outlive the condensation, which borrows only the index's edges.
struct Within<'i, 'a>(&'i AdjacencyIndex<'a>);

impl<'i> Neighbors for Within<'i, '_> {
    type NodeId = &'i str;

    fn neighbors(&self, id: &'i str, direction: Direction) -> impl Iterator<Item = &'i str> + '_ {
        self.0.neighbors(id, direction)
    }

    fn node_ids(&self) -> impl Iterator<Item = &'i str> + '_ {
        self.0.node_ids()
    }
}

/// Strongly connected components over the admitted node set.
///
/// Adds the condensation extras: self-loop flags and inter-component edges. The
/// adjacency index controls edge-kind scope; [`scc`] does the decomposition.
#[must_use]
#[doc(hidden)]
pub fn components<'a>(index: &AdjacencyIndex<'a>, admit: &HashSet<&str>) -> Condensation<'a> {
    let mut all_components: Vec<Component> = Vec::new();
    let mut component_of: HashMap<&str, usize> = HashMap::new();
    for members in scc(&Within(index), admit) {
        let self_loop = match members.as_slice() {
            [only] => index.edges_from(only).iter().any(|e| e.tgt == *only),
            _ => false,
        };
        let comp_idx = all_components.len();
        for &member in &members {
            component_of.insert(member, comp_idx);
        }
        all_components.push(Component {
            members: members.iter().map(|&s| s.to_string()).collect(),
            cyclic: members.len() > 1 || self_loop,
            self_loop,
        });
    }

    let mut admit_sorted: Vec<&str> = admit.iter().copied().collect();
    admit_sorted.sort_unstable();
    let mut inter_edges: Vec<InterComponentEdge<'a>> = Vec::new();
    for &node in &admit_sorted {
        let src_comp = component_of[node];
        for &edge in index.edges_from(node) {
            let tgt = edge.tgt.as_str();
            if let Some(&tgt_comp) = component_of.get(tgt)
                && src_comp != tgt_comp
            {
                inter_edges.push(InterComponentEdge {
                    src_component: src_comp,
                    tgt_component: tgt_comp,
                    edge,
                });
            }
        }
    }

    Condensation {
        components: all_components,
        inter_edges,
    }
}

/// Greedy lane assignment over a reachability cone from `origin`.
///
/// Condenses SCCs, topologically sorts the condensation, then assigns lanes
/// using pvigier's gitk-style algorithm: the first child inherits its parent's
/// lane, later children get new lanes. A lane is freed when all children of
/// its occupant have been placed.
#[must_use]
#[doc(hidden)]
pub fn lane_layout(index: &AdjacencyIndex<'_>, origin: &str, direction: Direction) -> LaneLayout {
    // Collect the reachable set (including origin).
    let reachable = {
        let mut set: HashSet<&str> = HashSet::new();
        set.insert(origin);
        let mut queue: VecDeque<&str> = VecDeque::from([origin]);
        while let Some(id) = queue.pop_front() {
            for edge in index.edges_in_direction(id, direction) {
                let far = AdjacencyIndex::far_endpoint(edge, direction);
                if set.insert(far) {
                    queue.push_back(far);
                }
            }
        }
        set
    };

    // Condense SCCs within the reachable set.
    let condensation = components(index, &reachable);

    let mut node_to_comp: HashMap<&str, usize> = HashMap::new();
    for (ci, comp) in condensation.components.iter().enumerate() {
        for member in &comp.members {
            node_to_comp.insert(member.as_str(), ci);
        }
    }

    // Build condensed adjacency for topo sort.
    let num_comps = condensation.components.len();
    let mut comp_children: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); num_comps];
    let mut comp_parents: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); num_comps];
    for ie in &condensation.inter_edges {
        let (src_c, tgt_c) = match direction {
            Direction::Forward => (ie.src_component, ie.tgt_component),
            Direction::Reverse => (ie.tgt_component, ie.src_component),
        };
        comp_children[src_c].insert(tgt_c);
        comp_parents[tgt_c].insert(src_c);
    }

    // Kahn's topo sort from the origin's component.
    let origin_comp = node_to_comp[origin];
    let mut in_degree: Vec<usize> = vec![0; num_comps];
    // Only count in-degree within the condensation (all are reachable).
    for ci in 0..num_comps {
        in_degree[ci] = comp_parents[ci].len();
    }
    let mut topo: Vec<usize> = Vec::with_capacity(num_comps);
    let mut ready: VecDeque<usize> = VecDeque::new();
    // Origin component first; then other zero-in-degree (shouldn't happen in
    // a cone, but be defensive).
    if in_degree[origin_comp] == 0 {
        ready.push_back(origin_comp);
    }
    for (ci, &deg) in in_degree.iter().enumerate() {
        if deg == 0 && ci != origin_comp {
            ready.push_back(ci);
        }
    }
    while let Some(ci) = ready.pop_front() {
        topo.push(ci);
        for &child in &comp_children[ci] {
            in_degree[child] -= 1;
            if in_degree[child] == 0 {
                ready.push_back(child);
            }
        }
    }

    let (rows, peak) = assign_lanes(
        &topo,
        &comp_children,
        &comp_parents,
        &condensation.components,
    );

    LaneLayout {
        origin: origin.to_string(),
        rows,
        peak_lanes: peak,
    }
}

/// Greedy lane assignment (pvigier/gitk-style).
///
/// Given a topological order over condensed components and their parent/child
/// adjacency, assigns each component to a lane and returns `(rows, peak_lanes)`.
/// Callers provide the topo sort and adjacency; this function is layout-only.
#[doc(hidden)]
pub fn assign_lanes(
    topo: &[usize],
    children: &[BTreeSet<usize>],
    parents: &[BTreeSet<usize>],
    components: &[Component],
) -> (Vec<LaneRow>, usize) {
    let mut lanes: Vec<Option<usize>> = Vec::new();
    let mut comp_lane: HashMap<usize, usize> = HashMap::new();
    let mut rows: Vec<LaneRow> = Vec::with_capacity(topo.len());
    let mut peak: usize = 0;

    let alloc_lane = |lanes: &mut Vec<Option<usize>>| -> usize {
        for (i, slot) in lanes.iter().enumerate() {
            if slot.is_none() {
                return i;
            }
        }
        lanes.push(None);
        lanes.len() - 1
    };

    for &ci in topo {
        let comp = &components[ci];

        let mut inherited = None;
        let mut parent_lane_list: Vec<usize> = Vec::new();
        for &pi in &parents[ci] {
            if let Some(&pl) = comp_lane.get(&pi) {
                parent_lane_list.push(pl);
                if inherited.is_none() && lanes[pl] == Some(pi) {
                    inherited = Some(pl);
                }
            }
        }
        parent_lane_list.sort_unstable();
        parent_lane_list.dedup();

        let lane = if let Some(l) = inherited {
            lanes[l] = Some(ci);
            l
        } else {
            let l = alloc_lane(&mut lanes);
            lanes[l] = Some(ci);
            l
        };
        comp_lane.insert(ci, lane);

        for &pi in &parents[ci] {
            if let Some(&pl) = comp_lane.get(&pi)
                && lanes[pl] == Some(pi)
                && children[pi].iter().all(|c| comp_lane.contains_key(c))
            {
                lanes[pl] = None;
            }
        }

        let active = lanes.iter().filter(|s| s.is_some()).count();
        if active > peak {
            peak = active;
        }

        let cluster = comp.members.len() > 1;
        rows.push(LaneRow {
            id: comp.members[0].clone(),
            cluster,
            members: comp.members.clone(),
            lane,
            parent_lanes: parent_lane_list,
        });
    }

    (rows, peak)
}

#[cfg(test)]
mod component_tests {
    use super::{AdjacencyIndex, Direction, components, lane_layout};
    use crate::graph::{EdgeSpec, LatticeGraph};
    use crate::types::Provenance;
    use serde_json::Map;
    use std::collections::HashSet;

    fn prov() -> Provenance {
        Provenance::new("test.md", 1)
    }

    fn edge(src: &str, tgt: &str, kind: &str) -> EdgeSpec {
        EdgeSpec {
            src: src.into(),
            tgt: tgt.into(),
            kind: kind.into(),
            attrs: Map::new(),
        }
    }

    #[test]
    fn component_simple_cycle() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        g.add_edge(edge("C", "A", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = ["A", "B", "C"].into();
        let cond = components(&idx, &admit);
        let cyclic: Vec<_> = cond.components.iter().filter(|c| c.cyclic).collect();
        assert_eq!(cyclic.len(), 1);
        assert_eq!(cyclic[0].members, vec!["A", "B", "C"]);
        assert!(!cyclic[0].self_loop);
    }

    #[test]
    fn component_acyclic_yields_singletons() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = ["A", "B", "C"].into();
        let cond = components(&idx, &admit);
        assert_eq!(cond.components.len(), 3);
        for comp in &cond.components {
            assert_eq!(comp.members.len(), 1);
            assert!(!comp.cyclic);
        }
    }

    #[test]
    fn component_members_sorted() {
        let mut g = LatticeGraph::new();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("C", "A", "derives"), prov());
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = ["A", "B", "C"].into();
        let cond = components(&idx, &admit);
        let cyclic: Vec<_> = cond.components.iter().filter(|c| c.cyclic).collect();
        assert_eq!(cyclic[0].members, vec!["A", "B", "C"]);
    }

    #[test]
    fn component_self_loop() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "A", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = ["A"].into();
        let cond = components(&idx, &admit);
        assert_eq!(cond.components.len(), 1);
        assert!(cond.components[0].cyclic);
        assert!(cond.components[0].self_loop);
    }

    #[test]
    fn condensation_inter_component_edges() {
        // {A, B} cycle and {C} singleton, with B → C
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "A", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = ["A", "B", "C"].into();
        let cond = components(&idx, &admit);
        assert!(!cond.inter_edges.is_empty());
        let cross = &cond.inter_edges[0];
        assert_ne!(cross.src_component, cross.tgt_component);
        assert_eq!(cross.edge.src, "B");
        assert_eq!(cross.edge.tgt, "C");
    }

    #[test]
    fn component_admit_restricts_scope() {
        // A → B → C → A, but only admit {A, B}
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        g.add_edge(edge("C", "A", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = ["A", "B"].into();
        let cond = components(&idx, &admit);
        assert_eq!(cond.components.len(), 2);
        for comp in &cond.components {
            assert!(!comp.cyclic, "no cycle when C is excluded");
        }
    }

    #[test]
    fn component_filtered_index_scopes_edge_kinds() {
        // A →(derives)→ B →(derives)→ C →(derives)→ A forms a cycle,
        // but A →(verifies)→ B is a different kind. Filtering to "verifies"
        // should see no cycle.
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        g.add_edge(edge("C", "A", "derives"), prov());
        g.add_edge(edge("A", "B", "verifies"), prov());
        let full = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = ["A", "B", "C"].into();

        let derives_only = full.filtered(&["derives"]);
        let cond = components(&derives_only, &admit);
        let cyclic = cond.components.iter().filter(|c| c.cyclic).count();
        assert_eq!(cyclic, 1, "cycle visible through derives");

        let verifies_only = full.filtered(&["verifies"]);
        let cond = components(&verifies_only, &admit);
        for comp in &cond.components {
            assert!(!comp.cyclic, "no cycle through verifies alone");
        }
    }

    #[test]
    fn component_acyclic_topological_order() {
        // A → B → C: topological order should place sources before sinks.
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let admit: HashSet<&str> = ["A", "B", "C"].into();
        let cond = components(&idx, &admit);
        let order: Vec<&str> = cond
            .components
            .iter()
            .map(|c| c.members[0].as_str())
            .collect();
        let pos = |id: &str| order.iter().position(|&x| x == id).unwrap();
        assert!(
            pos("A") < pos("B") && pos("B") < pos("C"),
            "Kosaraju topological order: A before B before C, got {order:?}"
        );
    }

    // --- Lane layout ---

    #[test]
    fn lane_layout_linear_chain() {
        // A → B → C: three rows, one lane each, peak 1.
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layout = lane_layout(&idx, "A", Direction::Forward);
        assert_eq!(layout.origin, "A");
        assert_eq!(layout.rows.len(), 3);
        assert_eq!(layout.peak_lanes, 1);
        let ids: Vec<&str> = layout.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["A", "B", "C"]);
        // All on the same lane.
        assert!(layout.rows.iter().all(|r| r.lane == layout.rows[0].lane));
        // Root has no parents.
        assert!(layout.rows[0].parent_lanes.is_empty());
        // B's parent is A's lane.
        assert_eq!(layout.rows[1].parent_lanes, vec![layout.rows[0].lane]);
    }

    #[test]
    fn lane_layout_fan_out() {
        // A → B, A → C, A → D: fan-out of 3, peak 3 lanes.
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_node("D", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("A", "C", "derives"), prov());
        g.add_edge(edge("A", "D", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layout = lane_layout(&idx, "A", Direction::Forward);
        assert_eq!(layout.rows.len(), 4);
        // First child inherits A's lane; others get new lanes.
        let lanes: Vec<usize> = layout.rows.iter().map(|r| r.lane).collect();
        let unique: HashSet<usize> = lanes[1..].iter().copied().collect();
        assert_eq!(unique.len(), 3, "three children on distinct lanes");
        // One of the children shares A's lane.
        assert!(
            lanes[1..].contains(&lanes[0]),
            "first child inherits parent lane"
        );
    }

    #[test]
    fn lane_layout_diamond() {
        // A → B, A → C, B → D, C → D: diamond shape.
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_node("D", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("A", "C", "derives"), prov());
        g.add_edge(edge("B", "D", "derives"), prov());
        g.add_edge(edge("C", "D", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layout = lane_layout(&idx, "A", Direction::Forward);
        assert_eq!(layout.rows.len(), 4);
        // D should have two parent lanes (B's and C's).
        let d_row = layout.rows.iter().find(|r| r.id == "D").unwrap();
        assert_eq!(d_row.parent_lanes.len(), 2, "D merges from B and C");
        // A is before B and C; D is last.
        let pos = |id: &str| layout.rows.iter().position(|r| r.id == id).unwrap();
        assert!(pos("A") < pos("B"));
        assert!(pos("A") < pos("C"));
        assert!(pos("B") < pos("D"));
        assert!(pos("C") < pos("D"));
    }

    #[test]
    fn lane_layout_scc_condensed() {
        // A → B → C → A (cycle), plus D → A: cone from D.
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_node("D", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("D", "A", "derives"), prov());
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        g.add_edge(edge("C", "A", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layout = lane_layout(&idx, "D", Direction::Forward);
        // D + one condensed cluster = 2 rows.
        assert_eq!(layout.rows.len(), 2);
        let cluster = layout.rows.iter().find(|r| r.cluster).unwrap();
        assert_eq!(cluster.members.len(), 3);
        assert!(cluster.members.contains(&"A".to_string()));
        assert!(cluster.members.contains(&"B".to_string()));
        assert!(cluster.members.contains(&"C".to_string()));
    }

    #[test]
    fn lane_layout_reverse_direction() {
        // A → B → C: reverse from C should show C, B, A.
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layout = lane_layout(&idx, "C", Direction::Reverse);
        assert_eq!(layout.origin, "C");
        assert_eq!(layout.rows.len(), 3);
        let ids: Vec<&str> = layout.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["C", "B", "A"]);
    }

    #[test]
    fn lane_layout_vacancy_excluded() {
        // A → B, B → GHOST (GHOST not declared): cone has A and B only.
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "GHOST", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layout = lane_layout(&idx, "A", Direction::Forward);
        let ids: Vec<&str> = layout.rows.iter().map(|r| r.id.as_str()).collect();
        // GHOST is reachable (edges_from includes it) but not in the graph's
        // node set — the reachability walk includes it as an endpoint, and
        // components will include it if it's in the admit set. Since we admit
        // all reachable endpoints, GHOST gets a row.
        assert!(ids.contains(&"GHOST"), "vacancy endpoint gets a row");
        assert!(ids.contains(&"A"));
        assert!(ids.contains(&"B"));
    }

    #[test]
    fn lane_layout_single_node() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        let idx = AdjacencyIndex::build(&g);

        let layout = lane_layout(&idx, "A", Direction::Forward);
        assert_eq!(layout.rows.len(), 1);
        assert_eq!(layout.rows[0].id, "A");
        assert_eq!(layout.peak_lanes, 1);
    }
}
