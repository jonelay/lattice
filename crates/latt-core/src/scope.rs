//! What a graph holds per declared kind: counts and the files they came from.
//!
//! One walk serves `EMPTY_KIND`, `validate`'s `scope` and `describe`, so the three
//! cannot disagree about what was read. Only kinds the profile declares are
//! tallied; a kind the register names and the profile does not is
//! `UNKNOWN_KIND`'s finding, and counting it here would make a misdeclared
//! register read as populated.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::LatticeGraph;
use crate::profile::Profile;
use crate::types::ValidateScope;

/// Per-kind tallies over one graph, keyed by kind name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KindTally<'g> {
    /// Declared node kind → (declared nodes, the distinct files declaring them).
    /// The files borrow the graph's own strings: a tally lives for one walk.
    pub(crate) nodes: BTreeMap<String, (usize, BTreeSet<&'g str>)>,
    /// Declared edge kind → edges of that kind.
    pub(crate) edges: BTreeMap<String, usize>,
    /// Every distinct file named by the provenance of a counted node or edge.
    pub(crate) files: BTreeSet<&'g str>,
}

impl<'g> KindTally<'g> {
    /// Walk `graph` once, tallying every kind `profile` declares.
    ///
    /// Only explicitly declared nodes count (`iter_nodes`): an edge endpoint no
    /// adapter declared is a `VACANCY`, and a node it does not add.
    pub(crate) fn of(graph: &'g LatticeGraph, profile: &Profile) -> Self {
        let mut tally = Self {
            nodes: profile
                .node_kinds()
                .keys()
                .map(|kind| (kind.clone(), (0, BTreeSet::new())))
                .collect(),
            edges: profile
                .edge_kinds()
                .keys()
                .map(|kind| (kind.clone(), 0))
                .collect(),
            files: BTreeSet::new(),
        };
        for node in graph.iter_nodes() {
            if let Some((count, files)) = tally.nodes.get_mut(&node.kind) {
                *count += 1;
                let file = node.provenance.file();
                files.insert(file);
                tally.files.insert(file);
            }
        }
        for edge in graph.iter_edges() {
            if let Some(count) = tally.edges.get_mut(&edge.kind) {
                *count += 1;
                tally.files.insert(edge.provenance.file());
            }
        }
        tally
    }

    /// Tally the declared kinds over bare instance kinds, for a population no
    /// one graph holds: fuse lists edges its composed graph leaves out, and
    /// declares kinds its composed profile does not.
    pub(crate) fn of_kinds<'a>(
        node_kinds: impl IntoIterator<Item = String>,
        edge_kinds: impl IntoIterator<Item = String>,
        nodes: impl IntoIterator<Item = &'a str>,
        edges: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut tally = Self {
            nodes: node_kinds
                .into_iter()
                .map(|kind| (kind, (0, BTreeSet::new())))
                .collect(),
            edges: edge_kinds.into_iter().map(|kind| (kind, 0)).collect(),
            files: BTreeSet::new(),
        };
        for kind in nodes {
            if let Some((count, _)) = tally.nodes.get_mut(kind) {
                *count += 1;
            }
        }
        for kind in edges {
            if let Some(count) = tally.edges.get_mut(kind) {
                *count += 1;
            }
        }
        tally
    }

    /// The counts alone, as `validate` reports them.
    pub(crate) fn scope(&self) -> ValidateScope {
        ValidateScope {
            nodes: self
                .nodes
                .iter()
                .map(|(kind, (count, _))| (kind.clone(), *count))
                .collect(),
            edges: self.edges.clone(),
            files: self.files.len(),
        }
    }

    /// Declared node kinds with no node, then edge kinds with no edge, by name.
    pub(crate) fn empty_kinds(&self) -> impl Iterator<Item = (&str, EmptyOf)> {
        let nodes = self
            .nodes
            .iter()
            .filter(|(_, (count, _))| *count == 0)
            .map(|(kind, _)| (kind.as_str(), EmptyOf::Node));
        let edges = self
            .edges
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(kind, _)| (kind.as_str(), EmptyOf::Edge));
        nodes.chain(edges)
    }
}

/// Which side of the profile an empty kind was declared on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EmptyOf {
    Node,
    Edge,
}

impl EmptyOf {
    /// The `EMPTY_KIND` message for `kind`.
    pub(crate) fn message(self, kind: &str) -> String {
        match self {
            Self::Node => format!("node kind '{kind}' has no node"),
            Self::Edge => format!("edge kind '{kind}' has no edge"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::prepare_profile;
    use crate::types::Provenance;
    use serde_json::Map;

    fn profile() -> Profile {
        prepare_profile(
            serde_norway::from_str(
                r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req: {id_pattern: "^REQ-\\d+$"}
  test: {id_pattern: "^T-\\d+$"}
edge_kinds:
  verifies: {allowed: [[test, req]]}
"#,
            )
            .unwrap(),
            "t",
        )
        .unwrap()
    }

    #[test]
    fn an_undeclared_kind_is_not_tallied_and_its_file_is_not_counted() {
        let mut graph = LatticeGraph::new();
        graph
            .add_node("REQ-1", "req", Map::new(), Provenance::new("r.md", 1))
            .unwrap();
        graph
            .add_node("X-1", "other", Map::new(), Provenance::new("x.md", 1))
            .unwrap();
        let tally = KindTally::of(&graph, &profile());
        assert_eq!(tally.nodes["req"].0, 1);
        assert!(!tally.nodes.contains_key("other"));
        assert_eq!(tally.files.len(), 1);
        let empty: Vec<_> = tally.empty_kinds().collect();
        assert_eq!(
            empty,
            [("test", EmptyOf::Node), ("verifies", EmptyOf::Edge)]
        );
    }

    #[test]
    fn a_file_declaring_nodes_and_edges_counts_once() {
        let mut graph = LatticeGraph::new();
        graph
            .add_node("REQ-1", "req", Map::new(), Provenance::new("r.md", 1))
            .unwrap();
        graph
            .add_node("T-1", "test", Map::new(), Provenance::new("r.md", 2))
            .unwrap();
        graph.add_edge(
            crate::graph::EdgeSpec {
                src: "T-1".into(),
                tgt: "REQ-1".into(),
                kind: "verifies".into(),
                attrs: Map::new(),
            },
            Provenance::new("r.md", 2),
        );
        let scope = KindTally::of(&graph, &profile()).scope();
        assert_eq!(scope.files, 1);
        assert_eq!(scope.nodes["req"], 1);
        assert_eq!(scope.edges["verifies"], 1);
    }
}
