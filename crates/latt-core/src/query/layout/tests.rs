//! Tests for the layered layout; the measured answers are pinned here.

use super::{
    AdjacencyIndex, Direction, count_layer_crossings, graph_layout, minimize_crossings,
    pr_positions,
};
use crate::graph::{EdgeSpec, LatticeGraph};
use crate::types::{LayoutEdge, LayoutNode, Provenance};
use serde_json::Map;
use std::collections::BTreeSet;

fn build_virtual_lists(
    edges: &[LayoutEdge],
    layer_of: &[usize],
    num_comps: usize,
) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
    let virtual_in: Vec<Vec<usize>> = (0..num_comps)
        .map(|ci| {
            edges
                .iter()
                .enumerate()
                .filter(|&(_, e)| {
                    e.tgt_component == ci && layer_of[e.src_component] + 1 < layer_of[ci]
                })
                .map(|(ei, _)| ei)
                .collect()
        })
        .collect();
    let virtual_out: Vec<Vec<usize>> = (0..num_comps)
        .map(|ci| {
            edges
                .iter()
                .enumerate()
                .filter(|&(_, e)| {
                    e.src_component == ci && layer_of[e.tgt_component] > layer_of[ci] + 1
                })
                .map(|(ei, _)| ei)
                .collect()
        })
        .collect();
    (virtual_in, virtual_out)
}

fn prov() -> Provenance {
    Provenance::new("test.md", 1)
}

fn edge(src: &str, tgt: &str, kind: &str) -> (EdgeSpec, Provenance) {
    (
        EdgeSpec {
            src: src.into(),
            tgt: tgt.into(),
            kind: kind.into(),
            attrs: Map::new(),
        },
        prov(),
    )
}

fn add_edge(g: &mut LatticeGraph, src: &str, tgt: &str, kind: &str) {
    let (spec, p) = edge(src, tgt, kind);
    g.add_edge(spec, p);
}

#[test]
fn linear_chain_three_layers() {
    let mut g = LatticeGraph::new();
    g.add_node("A", "kind", Map::new(), prov()).unwrap();
    g.add_node("B", "kind", Map::new(), prov()).unwrap();
    g.add_node("C", "kind", Map::new(), prov()).unwrap();
    add_edge(&mut g, "A", "B", "e");
    add_edge(&mut g, "B", "C", "e");
    let idx = AdjacencyIndex::build(&g);
    let layout = graph_layout(&idx, "A", 3, Direction::Forward);
    assert_eq!(layout.layers.len(), 3);
    assert_eq!(layout.layers[0].len(), 1);
    assert_eq!(layout.layers[1].len(), 1);
    assert_eq!(layout.layers[2].len(), 1);
}

#[test]
fn skip_edge_inserts_virtual_node() {
    let mut g = LatticeGraph::new();
    g.add_node("A", "kind", Map::new(), prov()).unwrap();
    g.add_node("B", "kind", Map::new(), prov()).unwrap();
    g.add_node("C", "kind", Map::new(), prov()).unwrap();
    add_edge(&mut g, "A", "B", "e");
    add_edge(&mut g, "B", "C", "e");
    add_edge(&mut g, "A", "C", "e");
    let idx = AdjacencyIndex::build(&g);
    let layout = graph_layout(&idx, "A", 3, Direction::Forward);
    assert_eq!(layout.layers.len(), 3);
    let has_virtual = layout.layers[1]
        .iter()
        .any(|n| matches!(n, LayoutNode::Virtual(_)));
    assert!(
        has_virtual,
        "skip edge A→C should produce a virtual node in layer 1"
    );
}

#[test]
fn diamond_crossing_minimized() {
    let mut g = LatticeGraph::new();
    g.add_node("A", "kind", Map::new(), prov()).unwrap();
    g.add_node("B", "kind", Map::new(), prov()).unwrap();
    g.add_node("C", "kind", Map::new(), prov()).unwrap();
    g.add_node("D", "kind", Map::new(), prov()).unwrap();
    add_edge(&mut g, "A", "B", "e");
    add_edge(&mut g, "A", "C", "e");
    add_edge(&mut g, "B", "D", "e");
    add_edge(&mut g, "C", "D", "e");
    let idx = AdjacencyIndex::build(&g);
    let layout = graph_layout(&idx, "A", 3, Direction::Forward);
    assert_eq!(layout.layers.len(), 3);
    assert_eq!(layout.layers[0].len(), 1);
    assert_eq!(layout.layers[1].len(), 2);
    assert_eq!(layout.layers[2].len(), 1);
}

#[test]
fn bc_reduces_crossings() {
    // R→A, R→B (layer 1 = [A, B]), A→D, B→C (layer 2 = [C, D] initially).
    // Crossing: A(0)→D(1) and B(1)→C(0) cross. BC reorders layer 2 to [D, C].
    let mut g = LatticeGraph::new();
    g.add_node("R", "kind", Map::new(), prov()).unwrap();
    g.add_node("A", "kind", Map::new(), prov()).unwrap();
    g.add_node("B", "kind", Map::new(), prov()).unwrap();
    g.add_node("C", "kind", Map::new(), prov()).unwrap();
    g.add_node("D", "kind", Map::new(), prov()).unwrap();
    add_edge(&mut g, "R", "A", "e");
    add_edge(&mut g, "R", "B", "e");
    add_edge(&mut g, "A", "D", "e");
    add_edge(&mut g, "B", "C", "e");
    let idx = AdjacencyIndex::build(&g);
    let layout = graph_layout(&idx, "R", 3, Direction::Forward);
    assert_eq!(layout.layers.len(), 3);
    let layer2_ids: Vec<_> = layout.layers[2]
        .iter()
        .filter_map(|n| match n {
            LayoutNode::Real(ci) => Some(layout.components[*ci].members[0].as_str()),
            LayoutNode::Virtual(_) => None,
        })
        .collect();
    // BC of D = pos(A) in layer 1, BC of C = pos(B) in layer 1.
    // If A precedes B, D should precede C (no crossing).
    let layer1_ids: Vec<_> = layout.layers[1]
        .iter()
        .filter_map(|n| match n {
            LayoutNode::Real(ci) => Some(layout.components[*ci].members[0].as_str()),
            LayoutNode::Virtual(_) => None,
        })
        .collect();
    // The crossing-free order: D mirrors A's position, C mirrors B's.
    let d_pos = layer2_ids.iter().position(|&id| id == "D").unwrap();
    let c_pos = layer2_ids.iter().position(|&id| id == "C").unwrap();
    let a_pos = layer1_ids.iter().position(|&id| id == "A").unwrap();
    let b_pos = layer1_ids.iter().position(|&id| id == "B").unwrap();
    // No crossing means: (A < B) == (D < C) or (A > B) == (D > C).
    assert_eq!(
        a_pos < b_pos,
        d_pos < c_pos,
        "BC should align children with parents to eliminate crossings"
    );
}

#[test]
fn center_component_tracked() {
    let mut g = LatticeGraph::new();
    g.add_node("X", "kind", Map::new(), prov()).unwrap();
    g.add_node("Y", "kind", Map::new(), prov()).unwrap();
    add_edge(&mut g, "X", "Y", "e");
    let idx = AdjacencyIndex::build(&g);
    let layout = graph_layout(&idx, "X", 2, Direction::Forward);
    let center_id = &layout.components[layout.center_component].members[0];
    assert_eq!(center_id, "X");
}

#[test]
fn pr_preserves_order() {
    // A→B, A→C: B and C in layer 1 must preserve crossing-min order.
    let mut g = LatticeGraph::new();
    g.add_node("A", "kind", Map::new(), prov()).unwrap();
    g.add_node("B", "kind", Map::new(), prov()).unwrap();
    g.add_node("C", "kind", Map::new(), prov()).unwrap();
    add_edge(&mut g, "A", "B", "e");
    add_edge(&mut g, "A", "C", "e");
    let idx = AdjacencyIndex::build(&g);
    let layout = graph_layout(&idx, "A", 3, Direction::Forward);
    // Order in layer 1 must be preserved in x_positions.
    let x0 = layout.x_positions[1][0];
    let x1 = layout.x_positions[1][1];
    assert!(
        x0 < x1,
        "PR must preserve crossing-minimized order: {x0} < {x1}"
    );
}

#[test]
fn pr_virtual_aligns_with_source() {
    // A→B (layer 0→1), A→C (layer 0→2, skip edge creates virtual in layer 1).
    // Virtual node for A→C should align with A's x position.
    let mut g = LatticeGraph::new();
    g.add_node("A", "kind", Map::new(), prov()).unwrap();
    g.add_node("B", "kind", Map::new(), prov()).unwrap();
    g.add_node("C", "kind", Map::new(), prov()).unwrap();
    add_edge(&mut g, "A", "B", "e");
    add_edge(&mut g, "B", "C", "e");
    add_edge(&mut g, "A", "C", "e");
    let idx = AdjacencyIndex::build(&g);
    let layout = graph_layout(&idx, "A", 3, Direction::Forward);
    // Layer 1 has B (real) and a virtual node. Virtual should be pulled
    // toward A's x position (which is in layer 0).
    let a_x = layout.x_positions[0][0];
    let mut virtual_x = None;
    for (i, node) in layout.layers[1].iter().enumerate() {
        if matches!(node, LayoutNode::Virtual(_)) {
            virtual_x = Some(layout.x_positions[1][i]);
        }
    }
    let vx = virtual_x.expect("skip edge A→C must produce a virtual node in layer 1");
    // Virtual should be close to A's position (within 1).
    assert!(
        vx.abs_diff(a_x) <= 1,
        "virtual node x={vx} should be near source A x={a_x}"
    );
}

#[test]
fn pr_positions_strictly_increasing() {
    // With multiple nodes per layer, all x positions must be strictly
    // increasing (no two nodes at the same position).
    let mut g = LatticeGraph::new();
    g.add_node("R", "kind", Map::new(), prov()).unwrap();
    g.add_node("A", "kind", Map::new(), prov()).unwrap();
    g.add_node("B", "kind", Map::new(), prov()).unwrap();
    g.add_node("C", "kind", Map::new(), prov()).unwrap();
    g.add_node("D", "kind", Map::new(), prov()).unwrap();
    add_edge(&mut g, "R", "A", "e");
    add_edge(&mut g, "R", "B", "e");
    add_edge(&mut g, "R", "C", "e");
    add_edge(&mut g, "A", "D", "e");
    add_edge(&mut g, "B", "D", "e");
    add_edge(&mut g, "C", "D", "e");
    let idx = AdjacencyIndex::build(&g);
    let layout = graph_layout(&idx, "R", 3, Direction::Forward);
    for (li, layer_positions) in layout.x_positions.iter().enumerate() {
        for w in layer_positions.windows(2) {
            assert!(
                w[0] < w[1],
                "layer {li}: positions must be strictly increasing, got {} >= {}",
                w[0],
                w[1]
            );
        }
    }
}

#[test]
fn count_crossings_long_edge_first_segment() {
    // Layer 0: [Real(0), Real(1)]  — P1, P2
    // Layer 1: [Real(2), Virtual(0)] — C, virtual for P1→Q
    // Edges: P1→C (direct), P2→C (direct), P1→Q (long, edge index 0)
    // P1(pos 0)→Virtual(pos 1) and P2(pos 1)→C(pos 0) cross.
    let layers = vec![
        vec![LayoutNode::Real(0), LayoutNode::Real(1)],
        vec![LayoutNode::Real(2), LayoutNode::Virtual(0)],
        vec![LayoutNode::Real(3)],
    ];
    let edges = vec![
        LayoutEdge {
            src_component: 0,
            tgt_component: 3,
            kind: "e".into(),
        },
        LayoutEdge {
            src_component: 0,
            tgt_component: 2,
            kind: "e".into(),
        },
        LayoutEdge {
            src_component: 1,
            tgt_component: 2,
            kind: "e".into(),
        },
    ];
    let layer_of = vec![0, 0, 1, 2]; // comp 0,1 at layer 0; comp 2 at layer 1; comp 3 at layer 2
    let comp_children: Vec<BTreeSet<usize>> = vec![
        BTreeSet::from([2, 3]), // P1 → C, Q
        BTreeSet::from([2]),    // P2 → C
        BTreeSet::new(),        // C
        BTreeSet::new(),        // Q
    ];
    let (_, virtual_out) = build_virtual_lists(&edges, &layer_of, 4);
    let crossings = count_layer_crossings(&layers, &edges, &layer_of, &comp_children, &virtual_out);
    assert_eq!(
        crossings, 1,
        "P1→Virtual(0) at pos 1 and P2→C at pos 0 must cross"
    );
}

#[test]
fn bc_reduces_crossings_from_crossed_initial() {
    // Hand-built crossed input:
    // Layer 0: [Real(0)]           — root
    // Layer 1: [Real(1), Real(2)]  — A, B
    // Layer 2: [Real(4), Real(3)]  — D before C (crossed: A→C at pos 1, B→D at pos 0)
    // Edges: 0→1, 0→2, 1→3, 2→4
    let mut layers = vec![
        vec![LayoutNode::Real(0)],
        vec![LayoutNode::Real(1), LayoutNode::Real(2)],
        vec![LayoutNode::Real(4), LayoutNode::Real(3)],
    ];
    let edges = vec![
        LayoutEdge {
            src_component: 0,
            tgt_component: 1,
            kind: "e".into(),
        },
        LayoutEdge {
            src_component: 0,
            tgt_component: 2,
            kind: "e".into(),
        },
        LayoutEdge {
            src_component: 1,
            tgt_component: 3,
            kind: "e".into(),
        },
        LayoutEdge {
            src_component: 2,
            tgt_component: 4,
            kind: "e".into(),
        },
    ];
    let layer_of = vec![0, 1, 1, 2, 2];
    let comp_parents: Vec<BTreeSet<usize>> = vec![
        BTreeSet::new(),
        BTreeSet::from([0]),
        BTreeSet::from([0]),
        BTreeSet::from([1]),
        BTreeSet::from([2]),
    ];
    let comp_children: Vec<BTreeSet<usize>> = vec![
        BTreeSet::from([1, 2]),
        BTreeSet::from([3]),
        BTreeSet::from([4]),
        BTreeSet::new(),
        BTreeSet::new(),
    ];

    let (virtual_in, virtual_out) = build_virtual_lists(&edges, &layer_of, 5);
    let before = count_layer_crossings(&layers, &edges, &layer_of, &comp_children, &virtual_out);
    assert_eq!(before, 1, "initial crossed order must have 1 crossing");

    minimize_crossings(
        &mut layers,
        &edges,
        &layer_of,
        &comp_parents,
        &comp_children,
        &virtual_in,
        &virtual_out,
    );

    let after = count_layer_crossings(&layers, &edges, &layer_of, &comp_children, &virtual_out);
    assert_eq!(after, 0, "BC must eliminate the crossing");
}

#[test]
fn pr_pulls_child_toward_parent() {
    // Layer 0: [Real(0), Real(1), Real(2)] — A(x=0), B(x=1), C(x=2)
    // Layer 1: [Real(3)]                   — D, only child of C (comp 2)
    // Init puts D at x=0. PR should pull D toward C's x=2.
    let layers = vec![
        vec![
            LayoutNode::Real(0),
            LayoutNode::Real(1),
            LayoutNode::Real(2),
        ],
        vec![LayoutNode::Real(3)],
    ];
    let edges = vec![LayoutEdge {
        src_component: 2,
        tgt_component: 3,
        kind: "e".into(),
    }];
    let layer_of = vec![0, 0, 0, 1];
    let comp_parents: Vec<BTreeSet<usize>> = vec![
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::from([2]),
    ];
    let comp_children: Vec<BTreeSet<usize>> = vec![
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::from([3]),
        BTreeSet::new(),
    ];

    let (virtual_in, virtual_out) = build_virtual_lists(&edges, &layer_of, 4);
    let x = pr_positions(
        &layers,
        &edges,
        &layer_of,
        &comp_parents,
        &comp_children,
        &virtual_in,
        &virtual_out,
    );
    assert_eq!(
        x[1][0], 2,
        "PR must pull D to parent C's position (x=2), got x={}",
        x[1][0]
    );
}

#[test]
fn bc_sweep_sees_virtual_neighbours() {
    // Two long edges that cross in their middle layer. BC must reorder layer 2
    // using Virtual positions in layer 1, not fallback insertion order.
    //
    // Layer 0: [Real(0), Real(1)]           — A, B
    // Layer 1: [Virtual(0), Virtual(1)]     — routing for A→C, B→D (correct order)
    // Layer 2: [Real(3), Real(2)]           — D before C (crossed!)
    //
    // Edges: A(0)→C(2) long (ei=0), B(1)→D(3) long (ei=1)
    //
    // Top-down correctly reorders layer 1 from parents in layer 0.
    // For layer 2: C(2) has parent A(0) at layer 0, not at l-1=1.
    //   Without fix: C gets fallback bc (insertion order), D likewise → no reorder.
    //   With fix: C sees Virtual(0) at layer 1, D sees Virtual(1) → correct bc.
    let mut layers = vec![
        vec![LayoutNode::Real(0), LayoutNode::Real(1)],
        vec![LayoutNode::Virtual(0), LayoutNode::Virtual(1)],
        vec![LayoutNode::Real(3), LayoutNode::Real(2)],
    ];
    let edges = vec![
        LayoutEdge {
            src_component: 0,
            tgt_component: 2,
            kind: "e".into(),
        },
        LayoutEdge {
            src_component: 1,
            tgt_component: 3,
            kind: "e".into(),
        },
    ];
    let layer_of = vec![0, 0, 2, 2];
    let comp_parents: Vec<BTreeSet<usize>> = vec![
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::from([0]),
        BTreeSet::from([1]),
    ];
    let comp_children: Vec<BTreeSet<usize>> = vec![
        BTreeSet::from([2]),
        BTreeSet::from([3]),
        BTreeSet::new(),
        BTreeSet::new(),
    ];

    // Layer 1→2: V0(pos 0)→D(pos 0) and V1(pos 1)→C(pos 1) — no crossing
    // between layers 1-2 in terms of virtual→real? Actually:
    // V0 routes edge 0 (A→C), so V0→C(2). C is at layer 2 pos 1.
    // V1 routes edge 1 (B→D), so V1→D(3). D is at layer 2 pos 0.
    // Edges: (0, 1) and (1, 0) → crossing.
    let (virtual_in, virtual_out) = build_virtual_lists(&edges, &layer_of, 4);
    let before = count_layer_crossings(&layers, &edges, &layer_of, &comp_children, &virtual_out);
    assert_eq!(
        before, 1,
        "initial layer 2 order [D, C] must cross with layer 1 [V0, V1]"
    );

    minimize_crossings(
        &mut layers,
        &edges,
        &layer_of,
        &comp_parents,
        &comp_children,
        &virtual_in,
        &virtual_out,
    );

    let after = count_layer_crossings(&layers, &edges, &layer_of, &comp_children, &virtual_out);
    assert_eq!(
        after, 0,
        "BC must reorder layer 2 to [C, D] via virtual neighbours"
    );
}

#[test]
fn inversion_count_matches_pairwise() {
    use super::merge_sort_inversions;
    let cases: Vec<Vec<(usize, usize)>> = vec![
        vec![(0, 1), (1, 0)],         // one crossing
        vec![(0, 0), (0, 1)],         // shared top, no crossing
        vec![(0, 1), (1, 1)],         // shared bottom, no crossing
        vec![(0, 2), (1, 0), (2, 1)], // two crossings
        vec![(0, 0), (1, 1), (2, 2)], // sorted, zero
        vec![(0, 2), (0, 1), (1, 0)], // shared top + crossing
        vec![],                       // empty
    ];
    for pairs in &cases {
        let pairwise = {
            let mut c = 0;
            for i in 0..pairs.len() {
                for j in (i + 1)..pairs.len() {
                    let (a1, b1) = pairs[i];
                    let (a2, b2) = pairs[j];
                    if (a1 < a2 && b1 > b2) || (a1 > a2 && b1 < b2) {
                        c += 1;
                    }
                }
            }
            c
        };
        let mut sorted = pairs.clone();
        sorted.sort_unstable();
        let mut bots: Vec<usize> = sorted.iter().map(|&(_, b)| b).collect();
        let merge = merge_sort_inversions(&mut bots);
        assert_eq!(merge, pairwise, "mismatch for {pairs:?}");
    }
}
