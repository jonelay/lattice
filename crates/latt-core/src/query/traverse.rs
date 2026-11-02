//! Reachability and ordered traversal over an adjacency index.

use std::collections::{BTreeSet, HashSet};

use crate::graph::{Edge, LatticeGraph};
use crate::profile::{Condition, Profile};
use crate::types::{Endpoint, NodeRef, ReachReport, TraversalReport, TraversalStep, Visit};
use crate::validate::eval_conditions;

use super::index::{AdjacencyIndex, SlotIndex, check_edge_kinds, node_slot};
use super::{Direction, QueryError, bfs_reachable};

/// The transitive closure from `origin`, excluding the origin itself.
///
/// `filters` narrow it after the walk. A filter selects from the closure; it never
/// cuts the traversal, so a node behind a non-matching one is still reached.
///
/// The walk follows edge endpoints as plain strings, so it traverses *through*
/// an endpoint no adapter declared; only declared nodes enter the answer.
/// An undeclared one is a dangling reference, not a phantom node.
///
/// With `check_resolved`, a second walk admits declared endpoints only. A
/// node the first walk reaches and the second does not is tainted: every
/// path to it crosses a vacancy. Without the flag the second walk is skipped
/// and `tainted` stays `None`.
///
/// # Errors
///
/// [`QueryError`] when `origin` names no declared node or an edge kind is not
/// one the profile declares.
pub fn reach(
    graph: &LatticeGraph,
    profile: &Profile,
    origin: &str,
    edge_kinds: &[&str],
    direction: Direction,
    filters: &[Condition],
    check_resolved: bool,
) -> Result<ReachReport, QueryError> {
    let origin_slot = node_slot(graph, origin)?;
    check_edge_kinds(profile, edge_kinds)?;

    let index = SlotIndex::build(graph, edge_kinds, direction);
    let seen = index.reached(origin_slot, |_| true);
    let clean = check_resolved.then(|| index.reached(origin_slot, |slot| slot < index.declared));
    // Only declared nodes enter the answer, ordered by ID.
    let mut reached: Vec<(usize, &crate::graph::Node)> = (0..index.declared)
        .filter(|&slot| seen[slot])
        .map(|slot| (slot, graph.node_at(slot)))
        .collect();
    reached.sort_unstable_by(|a, b| a.1.id.cmp(&b.1.id));

    Ok(ReachReport {
        origin: origin.to_string(),
        direction,
        edge_kinds: edge_kinds
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        nodes: reached
            .into_iter()
            .filter(|(_, node)| eval_conditions(filters, &node.attrs, true))
            .map(|(slot, n)| NodeRef {
                id: n.id.clone(),
                kind: n.kind.clone(),
                tainted: clean.as_ref().map(|clean| !clean[slot]),
            })
            .collect(),
    })
}

/// Depth-first traversal from a root node, yielding topology-preserving steps.
///
/// Taint is computed eagerly from a declared-only BFS closure before the walk
/// begins. A node the DFS visits but the closure does not reach is tainted.
/// The caller controls edge-kind filtering by passing a pre-filtered
/// `AdjacencyIndex`.
///
/// # Errors
///
/// [`QueryError`] when `root` names no declared node.
#[doc(hidden)]
pub fn traverse<'a>(
    graph: &'a LatticeGraph,
    index: &AdjacencyIndex<'a>,
    root: &str,
    direction: Direction,
) -> Result<TraversalReport<'a>, QueryError> {
    let root_id = graph.node_at(node_slot(graph, root)?).id.as_str();

    let clean_reachable = bfs_reachable(index, root_id, direction, |id| graph.has_node(id));

    let mut steps = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut path_set: HashSet<&str> = HashSet::new();

    enum Action<'a> {
        Enter {
            node_id: &'a str,
            depth: usize,
            edge: Option<&'a Edge>,
            parallel_ordinal: usize,
            parallel_count: usize,
        },
        Leave {
            node_id: &'a str,
        },
    }

    let mut stack: Vec<Action<'_>> = vec![Action::Enter {
        node_id: root_id,
        depth: 0,
        edge: None,
        parallel_ordinal: 0,
        parallel_count: 1,
    }];

    while let Some(action) = stack.pop() {
        match action {
            Action::Leave { node_id } => {
                path_set.remove(node_id);
            }
            Action::Enter {
                node_id,
                depth,
                edge,
                parallel_ordinal,
                parallel_count,
            } => {
                let visit = if path_set.contains(node_id) {
                    Visit::Cycle
                } else if visited.contains(node_id) {
                    Visit::Reconvergence
                } else {
                    Visit::First
                };

                let endpoint = if graph.has_node(node_id) {
                    Endpoint::Declared
                } else {
                    Endpoint::Vacancy
                };

                let tainted = if node_id == root_id {
                    false
                } else {
                    !clean_reachable.contains(node_id)
                };

                steps.push(TraversalStep {
                    depth,
                    edge,
                    node_id,
                    endpoint,
                    visit,
                    parallel_ordinal,
                    parallel_count,
                    tainted,
                });

                if visit == Visit::First {
                    visited.insert(node_id);
                    path_set.insert(node_id);
                    stack.push(Action::Leave { node_id });

                    let edges = index.edges_in_direction(node_id, direction);

                    // The slice is sorted by far endpoint, so each run of one far
                    // endpoint is a group of parallel edges.
                    let groups = edges.chunk_by(|a, b| {
                        AdjacencyIndex::far_endpoint(a, direction)
                            == AdjacencyIndex::far_endpoint(b, direction)
                    });

                    // Push children in reverse so the first child is processed first.
                    for group in groups.rev() {
                        let count = group.len();
                        for (ordinal, &edge) in group.iter().enumerate().rev() {
                            stack.push(Action::Enter {
                                node_id: AdjacencyIndex::far_endpoint(edge, direction),
                                depth: depth + 1,
                                edge: Some(edge),
                                parallel_ordinal: ordinal,
                                parallel_count: count,
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(TraversalReport {
        root: root_id,
        direction,
        steps,
    })
}

#[cfg(test)]
mod traversal_tests {
    use super::{AdjacencyIndex, Direction, traverse};
    use crate::graph::{EdgeSpec, LatticeGraph};
    use crate::types::{Endpoint, Provenance, Visit};
    use serde_json::Map;

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
    fn traversal_root_step() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        assert_eq!(report.steps.len(), 1);
        let s = &report.steps[0];
        assert_eq!(s.depth, 0);
        assert!(s.edge.is_none());
        assert_eq!(s.node_id, "A");
        assert_eq!(s.visit, Visit::First);
        assert_eq!(s.endpoint, Endpoint::Declared);
        assert!(!s.tainted);
    }

    #[test]
    fn traversal_one_edge() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "test", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "verifies"), prov());
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        assert_eq!(report.steps.len(), 2);
        assert_eq!(report.steps[0].node_id, "A");
        assert_eq!(report.steps[1].node_id, "B");
        assert_eq!(report.steps[1].depth, 1);
        assert!(report.steps[1].edge.is_some());
    }

    #[test]
    fn traversal_vacancy_endpoint() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "X", "verifies"), prov());
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        assert_eq!(report.steps.len(), 2);
        assert_eq!(report.steps[1].node_id, "X");
        assert_eq!(report.steps[1].endpoint, Endpoint::Vacancy);
    }

    #[test]
    fn traversal_cycle_detection() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        g.add_edge(edge("C", "A", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        let ids: Vec<_> = report.steps.iter().map(|s| s.node_id).collect();
        assert_eq!(ids, vec!["A", "B", "C", "A"]);
        assert_eq!(report.steps[3].visit, Visit::Cycle);
    }

    #[test]
    fn traversal_reconvergence_detection() {
        // Diamond: A → B → D, A → C → D
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
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        let d_steps: Vec<_> = report.steps.iter().filter(|s| s.node_id == "D").collect();
        assert_eq!(d_steps.len(), 2);
        assert_eq!(d_steps[0].visit, Visit::First);
        assert_eq!(d_steps[1].visit, Visit::Reconvergence);
    }

    #[test]
    fn traversal_reconvergence_stops_expansion() {
        // A → B → D → E, A → C → D → E
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_node("D", "req", Map::new(), prov()).unwrap();
        g.add_node("E", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("A", "C", "derives"), prov());
        g.add_edge(edge("B", "D", "derives"), prov());
        g.add_edge(edge("C", "D", "derives"), prov());
        g.add_edge(edge("D", "E", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        let e_count = report.steps.iter().filter(|s| s.node_id == "E").count();
        assert_eq!(
            e_count, 1,
            "E should appear only once under D's first expansion"
        );
    }

    #[test]
    fn traversal_parallel_edges() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "test", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("A", "B", "verifies"), prov());
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        let b_steps: Vec<_> = report.steps.iter().filter(|s| s.node_id == "B").collect();
        assert_eq!(b_steps.len(), 2);
        assert_eq!(b_steps[0].parallel_count, 2);
        assert_eq!(b_steps[1].parallel_count, 2);
        assert_eq!(b_steps[0].parallel_ordinal, 0);
        assert_eq!(b_steps[1].parallel_ordinal, 1);
    }

    #[test]
    fn traversal_reverse_direction() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "test", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "verifies"), prov());
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "B", Direction::Reverse).unwrap();
        let ids: Vec<_> = report.steps.iter().map(|s| s.node_id).collect();
        assert_eq!(ids, vec!["B", "A"]);
        assert_eq!(report.direction, Direction::Reverse);
    }

    #[test]
    fn traversal_filtered_edge_kinds() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "test", Map::new(), prov()).unwrap();
        g.add_node("C", "spec", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "verifies"), prov());
        g.add_edge(edge("A", "C", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let filtered = idx.filtered(&["verifies"]);
        let report = traverse(&g, &filtered, "A", Direction::Forward).unwrap();
        let ids: Vec<_> = report.steps.iter().map(|s| s.node_id).collect();
        assert_eq!(ids, vec!["A", "B"]);
    }

    #[test]
    fn traversal_taint_through_vacancy() {
        // A → X (undeclared) → B
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "test", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "X", "derives"), prov());
        g.add_edge(edge("X", "B", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        let x_step = report.steps.iter().find(|s| s.node_id == "X").unwrap();
        assert!(x_step.tainted, "vacancy itself is tainted");
        assert_eq!(x_step.endpoint, Endpoint::Vacancy);
        let b_step = report.steps.iter().find(|s| s.node_id == "B").unwrap();
        assert!(
            b_step.tainted,
            "node reachable only through vacancy is tainted"
        );
    }

    #[test]
    fn traversal_taint_clean_path_clears_taint() {
        // A → B (declared) and A → X (undeclared) → B
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "test", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("A", "X", "derives"), prov());
        g.add_edge(edge("X", "B", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        let b_first = report
            .steps
            .iter()
            .find(|s| s.node_id == "B" && s.visit == Visit::First)
            .unwrap();
        assert!(!b_first.tainted, "B has a clean direct path from A");
    }

    #[test]
    fn traversal_root_not_tainted() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        let idx = AdjacencyIndex::build(&g);
        let report = traverse(&g, &idx, "A", Direction::Forward).unwrap();
        assert!(!report.steps[0].tainted);
    }

    #[test]
    fn traversal_unknown_root_is_error() {
        let g = LatticeGraph::new();
        let idx = AdjacencyIndex::build(&g);
        assert!(traverse(&g, &idx, "Z", Direction::Forward).is_err());
    }
}

#[cfg(test)]
mod bfs_layers_tests {
    use crate::graph::{EdgeSpec, LatticeGraph};
    use crate::query::{AdjacencyIndex, Direction, bfs_layers};
    use crate::types::Provenance;
    use serde_json::Map;

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
    fn linear_chain_depth_grouping() {
        // A → B → C → D
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_node("D", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        g.add_edge(edge("C", "D", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layers = bfs_layers(&idx, "A", Direction::Forward, 10, |_| true);
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0], vec!["B"]);
        assert_eq!(layers[1], vec!["C"]);
        assert_eq!(layers[2], vec!["D"]);
    }

    #[test]
    fn diamond_reconvergence() {
        // A → B, A → C, B → D, C → D
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

        let layers = bfs_layers(&idx, "A", Direction::Forward, 10, |_| true);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0], vec!["B", "C"]);
        assert_eq!(layers[1], vec!["D"]); // D at depth 2, not duplicated
    }

    #[test]
    fn vacancy_admission() {
        // A → X (undeclared) → B
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "X", "derives"), prov());
        g.add_edge(edge("X", "B", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        // With enter = |_| true, vacancies are admitted
        let layers = bfs_layers(&idx, "A", Direction::Forward, 10, |_| true);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0], vec!["X"]);
        assert_eq!(layers[1], vec!["B"]);

        // With enter filtering to declared only: X refused, B unreachable
        let layers = bfs_layers(&idx, "A", Direction::Forward, 10, |id| g.has_node(id));
        assert!(
            layers.is_empty(),
            "no declared nodes reachable without traversing vacancy"
        );
    }

    #[test]
    fn reverse_direction() {
        // A → B → C
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layers = bfs_layers(&idx, "C", Direction::Reverse, 10, |_| true);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0], vec!["B"]);
        assert_eq!(layers[1], vec!["A"]);
    }

    #[test]
    fn single_node_graph() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        let idx = AdjacencyIndex::build(&g);

        let layers = bfs_layers(&idx, "A", Direction::Forward, 10, |_| true);
        assert!(layers.is_empty());
    }

    #[test]
    fn max_depth_caps_layers() {
        // A → B → C → D, max_depth = 2
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_node("D", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        g.add_edge(edge("C", "D", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layers = bfs_layers(&idx, "A", Direction::Forward, 2, |_| true);
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0], vec!["B"]);
        assert_eq!(layers[1], vec!["C"]);
    }

    #[test]
    fn origin_excluded_from_layers() {
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let layers = bfs_layers(&idx, "A", Direction::Forward, 10, |_| true);
        assert!(!layers.iter().any(|layer| layer.contains(&"A")));
    }
}

#[cfg(test)]
mod centrality_tests {
    use crate::graph::{EdgeSpec, LatticeGraph};
    use crate::query::{AdjacencyIndex, betweenness_centrality};
    use crate::types::Provenance;
    use serde_json::Map;

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
    fn directed_hub() {
        // L1 → H, L2 → H, H → R1, H → R2
        // All shortest paths between Li and Rj go through H.
        let mut g = LatticeGraph::new();
        g.add_node("L1", "req", Map::new(), prov()).unwrap();
        g.add_node("L2", "req", Map::new(), prov()).unwrap();
        g.add_node("H", "req", Map::new(), prov()).unwrap();
        g.add_node("R1", "req", Map::new(), prov()).unwrap();
        g.add_node("R2", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("L1", "H", "derives"), prov());
        g.add_edge(edge("L2", "H", "derives"), prov());
        g.add_edge(edge("H", "R1", "derives"), prov());
        g.add_edge(edge("H", "R2", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let c = betweenness_centrality(&idx);
        assert_eq!(c["H"], 1.0, "hub has max centrality");
        assert_eq!(c["L1"], 0.0, "leaf has zero centrality");
        assert_eq!(c["L2"], 0.0);
        assert_eq!(c["R1"], 0.0);
        assert_eq!(c["R2"], 0.0);
    }

    #[test]
    fn linear_chain_interior_higher() {
        // A → B → C → D
        let mut g = LatticeGraph::new();
        g.add_node("A", "req", Map::new(), prov()).unwrap();
        g.add_node("B", "req", Map::new(), prov()).unwrap();
        g.add_node("C", "req", Map::new(), prov()).unwrap();
        g.add_node("D", "req", Map::new(), prov()).unwrap();
        g.add_edge(edge("A", "B", "derives"), prov());
        g.add_edge(edge("B", "C", "derives"), prov());
        g.add_edge(edge("C", "D", "derives"), prov());
        let idx = AdjacencyIndex::build(&g);

        let c = betweenness_centrality(&idx);
        assert!(c["B"] > c["A"], "interior B > endpoint A");
        assert!(c["C"] > c["D"], "interior C > endpoint D");
        assert_eq!(c["A"], 0.0);
        assert_eq!(c["D"], 0.0);
    }

    #[test]
    fn empty_graph() {
        let g = LatticeGraph::new();
        let idx = AdjacencyIndex::build(&g);
        let c = betweenness_centrality(&idx);
        assert!(c.is_empty());
    }
}
