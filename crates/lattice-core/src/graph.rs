//! The in-memory register: nodes, edges, pathways and accumulated adapter issues.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Map, Value};

use crate::types::{Issue, Provenance};

/// An ordering pathway read from the target: its positions and where it stands now.
///
/// Ingested data, on the same footing as nodes and edges — the register declares
/// it and the adapter reads it. Nothing here is computed or defaulted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pathway {
    pub name: String,
    pub order: Vec<String>,
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
    /// False for a non-member, which callers must screen with `is_member` first —
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
#[derive(Debug)]
pub struct PathwayError(pub String);

impl std::fmt::Display for PathwayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PathwayError {}

/// Refuse a pathway declaration rather than repairing it: repeated positions
/// leave "before" and "after" depending on which occurrence matched, and a
/// current position outside the order places the pathway nowhere.
pub fn check_pathway(name: &str, order: &[String], current: &str) -> Result<(), PathwayError> {
    let unique: HashSet<&String> = order.iter().collect();
    if unique.len() != order.len() {
        return Err(PathwayError(format!(
            "pathway '{name}': positions are not unique: {order:?}"
        )));
    }
    if !order.iter().any(|p| p == current) {
        return Err(PathwayError(format!(
            "pathway '{name}': current position '{current}' is not in {order:?}"
        )));
    }
    Ok(())
}

/// A repeated node ID, carrying both provenances so the caller can report them.
///
/// Returned rather than swallowed: ingest turns it into a finding, and a builder
/// that resolved it here would hide the repeat from the core that reports it.
#[derive(Debug)]
pub struct DuplicateNode {
    pub node_id: String,
    pub existing_provenance: Provenance,
    pub new_provenance: Provenance,
}

impl std::fmt::Display for DuplicateNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "duplicate node '{}': first at {}, second at {}",
            self.node_id, self.existing_provenance, self.new_provenance
        )
    }
}

impl std::error::Error for DuplicateNode {}

/// A node as stored: its declared kind, attrs and provenance.
#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub attrs: Map<String, Value>,
    pub provenance: Provenance,
}

/// Which nodes participate in edges, indexed once from a borrowed graph.
///
/// Both endpoints of every edge count as connected, whether or not the
/// register declared them. An edge is evidence the node is referred to, which
/// is the question UNREFERENCED/UNTRACED asks; whether the endpoint resolves is
/// VACANCY's question, and answering it here would report one fault twice.
#[derive(Debug)]
pub struct EdgeIndex<'a> {
    incoming: HashSet<&'a str>,
    outgoing: HashSet<&'a str>,
}

impl<'a> EdgeIndex<'a> {
    pub fn build(graph: &'a LatticeGraph) -> Self {
        let mut incoming = HashSet::new();
        let mut outgoing = HashSet::new();
        for edge in graph.iter_edges() {
            outgoing.insert(edge.src.as_str());
            incoming.insert(edge.tgt.as_str());
        }
        Self { incoming, outgoing }
    }

    #[must_use]
    pub fn has_incoming(&self, id: &str) -> bool {
        self.incoming.contains(id)
    }

    #[must_use]
    pub fn has_outgoing(&self, id: &str) -> bool {
        self.outgoing.contains(id)
    }

    #[must_use]
    pub fn connected(&self, id: &str) -> bool {
        self.has_incoming(id) || self.has_outgoing(id)
    }
}

/// An edge as stored. Parallel edges are distinct, each keeping its own provenance.
#[derive(Clone, Debug)]
pub struct Edge {
    pub src: String,
    pub tgt: String,
    pub kind: String,
    pub attrs: Map<String, Value>,
    pub provenance: Provenance,
}

/// The named inputs needed to add an edge to a graph.
#[derive(Clone, Debug)]
pub struct EdgeSpec {
    pub src: String,
    pub tgt: String,
    pub kind: String,
    pub attrs: Map<String, Value>,
}

/// A typed node/edge register with provenance and accumulated parse issues.
///
/// An edge may name a target no adapter declared, so edges hold endpoint IDs as
/// plain strings and never materialize a node. `iter_nodes` therefore yields only
/// explicitly added nodes, and the unresolved reference surfaces as a VACANCY
/// at validation rather than as a phantom node.
#[derive(Debug, Default)]
pub struct LatticeGraph {
    nodes: Vec<Node>,
    index: HashMap<String, usize>,
    edges: Vec<Edge>,
    issues: Vec<Issue>,
    pathways: BTreeMap<String, Pathway>,
}

impl LatticeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node, or report the repeat. First declaration wins.
    pub fn add_node(
        &mut self,
        id: impl Into<String>,
        kind: impl Into<String>,
        attrs: Map<String, Value>,
        provenance: Provenance,
    ) -> Result<(), DuplicateNode> {
        let id = id.into();
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
            kind: kind.into(),
            attrs,
            provenance,
        });
        Ok(())
    }

    pub fn add_edge(&mut self, edge: EdgeSpec, provenance: Provenance) {
        self.edges.push(Edge {
            src: edge.src,
            tgt: edge.tgt,
            kind: edge.kind,
            attrs: edge.attrs,
            provenance,
        });
    }

    pub fn add_issue(&mut self, issue: Issue) {
        self.issues.push(issue);
    }

    /// Attach an ordering pathway the adapter read from the target, or refuse
    /// it on `check_pathway`'s terms.
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

    pub fn pathway(&self, name: &str) -> Option<&Pathway> {
        self.pathways.get(name)
    }

    /// Yield the attached pathways, in name order.
    pub fn iter_pathways(&self) -> impl Iterator<Item = &Pathway> {
        self.pathways.values()
    }

    pub fn adapter_issues(&self) -> &[Issue] {
        &self.issues
    }

    pub fn has_node(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    pub fn node(&self, id: &str) -> Option<&Node> {
        self.index.get(id).map(|&at| &self.nodes[at])
    }

    /// Yield explicitly added nodes, in the order they were added.
    pub fn iter_nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }

    pub fn iter_edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.iter()
    }
}
