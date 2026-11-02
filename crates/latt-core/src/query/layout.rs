//! Layered graph layout: crossing minimization and horizontal positioning.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::types::{GraphLayout, LayoutEdge, LayoutNode};

use super::components::components;
use super::index::AdjacencyIndex;
use super::{Direction, bfs_layers};

/// Count edge crossings between all adjacent layer pairs.
///
/// An edge from layer L to layer L+1 is any: Real parent → Real child at L+1,
/// Real source → Virtual(ei) at L+1 (first segment of a long edge), or
/// Virtual(ei) → Real target or Virtual(ei) at L+1 (continuation/last segment).
fn count_layer_crossings(
    layers: &[Vec<LayoutNode>],
    edges: &[LayoutEdge],
    layer_of: &[usize],
    comp_children: &[BTreeSet<usize>],
    virtual_out: &[Vec<usize>],
) -> usize {
    if layers.len() < 2 {
        return 0;
    }
    let max_layer = layers.len() - 1;
    let mut total = 0;
    for l in 0..max_layer {
        let next_pos: HashMap<LayoutNode, usize> = layers[l + 1]
            .iter()
            .enumerate()
            .map(|(i, &n)| (n, i))
            .collect();
        let mut edge_positions: Vec<(usize, usize)> = Vec::new();
        for (top_pos, node) in layers[l].iter().enumerate() {
            let children: Vec<usize> = match *node {
                LayoutNode::Real(ci) => {
                    let mut targets: Vec<usize> = comp_children[ci]
                        .iter()
                        .filter(|&&chi| layer_of[chi] == l + 1)
                        .filter_map(|&chi| next_pos.get(&LayoutNode::Real(chi)).copied())
                        .collect();
                    for &ei in &virtual_out[ci] {
                        if let Some(&p) = next_pos.get(&LayoutNode::Virtual(ei)) {
                            targets.push(p);
                        }
                    }
                    targets
                }
                LayoutNode::Virtual(ei) => {
                    let target = if layer_of[edges[ei].tgt_component] == l + 1 {
                        LayoutNode::Real(edges[ei].tgt_component)
                    } else {
                        LayoutNode::Virtual(ei)
                    };
                    next_pos.get(&target).copied().into_iter().collect()
                }
            };
            for bot_pos in children {
                edge_positions.push((top_pos, bot_pos));
            }
        }
        // Sort by (top, bot) so same-top edges group with ascending bot,
        // then count inversions in the bot sequence — each is a crossing.
        edge_positions.sort_unstable();
        let mut bots: Vec<usize> = edge_positions.iter().map(|&(_, b)| b).collect();
        total += merge_sort_inversions(&mut bots);
    }
    total
}

/// Count inversions via merge sort: pairs `(i < j)` where `seq[i] > seq[j]`. O(n log n).
fn merge_sort_inversions(seq: &mut [usize]) -> usize {
    let mut scratch = Vec::with_capacity(seq.len());
    merge_sort_inversions_with(seq, &mut scratch)
}

/// The recursion behind `merge_sort_inversions`; `scratch` is the one merge
/// buffer every level reuses instead of copying both halves afresh.
fn merge_sort_inversions_with(seq: &mut [usize], scratch: &mut Vec<usize>) -> usize {
    let n = seq.len();
    if n <= 1 {
        return 0;
    }
    let mid = n / 2;
    let mut count = merge_sort_inversions_with(&mut seq[..mid], scratch);
    count += merge_sort_inversions_with(&mut seq[mid..], scratch);
    scratch.clear();
    scratch.extend_from_slice(seq);
    let (left, right) = scratch.split_at(mid);
    let (mut li, mut ri, mut wi) = (0, 0, 0);
    while li < left.len() && ri < right.len() {
        if left[li] <= right[ri] {
            seq[wi] = left[li];
            li += 1;
        } else {
            // right[ri] < left[li]: all remaining left elements are also > right[ri].
            count += left.len() - li;
            seq[wi] = right[ri];
            ri += 1;
        }
        wi += 1;
    }
    while li < left.len() {
        seq[wi] = left[li];
        li += 1;
        wi += 1;
    }
    while ri < right.len() {
        seq[wi] = right[ri];
        ri += 1;
        wi += 1;
    }
    count
}

/// Barycenter crossing minimization with Phase 2 equal-group reversal.
///
/// Mutates `layers` in place. Phase 1 iterates BC sweeps (up to 8) until
/// convergence. Phase 2 reverses equal-barycenter groups, re-runs Phase 1,
/// and keeps the result only if it reduces crossings.
fn minimize_crossings(
    layers: &mut Vec<Vec<LayoutNode>>,
    edges: &[LayoutEdge],
    layer_of: &[usize],
    comp_parents: &[BTreeSet<usize>],
    comp_children: &[BTreeSet<usize>],
    virtual_in: &[Vec<usize>],
    virtual_out: &[Vec<usize>],
) {
    if layers.len() < 2 {
        return;
    }
    let max_layer = layers.len() - 1;

    let positions_of = |layer: &[LayoutNode]| -> HashMap<LayoutNode, f64> {
        layer
            .iter()
            .enumerate()
            .map(|(i, &n)| (n, i as f64))
            .collect()
    };

    let bc_sweep = |layers: &mut Vec<Vec<LayoutNode>>| -> Vec<Vec<(f64, LayoutNode)>> {
        let mut all_scored: Vec<Vec<(f64, LayoutNode)>> = vec![Vec::new(); max_layer + 1];
        // Top-down.
        for l in 1..=max_layer {
            let adj_pos = positions_of(&layers[l - 1]);
            let node_pos = |node: LayoutNode| adj_pos.get(&node).copied();
            let mut scored: Vec<(f64, LayoutNode)> = Vec::new();
            for node in &layers[l] {
                let bc = match node {
                    LayoutNode::Real(ci) => {
                        let mut positions: Vec<f64> = comp_parents[*ci]
                            .iter()
                            .filter(|&&pi| layer_of[pi] == l - 1)
                            .filter_map(|&pi| node_pos(LayoutNode::Real(pi)))
                            .collect();
                        for &ei in &virtual_in[*ci] {
                            if let Some(p) = node_pos(LayoutNode::Virtual(ei)) {
                                positions.push(p);
                            }
                        }
                        if positions.is_empty() {
                            scored.len() as f64
                        } else {
                            positions.iter().sum::<f64>() / positions.len() as f64
                        }
                    }
                    LayoutNode::Virtual(ei) => {
                        let src_c = edges[*ei].src_component;
                        let src_layer = layer_of[src_c];
                        let adjacent = if l == src_layer + 1 {
                            LayoutNode::Real(src_c)
                        } else {
                            LayoutNode::Virtual(*ei)
                        };
                        node_pos(adjacent).unwrap_or(scored.len() as f64)
                    }
                };
                scored.push((bc, *node));
            }
            scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            layers[l] = scored.iter().map(|(_, n)| *n).collect();
            all_scored[l] = scored;
        }
        // Bottom-up.
        for l in (0..max_layer).rev() {
            let adj_pos = positions_of(&layers[l + 1]);
            let node_pos = |node: LayoutNode| adj_pos.get(&node).copied();
            let mut scored: Vec<(f64, LayoutNode)> = Vec::new();
            for node in &layers[l] {
                let bc = match node {
                    LayoutNode::Real(ci) => {
                        let mut positions: Vec<f64> = comp_children[*ci]
                            .iter()
                            .filter(|&&chi| layer_of[chi] == l + 1)
                            .filter_map(|&chi| node_pos(LayoutNode::Real(chi)))
                            .collect();
                        for &ei in &virtual_out[*ci] {
                            if let Some(p) = node_pos(LayoutNode::Virtual(ei)) {
                                positions.push(p);
                            }
                        }
                        if positions.is_empty() {
                            scored.len() as f64
                        } else {
                            positions.iter().sum::<f64>() / positions.len() as f64
                        }
                    }
                    LayoutNode::Virtual(ei) => {
                        let tgt_c = edges[*ei].tgt_component;
                        let tgt_layer = layer_of[tgt_c];
                        let adjacent = if l == tgt_layer - 1 {
                            LayoutNode::Real(tgt_c)
                        } else {
                            LayoutNode::Virtual(*ei)
                        };
                        node_pos(adjacent).unwrap_or(scored.len() as f64)
                    }
                };
                scored.push((bc, *node));
            }
            scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            layers[l] = scored.iter().map(|(_, n)| *n).collect();
            all_scored[l] = scored;
        }
        all_scored
    };

    // Phase 1: iterate BC sweeps until convergence (max 8).
    let mut last_scored = Vec::new();
    for _ in 0..8 {
        let prev = layers.clone();
        last_scored = bc_sweep(layers);
        if *layers == prev {
            break;
        }
    }

    // Phase 2: reverse equal-barycenter groups, re-run Phase 1, keep if fewer crossings.
    let crossings_before =
        count_layer_crossings(layers, edges, layer_of, comp_children, virtual_out);
    let saved = layers.clone();
    for (l, scored) in last_scored.iter().enumerate() {
        if scored.is_empty() {
            continue;
        }
        let mut i = 0;
        while i < scored.len() {
            let mut j = i + 1;
            while j < scored.len() && (scored[j].0 - scored[i].0).abs() < 1e-9 {
                j += 1;
            }
            if j - i > 1 {
                layers[l][i..j].reverse();
            }
            i = j;
        }
    }
    for _ in 0..8 {
        let prev = layers.clone();
        bc_sweep(layers);
        if *layers == prev {
            break;
        }
    }
    if count_layer_crossings(layers, edges, layer_of, comp_children, virtual_out) > crossings_before
    {
        *layers = saved;
    }
}

/// PR (Priority) horizontal positioning for a Sugiyama layout.
///
/// Assigns integer x-coordinates to each node per layer, preserving the
/// crossing-minimized order while pulling connected nodes closer together.
/// Based on Section IV-B of Sugiyama, Tagawa & Toda (1981).
fn pr_positions(
    layers: &[Vec<LayoutNode>],
    edges: &[LayoutEdge],
    layer_of: &[usize],
    comp_parents: &[BTreeSet<usize>],
    comp_children: &[BTreeSet<usize>],
    virtual_in: &[Vec<usize>],
    virtual_out: &[Vec<usize>],
) -> Vec<Vec<usize>> {
    if layers.is_empty() {
        return Vec::new();
    }
    let max_layer = layers.len() - 1;

    // Find maximum connectivity for the dummy-priority ceiling.
    let mut max_conn: usize = 0;
    for layer in layers {
        for node in layer {
            if let LayoutNode::Real(ci) = node {
                let conn = comp_parents[*ci].len() + comp_children[*ci].len();
                if conn > max_conn {
                    max_conn = conn;
                }
            }
        }
    }
    let virtual_priority = max_conn + 1;

    let mut pos: Vec<Vec<i64>> = layers
        .iter()
        .map(|layer| (0..layer.len() as i64).collect())
        .collect();

    // Barycenter of a node's neighbours in an adjacent layer.
    let neighbour_barycenter = |node: &LayoutNode,
                                layer_idx: usize,
                                adj_layer: usize,
                                adj_pos: &HashMap<LayoutNode, i64>|
     -> Option<f64> {
        let mut sum = 0i64;
        let mut count = 0usize;

        match node {
            LayoutNode::Real(ci) => {
                if adj_layer < layer_idx {
                    for &pi in &comp_parents[*ci] {
                        if let Some(&p) = adj_pos.get(&LayoutNode::Real(pi)) {
                            sum += p;
                            count += 1;
                        }
                    }
                    for &ei in &virtual_in[*ci] {
                        if let Some(&p) = adj_pos.get(&LayoutNode::Virtual(ei)) {
                            sum += p;
                            count += 1;
                        }
                    }
                } else {
                    for &chi in &comp_children[*ci] {
                        if let Some(&p) = adj_pos.get(&LayoutNode::Real(chi)) {
                            sum += p;
                            count += 1;
                        }
                    }
                    for &ei in &virtual_out[*ci] {
                        if let Some(&p) = adj_pos.get(&LayoutNode::Virtual(ei)) {
                            sum += p;
                            count += 1;
                        }
                    }
                }
            }
            LayoutNode::Virtual(ei) => {
                let src_c = edges[*ei].src_component;
                let tgt_c = edges[*ei].tgt_component;
                let src_layer = layer_of[src_c];
                let tgt_layer = layer_of[tgt_c];

                if adj_layer < layer_idx {
                    if adj_layer == src_layer {
                        if let Some(&p) = adj_pos.get(&LayoutNode::Real(src_c)) {
                            sum += p;
                            count += 1;
                        }
                    } else if let Some(&p) = adj_pos.get(&LayoutNode::Virtual(*ei)) {
                        sum += p;
                        count += 1;
                    }
                } else if adj_layer == tgt_layer {
                    if let Some(&p) = adj_pos.get(&LayoutNode::Real(tgt_c)) {
                        sum += p;
                        count += 1;
                    }
                } else if let Some(&p) = adj_pos.get(&LayoutNode::Virtual(*ei)) {
                    sum += p;
                    count += 1;
                }
            }
        }

        if count == 0 {
            None
        } else {
            Some(sum as f64 / count as f64)
        }
    };

    // Directional priority: upper connectivity for DOWN, lower for UP.
    // Virtual nodes get the highest priority to straighten long edges (Element C2).
    let directional_priority = |node: &LayoutNode, adj_layer: usize, layer_idx: usize| -> usize {
        match node {
            LayoutNode::Virtual(_) => virtual_priority,
            LayoutNode::Real(ci) => {
                if adj_layer < layer_idx {
                    comp_parents[*ci].len()
                } else {
                    comp_children[*ci].len()
                }
            }
        }
    };

    // Improve one layer's positions given the adjacent layer's positions.
    let improve_layer = |layer_idx: usize, adj_layer: usize, pos: &mut Vec<Vec<i64>>| {
        let layer = &layers[layer_idx];
        let n = layer.len();
        if n == 0 {
            return;
        }

        // Build adj_pos once per layer pair.
        let adj_pos: HashMap<LayoutNode, i64> = layers[adj_layer]
            .iter()
            .enumerate()
            .map(|(i, &n)| (n, pos[adj_layer][i]))
            .collect();

        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            directional_priority(&layer[b], adj_layer, layer_idx)
                .cmp(&directional_priority(&layer[a], adj_layer, layer_idx))
        });

        // Track which nodes have been placed (walls).
        let mut placed = vec![false; n];

        for &idx in &order {
            let target = match neighbour_barycenter(&layer[idx], layer_idx, adj_layer, &adj_pos) {
                Some(bc) => bc.round() as i64,
                None => {
                    placed[idx] = true;
                    continue;
                }
            };

            let current = pos[layer_idx][idx];
            if target == current {
                placed[idx] = true;
                continue;
            }

            let new_pos = if target > current {
                let mut limit = target;
                for j in (idx + 1)..n {
                    if placed[j] {
                        limit = limit.min(pos[layer_idx][j] - (j - idx) as i64);
                        break;
                    }
                }
                limit.max(current)
            } else {
                let mut limit = target;
                for j in (0..idx).rev() {
                    if placed[j] {
                        limit = limit.max(pos[layer_idx][j] + (idx - j) as i64);
                        break;
                    }
                }
                limit.min(current)
            };

            let delta = new_pos - current;
            if delta != 0 {
                pos[layer_idx][idx] = new_pos;
                if delta > 0 {
                    for j in (idx + 1)..n {
                        if !placed[j] && pos[layer_idx][j] < pos[layer_idx][j - 1] + 1 {
                            pos[layer_idx][j] = pos[layer_idx][j - 1] + 1;
                        } else {
                            break;
                        }
                    }
                } else {
                    for j in (0..idx).rev() {
                        if !placed[j] && pos[layer_idx][j] > pos[layer_idx][j + 1] - 1 {
                            pos[layer_idx][j] = pos[layer_idx][j + 1] - 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            placed[idx] = true;
        }
    };

    // DOWN, UP, DOWN passes.
    for l in 1..=max_layer {
        improve_layer(l, l - 1, &mut pos);
    }
    for l in (0..max_layer).rev() {
        improve_layer(l, l + 1, &mut pos);
    }
    for l in 1..=max_layer {
        improve_layer(l, l - 1, &mut pos);
    }

    // Normalize: shift so global minimum is 0, convert to usize.
    let global_min = pos
        .iter()
        .flat_map(|row| row.iter())
        .copied()
        .min()
        .unwrap_or(0);
    pos.iter()
        .map(|row| row.iter().map(|&x| (x - global_min) as usize).collect())
        .collect()
}

/// Rooted Sugiyama layered DAG layout.
///
/// BFS-bounded subgraph from `center` up to `depth_radius` in both directions,
/// condensed via SCC, layered by longest path, with barycenter crossing
/// minimization.
#[must_use]
#[doc(hidden)]
pub fn graph_layout(
    index: &AdjacencyIndex<'_>,
    center: &str,
    depth_radius: usize,
    direction: Direction,
) -> GraphLayout {
    // 1. BFS subgraph: collect all nodes within depth_radius in both directions.
    let fwd_layers = bfs_layers(index, center, Direction::Forward, depth_radius, |_| true);
    let rev_layers = bfs_layers(index, center, Direction::Reverse, depth_radius, |_| true);
    let mut reachable: HashSet<&str> = HashSet::new();
    reachable.insert(center);
    for layer in &fwd_layers {
        for &id in layer {
            reachable.insert(id);
        }
    }
    for layer in &rev_layers {
        for &id in layer {
            reachable.insert(id);
        }
    }

    // 2. SCC condensation.
    let condensation = components(index, &reachable);
    let num_comps = condensation.components.len();

    let mut node_to_comp: HashMap<&str, usize> = HashMap::new();
    for (ci, comp) in condensation.components.iter().enumerate() {
        for member in &comp.members {
            node_to_comp.insert(member.as_str(), ci);
        }
    }

    let center_component = node_to_comp.get(center).copied().unwrap_or(0);

    // Build condensed adjacency.
    let mut comp_children: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); num_comps];
    let mut comp_parents: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); num_comps];
    let mut edges: Vec<LayoutEdge> = Vec::new();
    let mut seen_edges: HashSet<(usize, usize)> = HashSet::new();

    for ie in &condensation.inter_edges {
        let (src_c, tgt_c) = match direction {
            Direction::Forward => (ie.src_component, ie.tgt_component),
            Direction::Reverse => (ie.tgt_component, ie.src_component),
        };
        comp_children[src_c].insert(tgt_c);
        comp_parents[tgt_c].insert(src_c);
        if seen_edges.insert((src_c, tgt_c)) {
            edges.push(LayoutEdge {
                src_component: src_c,
                tgt_component: tgt_c,
                kind: ie.edge.kind.clone(),
            });
        }
    }

    // 3. Layer assignment (longest-path): roots at layer 0.
    let mut layer_of: Vec<usize> = vec![0; num_comps];
    // Kahn's topo sort.
    let mut in_degree: Vec<usize> = (0..num_comps).map(|ci| comp_parents[ci].len()).collect();
    let mut ready: VecDeque<usize> = VecDeque::new();
    for (ci, &deg) in in_degree.iter().enumerate() {
        if deg == 0 {
            ready.push_back(ci);
        }
    }
    let mut topo: Vec<usize> = Vec::with_capacity(num_comps);
    while let Some(ci) = ready.pop_front() {
        topo.push(ci);
        for &child in &comp_children[ci] {
            let new_layer = layer_of[ci] + 1;
            if new_layer > layer_of[child] {
                layer_of[child] = new_layer;
            }
            in_degree[child] -= 1;
            if in_degree[child] == 0 {
                ready.push_back(child);
            }
        }
    }

    let max_layer = layer_of.iter().copied().max().unwrap_or(0);

    // 4. Build layers and insert virtual nodes for long edges.
    let mut layers: Vec<Vec<LayoutNode>> = vec![Vec::new(); max_layer + 1];
    for &ci in &topo {
        layers[layer_of[ci]].push(LayoutNode::Real(ci));
    }

    for (ei, edge) in edges.iter().enumerate() {
        let src_layer = layer_of[edge.src_component];
        let tgt_layer = layer_of[edge.tgt_component];
        if tgt_layer > src_layer + 1 {
            for layer in &mut layers[(src_layer + 1)..tgt_layer] {
                layer.push(LayoutNode::Virtual(ei));
            }
        }
    }

    // 4b. Precompute per-component virtual-neighbour edge indices: one pass
    // over the edges, each list in edge-index order.
    let mut virtual_in: Vec<Vec<usize>> = vec![Vec::new(); num_comps];
    let mut virtual_out: Vec<Vec<usize>> = vec![Vec::new(); num_comps];
    for (ei, e) in edges.iter().enumerate() {
        if layer_of[e.src_component] + 1 < layer_of[e.tgt_component] {
            virtual_in[e.tgt_component].push(ei);
            virtual_out[e.src_component].push(ei);
        }
    }

    // 5. Crossing minimization (barycenter heuristic).
    minimize_crossings(
        &mut layers,
        &edges,
        &layer_of,
        &comp_parents,
        &comp_children,
        &virtual_in,
        &virtual_out,
    );

    // 6. PR horizontal positioning.
    let x_positions = pr_positions(
        &layers,
        &edges,
        &layer_of,
        &comp_parents,
        &comp_children,
        &virtual_in,
        &virtual_out,
    );

    GraphLayout {
        components: condensation.components,
        layers,
        edges,
        x_positions,
        center_component,
    }
}

#[cfg(test)]
mod tests;
