//! The path, orphans, counts, coverage, `at` and diff reports.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde_json::{Map, Value};

use crate::graph::LatticeGraph;
use crate::profile::{Condition, Profile};
use crate::trace::build_trace_report_for_nodes;
use crate::types::{
    AtReport, CountsReport, CoverageReport, DiffReport, EdgeChangedRef, EdgeRef, Issue,
    KindCoverage, NodeChangedRef, NodeRef, OrphanEntry, OrphansReport, PathReport,
};
use crate::validate::eval_conditions;

use super::index::{SlotIndex, check_edge_kinds, node_slot};
use super::{Direction, QueryError};

/// One shortest path from `src` to `tgt` along outgoing edges.
///
/// BFS over adjacency lists that are already sorted, so among equal-length
/// paths the one through lexicographically earlier neighbours wins. This is
/// the deterministic tie-break the spec promises.
///
/// # Errors
///
/// [`QueryError`] when `src` or `tgt` names no declared node, an edge kind is
/// not one the profile declares, or the walk breaks its own invariant. No path
/// is not an error: it is a not-found report.
pub fn path(
    graph: &LatticeGraph,
    profile: &Profile,
    src: &str,
    tgt: &str,
    edge_kinds: &[&str],
) -> Result<PathReport, QueryError> {
    path_typed(graph, profile, src, tgt, edge_kinds).map_err(|error| match error {
        PathError::Request(error) => error,
        PathError::Invariant(message) => QueryError::new(message),
    })
}

pub(crate) enum PathError {
    Request(QueryError),
    Invariant(&'static str),
}

impl From<QueryError> for PathError {
    fn from(error: QueryError) -> Self {
        Self::Request(error)
    }
}

pub(crate) fn path_typed(
    graph: &LatticeGraph,
    profile: &Profile,
    src: &str,
    tgt: &str,
    edge_kinds: &[&str],
) -> Result<PathReport, PathError> {
    let src_slot = node_slot(graph, src)?;
    let tgt_slot = node_slot(graph, tgt)?;
    check_edge_kinds(profile, edge_kinds)?;

    let index = SlotIndex::build(graph, edge_kinds, Direction::Forward);
    let mut came_from: Vec<Option<(usize, &str)>> = vec![None; index.ids.len()];
    let mut queue = VecDeque::from([src_slot]);
    let mut found = src_slot == tgt_slot;
    'walk: while let Some(slot) = queue.pop_front() {
        for &(far, kind) in index.row(slot) {
            if far == src_slot || came_from[far].is_some() {
                continue;
            }
            came_from[far] = Some((slot, kind));
            if far == tgt_slot {
                found = true;
                break 'walk;
            }
            queue.push_back(far);
        }
    }

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    if found {
        let mut at = tgt_slot;
        nodes.push(index.ids[at].to_string());
        while at != src_slot {
            let Some((prev, kind)) = came_from[at] else {
                return Err(PathError::Invariant("path walk lost its predecessor"));
            };
            edges.push(kind.to_string());
            nodes.push(index.ids[prev].to_string());
            at = prev;
        }
        nodes.reverse();
        edges.reverse();
    }

    if found {
        PathReport::found(src.to_string(), tgt.to_string(), nodes, edges)
            .map_err(PathError::Invariant)
    } else {
        Ok(PathReport::not_found(src.to_string(), tgt.to_string()))
    }
}

/// Every declared node no edge names, ordered by ID.
///
/// An edge counts for a node whenever it names that node's ID, even when its
/// far endpoint was never declared. The node is referenced, so it is not
/// standing alone; the far endpoint is `VACANCY`'s business.
///
/// # Errors
///
/// [`QueryError`] when `kind` is not a node kind the profile declares.
pub fn orphans(
    graph: &LatticeGraph,
    profile: &Profile,
    kind: Option<&str>,
    filters: &[Condition],
) -> Result<OrphansReport, QueryError> {
    if let Some(kind) = kind
        && !profile.node_kinds().contains_key(kind)
    {
        return Err(QueryError::new(format!(
            "unknown node kind '{kind}': the profile declares {:?}",
            profile.node_kinds().keys().collect::<Vec<_>>()
        )));
    }

    let (has_in, has_out) = graph.degree_flags();

    let mut entries: Vec<OrphanEntry> = graph
        .iter_nodes()
        .enumerate()
        .filter(|&(slot, _)| !has_in[slot] && !has_out[slot])
        .map(|(_, n)| n)
        .filter(|n| kind.is_none_or(|k| n.kind == k))
        .filter(|n| eval_conditions(filters, &n.attrs, true))
        .map(|n| OrphanEntry {
            id: n.id.clone(),
            kind: n.kind.clone(),
            provenance: n.provenance.clone(),
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(OrphansReport {
        kind_filter: kind.map(str::to_string),
        orphans: entries,
    })
}

/// Per-kind node and edge tallies.
///
/// Every kind the profile declares appears, zero when uninstantiated. A zero
/// edge count must be visible, not absent. A kind the register carries without
/// a declaration appears too, so counts never under-report what was ingested.
/// `filters` narrows the node tallies only: a node is counted when it satisfies
/// every condition, while edge tallies always cover the whole graph.
#[must_use]
pub fn counts(graph: &LatticeGraph, profile: &Profile, filters: &[Condition]) -> CountsReport {
    let mut nodes: BTreeMap<String, i64> = profile
        .node_kinds()
        .keys()
        .map(|k| (k.clone(), 0))
        .collect();
    let mut edges: BTreeMap<String, i64> = profile
        .edge_kinds()
        .keys()
        .map(|k| (k.clone(), 0))
        .collect();
    for node in graph.iter_nodes() {
        if eval_conditions(filters, &node.attrs, true) {
            tally(&mut nodes, &node.kind);
        }
    }
    for edge in graph.iter_edges() {
        tally(&mut edges, &edge.kind);
    }
    CountsReport { nodes, edges }
}

/// Add one to `key`'s count, allocating the key only the first time it is met
/// rather than once per element counted.
pub(crate) fn tally(counts: &mut BTreeMap<String, i64>, key: &str) {
    if let Some(count) = counts.get_mut(key) {
        *count += 1;
    } else {
        counts.insert(key.to_owned(), 1);
    }
}

/// Per-kind counts of nodes any edge enters or leaves, with percentages.
///
/// An edge counts for a node whenever it names that node's ID, on the same
/// terms as `orphans`: whether the far endpoint resolves is `VACANCY`'s
/// business. Only declared nodes are counted, so a dangling endpoint adds to
/// no kind's total. Kinds follow `counts`: every declared kind, plus any the
/// register carries undeclared.
#[must_use]
pub fn coverage(graph: &LatticeGraph, profile: &Profile) -> CoverageReport {
    let (has_in, has_out) = graph.degree_flags();
    let mut tallies: BTreeMap<&str, (i64, i64, i64)> = profile
        .node_kinds()
        .keys()
        .map(|k| (k.as_str(), (0, 0, 0)))
        .collect();
    for (slot, node) in graph.iter_nodes().enumerate() {
        let (total, incoming, outgoing) = tallies.entry(&node.kind).or_insert((0, 0, 0));
        *total += 1;
        *incoming += i64::from(has_in[slot]);
        *outgoing += i64::from(has_out[slot]);
    }
    let kinds = tallies
        .into_iter()
        .map(|(kind, (total, incoming, outgoing))| KindCoverage {
            kind: kind.to_string(),
            total,
            incoming,
            outgoing,
            incoming_pct: percent(incoming, total),
            outgoing_pct: percent(outgoing, total),
        })
        .collect();
    CoverageReport { kinds }
}

/// `part` of `whole` as a percentage rounded to one decimal; 0.0 of nothing.
fn percent(part: i64, whole: i64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    (part as f64 * 1000.0 / whole as f64).round() / 10.0
}

/// Register entries and findings originating at one source path.
///
/// `issues` is the run's full validation output; findings are answer content
/// here, never a verdict. The question is posed when the path exists under
/// the target (or as given) or any provenance in the run names it. A
/// register file the adapter reported missing must stay queryable. Anything
/// matching neither is a mistyped path, returned as a [`QueryError`] rather
/// than an empty answer. Matching is lexical: the argument as given and joined
/// to the target, equal to a provenance file or a directory prefix of one.
///
/// Unlike the service's `At` request, this reads the filesystem: it checks
/// whether the path exists under `target`.
///
/// # Errors
///
/// [`QueryError`] when the path neither exists nor names any provenance.
pub fn at(
    graph: &LatticeGraph,
    profile: &Profile,
    issues: Vec<Issue>,
    target: &Path,
    path_arg: &str,
    lattice_version: &str,
    filters: &[Condition],
) -> Result<AtReport, QueryError> {
    let arg = path_arg.trim_end_matches('/');
    let host_path_exists = target.join(arg).exists() || Path::new(arg).exists();
    at_in_view(
        graph,
        profile,
        issues,
        target,
        path_arg,
        lattice_version,
        filters,
        host_path_exists,
    )
}

// Retain lexical selection in the core; only the caller may probe a filesystem.
#[allow(clippy::too_many_arguments)]
pub(crate) fn at_in_view(
    graph: &LatticeGraph,
    profile: &Profile,
    issues: Vec<Issue>,
    target: &Path,
    path_arg: &str,
    lattice_version: &str,
    filters: &[Condition],
    host_path_exists: bool,
) -> Result<AtReport, QueryError> {
    let arg = path_arg.trim_end_matches('/');
    let joined = target.join(arg).to_string_lossy().into_owned();
    let mut candidates = vec![arg.to_string(), joined];
    // An absolute argument beneath the target must also match provenance an
    // adapter wrote target-relative. Without this, the absolute form of a
    // populated file reads as a clean empty answer.
    let rel = Path::new(arg).strip_prefix(target).ok();
    if let Some(rel) = rel {
        candidates.push(rel.to_string_lossy().into_owned());
    }
    // The target root itself (`.` or the target's own path) is beneath no
    // candidate string, yet every target-relative provenance lies under it.
    let is_root = matches!(arg, "" | ".") || rel.is_some_and(|rel| rel.as_os_str().is_empty());
    let matches = |file: &str| {
        (is_root && !Path::new(file).is_absolute())
            || candidates.iter().any(|c| {
                file == c
                    || file
                        .strip_prefix(c.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
    };

    let posed = host_path_exists
        || graph.iter_nodes().any(|n| matches(n.provenance.file()))
        || graph.iter_edges().any(|e| matches(e.provenance.file()))
        || issues
            .iter()
            .any(|i| i.origin.location().is_some_and(|at| matches(at.file())));
    if !posed {
        return Err(QueryError::new(format!(
            "path '{path_arg}' does not exist under the target and no provenance names it"
        )));
    }

    let node_ids: BTreeSet<&str> = graph
        .iter_nodes()
        .filter(|node| matches(node.provenance.file()))
        .filter(|node| eval_conditions(filters, &node.attrs, true))
        .map(|node| node.id.as_str())
        .collect();
    let report =
        build_trace_report_for_nodes(profile, graph, issues, lattice_version, Some(&node_ids));
    let mut entries = Vec::new();
    let mut findings = Vec::new();
    for entry in report.entries {
        if matches(entry.provenance.file()) {
            entries.push(entry);
        } else {
            findings.extend(
                entry
                    .findings
                    .into_iter()
                    .filter(|i| i.origin.location().is_some_and(|at| matches(at.file()))),
            );
        }
    }
    findings.extend(
        report
            .unattachable_findings
            .into_iter()
            .filter(|i| i.origin.location().is_some_and(|at| matches(at.file()))),
    );
    findings.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    Ok(AtReport {
        path: path_arg.to_string(),
        entries,
        findings,
    })
}

/// Compare two ingested registers by semantic identity, provenance excluded.
///
/// Nodes compare by ID → (kind, attrs); edges by (src, tgt, kind) → attrs,
/// with parallel edges paired in document order; pathways by name → (order,
/// current). A declaration that merely moved lines therefore does not diff;
/// line numbers are where a thing was said, not what was said.
#[must_use]
pub fn diff(rev_a: &str, a: &LatticeGraph, rev_b: &str, b: &LatticeGraph) -> DiffReport {
    // Both registers outlive the answer, so the identities compare as borrowed
    // views of them: a diff of two large registers otherwise duplicated every
    // node's attrs just to compare them.
    fn nodes_of(g: &LatticeGraph) -> BTreeMap<&str, (&str, &Map<String, Value>)> {
        g.iter_nodes()
            .map(|n| (n.id.as_str(), (n.kind.as_str(), &n.attrs)))
            .collect()
    }
    let nodes_a = nodes_of(a);
    let nodes_b = nodes_of(b);

    let node_ref = |id: &str, kind: &str| NodeRef {
        id: id.to_string(),
        kind: kind.to_string(),
        tainted: None,
    };
    let mut nodes_added = Vec::new();
    let mut nodes_changed = Vec::new();
    for (id, (kind, attrs)) in &nodes_b {
        match nodes_a.get(id) {
            None => nodes_added.push(node_ref(id, kind)),
            Some((kind_a, attrs_a)) if kind_a != kind || attrs_a != attrs => {
                nodes_changed.push(NodeChangedRef {
                    id: id.to_string(),
                    kind: kind.to_string(),
                    attrs_a: (*attrs_a).clone(),
                    attrs_b: (*attrs).clone(),
                });
            }
            Some(_) => {}
        }
    }
    let nodes_removed = nodes_a
        .iter()
        .filter(|(id, _)| !nodes_b.contains_key(*id))
        .map(|(id, (kind, _))| node_ref(id, kind))
        .collect();

    // Attrs per identity tuple in document order, so parallel edges pair up
    // positionally rather than being matched by content. Content is what
    // the pairing is meant to compare.
    type EdgeAttrs<'g> = BTreeMap<(&'g str, &'g str, &'g str), Vec<&'g Map<String, Value>>>;
    fn edges_of(g: &LatticeGraph) -> EdgeAttrs<'_> {
        let mut by_tuple: EdgeAttrs<'_> = BTreeMap::new();
        for e in g.iter_edges() {
            by_tuple
                .entry((e.src.as_str(), e.tgt.as_str(), e.kind.as_str()))
                .or_default()
                .push(&e.attrs);
        }
        by_tuple
    }
    let edges_a = edges_of(a);
    let edges_b = edges_of(b);
    let mut edges_added = Vec::new();
    let mut edges_removed = Vec::new();
    let mut edges_changed = Vec::new();
    let keys: BTreeSet<_> = edges_a.keys().chain(edges_b.keys()).collect();
    let edge_ref = |key: &(&str, &str, &str)| EdgeRef {
        src: key.0.to_string(),
        tgt: key.1.to_string(),
        kind: key.2.to_string(),
    };
    for key in keys {
        let in_a = edges_a.get(key).map_or(&[][..], Vec::as_slice);
        let in_b = edges_b.get(key).map_or(&[][..], Vec::as_slice);
        for (attrs_a, attrs_b) in in_a.iter().zip(in_b) {
            if attrs_a != attrs_b {
                edges_changed.push(EdgeChangedRef {
                    src: key.0.to_string(),
                    tgt: key.1.to_string(),
                    kind: key.2.to_string(),
                    attrs_a: (*attrs_a).clone(),
                    attrs_b: (*attrs_b).clone(),
                });
            }
        }
        let paired = in_a.len().min(in_b.len());
        edges_added.extend((paired..in_b.len()).map(|_| edge_ref(key)));
        edges_removed.extend((paired..in_a.len()).map(|_| edge_ref(key)));
    }

    let pathway_names: BTreeSet<&str> = a
        .iter_pathways()
        .chain(b.iter_pathways())
        .map(|pathway| pathway.name.as_str())
        .collect();
    let pathways_changed = pathway_names
        .into_iter()
        .filter(|name| a.pathway(name) != b.pathway(name))
        .map(str::to_string)
        .collect();

    DiffReport {
        rev_a: rev_a.to_string(),
        rev_b: rev_b.to_string(),
        nodes_added,
        nodes_removed,
        nodes_changed,
        edges_added,
        edges_removed,
        edges_changed,
        pathways_changed,
    }
}
