//! The in-memory register: nodes, edges, pathways and accumulated adapter issues.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Map, Value};

use latt_adapter::SortedMap;

use crate::types::{Issue, Provenance};

/// An ordering pathway read from the target: its positions and where it stands now.
///
/// Ingested data, on the same footing as nodes and edges. The register declares
/// it and the adapter reads it. Nothing here is computed or defaulted.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pathway {
    /// The name a profile validation entry binds a code to.
    pub name: String,
    /// The positions, earliest first; unique, as [`check_pathway`] requires.
    pub order: Vec<String>,
    /// Where the target stands now, read from the register — never from the profile.
    pub current: String,
}

impl Pathway {
    /// True when `position` is one of this pathway's declared positions.
    #[must_use]
    pub fn is_member(&self, position: &str) -> bool {
        self.order.iter().any(|p| p == position)
    }

    /// True when `position` sits strictly later on the pathway than `current`.
    ///
    /// False for a non-member, which callers must screen with `is_member` first;
    /// the two cases mean different things and share no answer.
    #[must_use]
    pub fn is_after(&self, position: &str) -> bool {
        let (Some(at), Some(now)) = (self.index_of(position), self.index_of(&self.current)) else {
            return false;
        };
        at > now
    }

    fn index_of(&self, position: &str) -> Option<usize> {
        self.order.iter().position(|p| p == position)
    }
}

/// A pathway the graph refuses: repeated positions, or a `current` outside the order.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
#[non_exhaustive]
pub struct PathwayError {
    message: String,
}

impl PathwayError {
    fn new(message: String) -> Self {
        Self { message }
    }

    /// Return the explanation, the same text `Display` writes. It is for
    /// people and may change editorially; do not parse it.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Refuse a pathway declaration rather than repairing it.
///
/// Repeated positions leave "before" and "after" depending on which occurrence
/// matched, and a current position outside the order places the pathway nowhere.
///
/// # Errors
///
/// [`PathwayError`] when `order` repeats a position or `current` is not in it.
pub fn check_pathway(name: &str, order: &[String], current: &str) -> Result<(), PathwayError> {
    let unique: HashSet<&String> = order.iter().collect();
    if unique.len() != order.len() {
        return Err(PathwayError::new(format!(
            "pathway '{name}': positions are not unique: {order:?}"
        )));
    }
    if !order.iter().any(|p| p == current) {
        return Err(PathwayError::new(format!(
            "pathway '{name}': current position '{current}' is not in {order:?}"
        )));
    }
    Ok(())
}

/// A repeated node ID, carrying both provenances so the caller can report them.
///
/// Returned rather than swallowed: ingest turns it into a finding, and a builder
/// that resolved it here would hide the repeat from the core that reports it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("duplicate node '{node_id}': first at {existing_provenance}, second at {new_provenance}")]
#[non_exhaustive]
pub struct DuplicateNode {
    /// The ID both declarations carry.
    pub node_id: String,
    /// Where the node kept in the graph was declared.
    pub existing_provenance: Provenance,
    /// Where the refused repeat was declared.
    pub new_provenance: Provenance,
}

/// A node as stored: its declared kind, attrs and provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Node {
    /// The public join key, unique within the graph.
    pub id: String,
    /// A node kind the profile declares, or a kind validation reports as unknown.
    pub kind: String,
    /// The register's attributes for this node, as the adapter emitted them.
    pub attrs: Map<String, Value>,
    /// Where in the target the node is declared.
    pub provenance: Provenance,
    /// Where each attr was written, for the attrs the adapter located. Private so
    /// the representation, one compact map per node, can change without a break.
    attr_provenance: SortedMap<String, Provenance>,
}

impl Node {
    /// Return where `attr` was written: its own location when the adapter gave
    /// one, else the node's. A finding about one attr is reported here, so a user
    /// lands on the value rather than on the table holding it.
    #[must_use]
    pub fn provenance_of(&self, attr: &str) -> &Provenance {
        self.attr_provenance.get(attr).unwrap_or(&self.provenance)
    }

    /// Return the location the adapter gave `attr`, or `None` when it gave none
    /// and the attr was written at the node's [`provenance`](Self::provenance).
    #[must_use]
    pub fn attr_location(&self, attr: &str) -> Option<&Provenance> {
        self.attr_provenance.get(attr)
    }

    /// Iterate the attrs the adapter located and where each was written, in
    /// attr-name order. An attr not named here was written at the node's
    /// [`provenance`](Self::provenance).
    ///
    /// ```
    /// use latt_core::document::{ingest_document, parse_document};
    ///
    /// let graph = ingest_document(parse_document(r#"{"interface_version": "1.4", "nodes": [
    ///     {"id": "R-1", "kind": "req", "attrs": {"title": "t", "status": "draft"},
    ///      "provenance": {"file": "r.toml", "line": 1},
    ///      "attr_provenance": {"status": {"file": "r.toml", "line": 3}}}]}"#)?)?;
    /// let node = graph.node("R-1").ok_or("no node")?;
    /// let located: Vec<(&str, Option<u32>)> =
    ///     node.attr_provenance().map(|(a, p)| (a, p.line().map(u32::from))).collect();
    /// assert_eq!(located, [("status", Some(3))]);
    /// assert_eq!(node.attr_location("title"), None);
    /// assert_eq!(node.provenance_of("title").line().map(u32::from), Some(1));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn attr_provenance(
        &self,
    ) -> impl DoubleEndedIterator<Item = (&str, &Provenance)> + ExactSizeIterator {
        self.attr_provenance
            .iter()
            .map(|(attr, provenance)| (attr.as_str(), provenance))
    }
}

/// An edge as stored. Parallel edges are distinct, each keeping its own provenance.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Edge {
    /// The source node ID. It may name a node never declared, which validation
    /// reports as `VACANCY`.
    pub src: String,
    /// The target node ID, which may likewise be dangling.
    pub tgt: String,
    /// An edge kind the profile declares, or a kind validation reports as unknown.
    pub kind: String,
    /// The register's attributes for this edge, as the adapter emitted them.
    pub attrs: Map<String, Value>,
    /// Where in the target the edge is declared.
    pub provenance: Provenance,
}

/// The named inputs needed to add an edge to a graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeSpec {
    /// As [`Edge::src`].
    pub src: String,
    /// As [`Edge::tgt`].
    pub tgt: String,
    /// As [`Edge::kind`].
    pub kind: String,
    /// As [`Edge::attrs`].
    pub attrs: Map<String, Value>,
}

/// A typed node/edge register with provenance and accumulated parse issues.
///
/// An edge may name a target no adapter declared, so edges hold endpoint IDs as
/// plain strings and never materialize a node. `iter_nodes` therefore yields only
/// explicitly added nodes, and the unresolved reference surfaces as a VACANCY
/// at validation rather than as a phantom node.
///
/// Equality is ingest identity: nodes, edges and issues compare in insertion
/// order and with provenance. Two ingests of the same register compare unequal
/// after a line shift or a reordering that `query::diff` reports as no change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LatticeGraph {
    nodes: Vec<Node>,
    index: HashMap<String, usize>,
    edges: Vec<Edge>,
    issues: Vec<Issue>,
    pathways: BTreeMap<String, Pathway>,
}

impl LatticeGraph {
    /// Construct an empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node, or report the repeat. First declaration wins.
    ///
    /// # Errors
    ///
    /// [`DuplicateNode`] when the ID is already declared; the graph keeps the
    /// first node unchanged.
    pub fn add_node(
        &mut self,
        id: impl Into<String>,
        kind: impl Into<String>,
        attrs: Map<String, Value>,
        provenance: Provenance,
    ) -> Result<(), DuplicateNode> {
        self.add_located_node(id.into(), kind.into(), attrs, provenance, SortedMap::new())
    }

    /// Add a node with where its attrs were written; see [`Node::provenance_of`].
    /// Taking the locations with the node spares ingestion a second lookup by ID.
    /// Repeats are refused as by [`add_node`](Self::add_node).
    pub(crate) fn add_located_node(
        &mut self,
        id: String,
        kind: String,
        attrs: Map<String, Value>,
        provenance: Provenance,
        attr_provenance: SortedMap<String, Provenance>,
    ) -> Result<(), DuplicateNode> {
        if let Some(&at) = self.index.get(&id) {
            return Err(DuplicateNode {
                node_id: id,
                existing_provenance: self.nodes[at].provenance.clone(),
                new_provenance: provenance,
            });
        }
        self.index.insert(id.clone(), self.nodes.len());
        self.nodes.push(Node {
            id,
            kind,
            attrs,
            provenance,
            attr_provenance,
        });
        Ok(())
    }

    /// Add an edge while retaining its source provenance.
    pub fn add_edge(&mut self, edge: EdgeSpec, provenance: Provenance) {
        self.edges.push(Edge {
            src: edge.src,
            tgt: edge.tgt,
            kind: edge.kind,
            attrs: edge.attrs,
            provenance,
        });
    }

    /// Attach one adapter issue to the graph.
    pub fn add_issue(&mut self, issue: Issue) {
        self.issues.push(issue);
    }

    /// Attach an ordering pathway the adapter read from the target, or refuse
    /// it on `check_pathway`'s terms. A valid declaration replaces any pathway
    /// already attached under the same name.
    ///
    /// ```
    /// use latt_core::graph::LatticeGraph;
    ///
    /// let stages = |s: &[&str]| s.iter().map(|&p| p.to_owned()).collect::<Vec<_>>();
    /// let mut graph = LatticeGraph::new();
    /// graph.set_pathway("stage", stages(&["M0", "M1"]), "M0")?;
    /// graph.set_pathway("stage", stages(&["M0", "M1", "M2"]), "M2")?;
    /// assert!(graph.pathway("stage").is_some_and(|p| p.is_member("M2")));
    ///
    /// assert!(graph.set_pathway("stage", stages(&["M0", "M0"]), "M0").is_err());
    /// assert!(graph.pathway("stage").is_some_and(|p| p.is_member("M2")));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`PathwayError`] when the positions repeat or `current` is not among
    /// them; the graph is left unchanged.
    pub fn set_pathway(
        &mut self,
        name: impl Into<String>,
        order: Vec<String>,
        current: impl Into<String>,
    ) -> Result<(), PathwayError> {
        let name = name.into();
        let current = current.into();
        check_pathway(&name, &order, &current)?;
        self.pathways.insert(
            name.clone(),
            Pathway {
                name,
                order,
                current,
            },
        );
        Ok(())
    }

    /// Return a pathway by its declared name.
    pub fn pathway(&self, name: &str) -> Option<&Pathway> {
        self.pathways.get(name)
    }

    /// Yield the attached pathways, in name order.
    pub fn iter_pathways(&self) -> impl Iterator<Item = &Pathway> {
        self.pathways.values()
    }

    /// Return adapter issues in insertion order.
    pub fn adapter_issues(&self) -> &[Issue] {
        &self.issues
    }

    /// Report whether an explicitly declared node has this ID.
    pub fn has_node(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    /// Return an explicitly declared node by ID.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.index.get(id).map(|&at| &self.nodes[at])
    }

    /// Yield explicitly added nodes, in the order they were added.
    pub fn iter_nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }

    /// The position of a declared node in [`iter_nodes`](Self::iter_nodes) order.
    /// Stable for as long as the graph is borrowed, which is what lets a single
    /// call index nodes by position instead of hashing their IDs again.
    pub(crate) fn slot(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    pub(crate) fn node_at(&self, slot: usize) -> &Node {
        &self.nodes[slot]
    }

    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Whether each declared node has an incoming and an outgoing edge, by slot.
    ///
    /// An edge counts for a node whenever it names the node's ID, whether or not
    /// its far end is declared: the same test as `AdjacencyIndex`'s
    /// `has_incoming`/`has_outgoing`, without building the index.
    pub(crate) fn degree_flags(&self) -> (Vec<bool>, Vec<bool>) {
        let mut has_in = vec![false; self.nodes.len()];
        let mut has_out = vec![false; self.nodes.len()];
        for edge in &self.edges {
            if let Some(src) = self.slot(&edge.src) {
                has_out[src] = true;
            }
            if let Some(tgt) = self.slot(&edge.tgt) {
                has_in[tgt] = true;
            }
        }
        (has_in, has_out)
    }

    /// Yield edges in the order they were added.
    pub fn iter_edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.iter()
    }
}
