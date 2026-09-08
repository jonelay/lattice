//! Ask-time graph queries: reachability, paths, orphans and counts.
//!
//! The graph stores flat node and edge vectors; every index here is built per
//! invocation from a borrowed graph and dropped with the answer. Nothing is
//! cached or written back — a stored index would be derived state at rest.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

use serde_json::{Map, Value};

use crate::graph::{Edge, LatticeGraph};
use crate::profile::Profile;
use crate::trace::build_trace_report_for_nodes;
use crate::types::{
    AtReport, CountsReport, DiffReport, EdgeRef, Issue, NodeRef, OrphanEntry, OrphansReport,
    PathReport, ReachReport,
};

/// A question that cannot be posed: an ID or kind the run does not know.
///
/// Operational, not a finding — the caller reports it and exits 2.
#[derive(Debug)]
pub struct QueryError(pub String);

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for QueryError {}

/// Which way a reachability walk follows edges.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Along outgoing edges: what does this node reach?
    Forward,
    /// Along incoming edges: what reaches this node?
    Reverse,
}

/// Forward and reverse adjacency over a borrowed graph.
///
/// Adjacency lists are sorted by (far endpoint, edge kind) so every walk
/// expands neighbours in one order and answers are deterministic.
struct Adjacency<'a> {
    forward: HashMap<&'a str, Vec<&'a Edge>>,
    reverse: HashMap<&'a str, Vec<&'a Edge>>,
}

impl<'a> Adjacency<'a> {
    fn build(graph: &'a LatticeGraph) -> Self {
        let mut forward: HashMap<&str, Vec<&Edge>> = HashMap::new();
        let mut reverse: HashMap<&str, Vec<&Edge>> = HashMap::new();
        for edge in graph.iter_edges() {
            forward.entry(&edge.src).or_default().push(edge);
            reverse.entry(&edge.tgt).or_default().push(edge);
        }
        for list in forward.values_mut() {
            list.sort_by_key(|e| (&e.tgt, &e.kind));
        }
        for list in reverse.values_mut() {
            list.sort_by_key(|e| (&e.src, &e.kind));
        }
        Self { forward, reverse }
    }

    /// Edges leaving `id` in the walk's direction, restricted to `kinds` when
    /// the set is non-empty. Each yielded pair is (edge kind, far endpoint).
    fn neighbours<'b>(
        &'b self,
        id: &str,
        direction: Direction,
        kinds: &'b BTreeSet<String>,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + 'b {
        let map = match direction {
            Direction::Forward => &self.forward,
            Direction::Reverse => &self.reverse,
        };
        map.get(id)
            .into_iter()
            .flatten()
            .filter(move |e| kinds.is_empty() || kinds.contains(&e.kind))
            .map(move |e| {
                let far = match direction {
                    Direction::Forward => e.tgt.as_str(),
                    Direction::Reverse => e.src.as_str(),
                };
                (e.kind.as_str(), far)
            })
    }
}

/// Refuse an edge kind the profile does not declare, before any traversal.
fn check_edge_kinds(profile: &Profile, kinds: &BTreeSet<String>) -> Result<(), QueryError> {
    for kind in kinds {
        if !profile.edge_kinds().contains_key(kind) {
            return Err(QueryError(format!(
                "unknown edge kind '{kind}': the profile declares {:?}",
                profile.edge_kinds().keys().collect::<Vec<_>>()
            )));
        }
    }
    Ok(())
}

/// Refuse a node ID the graph does not hold, before any traversal.
fn check_node(graph: &LatticeGraph, id: &str) -> Result<(), QueryError> {
    if graph.has_node(id) {
        Ok(())
    } else {
        Err(QueryError(format!("unknown node '{id}'")))
    }
}

/// The transitive closure from `origin`, excluding the origin itself.
///
/// The walk follows edge endpoints as plain strings, so it traverses *through*
/// an endpoint no adapter declared; only declared nodes enter the answer — an
/// undeclared one is a dangling reference, not a phantom node.
pub fn reach(
    graph: &LatticeGraph,
    profile: &Profile,
    origin: &str,
    edge_kinds: &BTreeSet<String>,
    direction: Direction,
) -> Result<ReachReport, QueryError> {
    check_node(graph, origin)?;
    check_edge_kinds(profile, edge_kinds)?;

    let adjacency = Adjacency::build(graph);
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut queue: VecDeque<&str> = VecDeque::from([origin]);
    seen.insert(origin);
    while let Some(id) = queue.pop_front() {
        for (_, far) in adjacency.neighbours(id, direction, edge_kinds) {
            if seen.insert(far) {
                queue.push_back(far);
            }
        }
    }
    seen.remove(origin);

    Ok(ReachReport {
        origin: origin.to_string(),
        direction: match direction {
            Direction::Forward => "reaches",
            Direction::Reverse => "reached-by",
        }
        .to_string(),
        edge_kinds: edge_kinds.iter().cloned().collect(),
        nodes: seen
            .iter()
            .filter_map(|id| graph.node(id))
            .map(|n| NodeRef {
                id: n.id.clone(),
                kind: n.kind.clone(),
            })
            .collect(),
    })
}

/// One shortest path from `src` to `tgt` along outgoing edges.
///
/// BFS over adjacency lists that are already sorted, so among equal-length
/// paths the one through lexicographically earlier neighbours wins — the
/// deterministic tie-break the spec promises.
pub fn path(
    graph: &LatticeGraph,
    profile: &Profile,
    src: &str,
    tgt: &str,
    edge_kinds: &BTreeSet<String>,
) -> Result<PathReport, QueryError> {
    check_node(graph, src)?;
    check_node(graph, tgt)?;
    check_edge_kinds(profile, edge_kinds)?;

    let adjacency = Adjacency::build(graph);
    // Predecessor of each visited node: (previous node, edge kind into here).
    let mut came_from: HashMap<&str, (&str, &str)> = HashMap::new();
    let mut queue: VecDeque<&str> = VecDeque::from([src]);
    let mut found = src == tgt;
    'walk: while let Some(id) = queue.pop_front() {
        for (kind, far) in adjacency.neighbours(id, Direction::Forward, edge_kinds) {
            if far == src || came_from.contains_key(far) {
                continue;
            }
            came_from.insert(far, (id, kind));
            if far == tgt {
                found = true;
                break 'walk;
            }
            queue.push_back(far);
        }
    }

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    if found {
        let mut at = tgt;
        nodes.push(at.to_string());
        while at != src {
            let (prev, kind) = came_from[at];
            edges.push(kind.to_string());
            nodes.push(prev.to_string());
            at = prev;
        }
        nodes.reverse();
        edges.reverse();
    }

    if found {
        PathReport::found(src.to_string(), tgt.to_string(), nodes, edges)
            .map_err(|message| QueryError(message.to_owned()))
    } else {
        Ok(PathReport::not_found(src.to_string(), tgt.to_string()))
    }
}

/// Every declared node no edge names, ordered by ID.
///
/// An edge counts for a node whenever it names that node's ID, even when its
/// far endpoint was never declared — the node is referenced, so it is not
/// standing alone; the far endpoint is `DANGLING_REF`'s business.
pub fn orphans(
    graph: &LatticeGraph,
    profile: &Profile,
    kind: Option<&str>,
) -> Result<OrphansReport, QueryError> {
    if let Some(kind) = kind
        && !profile.node_kinds().contains_key(kind)
    {
        return Err(QueryError(format!(
            "unknown node kind '{kind}': the profile declares {:?}",
            profile.node_kinds().keys().collect::<Vec<_>>()
        )));
    }

    let mut named: BTreeSet<&str> = BTreeSet::new();
    for edge in graph.iter_edges() {
        named.insert(&edge.src);
        named.insert(&edge.tgt);
    }

    let mut entries: Vec<OrphanEntry> = graph
        .iter_nodes()
        .filter(|n| !named.contains(n.id.as_str()))
        .filter(|n| kind.is_none_or(|k| n.kind == k))
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
/// Every kind the profile declares appears, zero when uninstantiated — a zero
/// edge count must be visible, not absent. A kind the register carries without
/// a declaration appears too, so counts never under-report what was ingested.
pub fn counts(graph: &LatticeGraph, profile: &Profile) -> CountsReport {
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
        *nodes.entry(node.kind.clone()).or_insert(0) += 1;
    }
    for edge in graph.iter_edges() {
        *edges.entry(edge.kind.clone()).or_insert(0) += 1;
    }
    CountsReport { nodes, edges }
}

/// Register entries and findings originating at one source path.
///
/// `issues` is the run's full validation output; findings are answer content
/// here, never a verdict. The question is posed when the path exists under
/// the target (or as given) or any provenance in the run names it — a
/// register file the adapter reported missing must stay queryable. Anything
/// matching neither is a mistyped path, which is exit 2 rather than an
/// empty answer. Matching is lexical: the argument as given and joined to
/// the target, equal to a provenance file or a directory prefix of one.
pub fn at(
    graph: &LatticeGraph,
    profile: &Profile,
    issues: Vec<Issue>,
    target: &Path,
    path_arg: &str,
    lattice_version: &str,
) -> Result<AtReport, QueryError> {
    let arg = path_arg.trim_end_matches('/');
    let joined = target.join(arg).to_string_lossy().into_owned();
    let mut candidates = vec![arg.to_string(), joined];
    // An absolute argument beneath the target must also match provenance an
    // adapter wrote target-relative — without this, the absolute form of a
    // populated file reads as a clean empty answer.
    if let Ok(rel) = Path::new(arg).strip_prefix(target) {
        candidates.push(rel.to_string_lossy().into_owned());
    }
    let matches = |file: &str| {
        candidates.iter().any(|c| {
            file == c
                || file
                    .strip_prefix(c.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    };

    let posed = target.join(arg).exists()
        || Path::new(arg).exists()
        || graph.iter_nodes().any(|n| matches(&n.provenance.file))
        || graph.iter_edges().any(|e| matches(&e.provenance.file))
        || issues.iter().any(|i| matches(&i.provenance.file));
    if !posed {
        return Err(QueryError(format!(
            "path '{path_arg}' does not exist under the target and no provenance names it"
        )));
    }

    let node_ids: BTreeSet<&str> = graph
        .iter_nodes()
        .filter(|node| matches(&node.provenance.file))
        .map(|node| node.id.as_str())
        .collect();
    let report =
        build_trace_report_for_nodes(profile, graph, issues, lattice_version, Some(&node_ids));
    let mut entries = Vec::new();
    let mut findings = Vec::new();
    for entry in report.entries {
        if matches(&entry.provenance.file) {
            entries.push(entry);
        } else {
            findings.extend(
                entry
                    .findings
                    .into_iter()
                    .filter(|i| matches(&i.provenance.file)),
            );
        }
    }
    findings.extend(
        report
            .unattachable_findings
            .into_iter()
            .filter(|i| matches(&i.provenance.file)),
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
/// Nodes compare by ID → (kind, attrs); edges as a multiset of (src, tgt,
/// kind); axes by name → (order, current). A declaration that merely moved
/// lines therefore does not diff — line numbers are where a thing was said,
/// not what was said.
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
    };
    let mut nodes_added = Vec::new();
    let mut nodes_changed = Vec::new();
    for (id, (kind, attrs)) in &nodes_b {
        match nodes_a.get(id) {
            None => nodes_added.push(node_ref(id, kind)),
            Some((kind_a, attrs_a)) if kind_a != kind || attrs_a != attrs => {
                nodes_changed.push(node_ref(id, kind));
            }
            Some(_) => {}
        }
    }
    let nodes_removed = nodes_a
        .iter()
        .filter(|(id, _)| !nodes_b.contains_key(*id))
        .map(|(id, (kind, _))| node_ref(id, kind))
        .collect();

    fn edges_of(g: &LatticeGraph) -> BTreeMap<(&str, &str, &str), i64> {
        let mut multiset = BTreeMap::new();
        for e in g.iter_edges() {
            *multiset
                .entry((e.src.as_str(), e.tgt.as_str(), e.kind.as_str()))
                .or_insert(0) += 1;
        }
        multiset
    }
    let edges_a = edges_of(a);
    let edges_b = edges_of(b);
    let mut edges_added = Vec::new();
    let mut edges_removed = Vec::new();
    let keys: BTreeSet<_> = edges_a.keys().chain(edges_b.keys()).collect();
    for key in keys {
        let surplus = edges_b.get(key).unwrap_or(&0) - edges_a.get(key).unwrap_or(&0);
        let (list, count) = if surplus > 0 {
            (&mut edges_added, surplus)
        } else {
            (&mut edges_removed, -surplus)
        };
        for _ in 0..count {
            list.push(EdgeRef {
                src: key.0.to_string(),
                tgt: key.1.to_string(),
                kind: key.2.to_string(),
            });
        }
    }

    let axis_names: BTreeSet<&str> = a
        .iter_axes()
        .chain(b.iter_axes())
        .map(|axis| axis.name.as_str())
        .collect();
    let axes_changed = axis_names
        .into_iter()
        .filter(|name| a.axis(name) != b.axis(name))
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
        axes_changed,
    }
}

/// One directory both revisions are materialized into, removed on drop.
///
/// One path, not one per revision, and that is load-bearing: adapters embed
/// the target path in attrs (an adapter's `file` attr can), so two
/// materialization directories would make every such node "changed" in a diff
/// of identical content. Extractions are sequential; the second replaces the
/// first.
pub struct MaterializationDir {
    path: PathBuf,
}

impl MaterializationDir {
    pub fn new() -> Result<Self, QueryError> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "lattice-diff-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path)
            .map_err(|e| QueryError(format!("could not create {}: {e}", path.display())))?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for MaterializationDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Extract the target's committed tree at `rev` into `into`, replacing
/// whatever revision was extracted there before.
///
/// `git archive` piped through `tar -x`, chosen over a worktree because it
/// touches none of the target's git state — there is nothing to deregister,
/// and the working tree stays exactly as the user left it.
pub fn materialize_revision(
    target: &Path,
    rev: &str,
    into: &MaterializationDir,
) -> Result<(), QueryError> {
    let _ = std::fs::remove_dir_all(into.path());
    std::fs::create_dir_all(into.path())
        .map_err(|e| QueryError(format!("could not create {}: {e}", into.path().display())))?;

    let mut archive = Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["archive", rev])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| QueryError(format!("could not run git: {e}")))?;
    let tar_status = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(into.path())
        .stdin(Stdio::from(
            archive.stdout.take().expect("stdout was piped"),
        ))
        .status()
        .map_err(|e| QueryError(format!("could not run tar: {e}")))?;
    let archive_output = archive
        .wait_with_output()
        .map_err(|e| QueryError(format!("could not wait for git: {e}")))?;

    if !archive_output.status.success() {
        let detail = String::from_utf8_lossy(&archive_output.stderr);
        return Err(QueryError(format!(
            "git archive '{rev}' failed in {}: {}",
            target.display(),
            detail.trim()
        )));
    }
    if !tar_status.success() {
        return Err(QueryError(format!("tar failed extracting '{rev}'")));
    }
    Ok(())
}
