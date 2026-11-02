//! The schema graph: the profile's kinds and the edges it allows between them.

use crate::graph::LatticeGraph;
use crate::profile::Profile;
use crate::types::{AllowedTriple, SchemaGraph, SchemaJoinCounts};

use super::AdjacencyIndex;

/// Extract the profile's type structure as a schema graph.
///
/// # Examples
///
/// ```
/// use latt_core::query::schema_graph;
/// # use latt_core::service::prepare_profile;
/// # let profile = prepare_profile(serde_norway::from_str(r#"
/// # name: demo
/// # profile_version: "1.0.0"
/// # node_kinds:
/// #   req:
/// #     id_pattern: "^REQ-\\d+$"
/// #   test:
/// #     id_pattern: "^T-\\d+$"
/// # edge_kinds:
/// #   verifies:
/// #     allowed: [[test, req]]
/// # "#)?, "demo")?;
///
/// let schema = schema_graph(&profile);
/// assert_eq!(schema.kinds, ["req", "test"]);
/// assert_eq!(schema.triples.len(), 1);
/// assert_eq!(schema.triples[0].edge_kind, "verifies");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
#[doc(hidden)]
pub fn schema_graph(profile: &Profile) -> SchemaGraph {
    let mut kind_pairs: Vec<(&str, usize)> = profile
        .node_kinds()
        .iter()
        .map(|(name, nk)| (name.as_str(), nk.declared_index))
        .collect();
    kind_pairs.sort_by_key(|&(_, idx)| idx);
    let kinds: Vec<String> = kind_pairs
        .iter()
        .map(|&(name, _)| name.to_string())
        .collect();

    let mut triples: Vec<AllowedTriple> = Vec::new();
    for (edge_kind, ek) in profile.edge_kinds() {
        for (src, tgt) in &ek.allowed {
            triples.push(AllowedTriple {
                edge_kind: edge_kind.clone(),
                src_kind: src.clone(),
                tgt_kind: tgt.clone(),
            });
        }
    }
    triples.sort();

    SchemaGraph { kinds, triples }
}

impl SchemaGraph {
    /// Join instance counts from a loaded graph onto this schema.
    ///
    /// # Examples
    ///
    /// ```
    /// use latt_core::query::{AdjacencyIndex, schema_graph};
    /// # use latt_core::service::prepare_profile;
    /// # let profile = prepare_profile(serde_norway::from_str(r#"
    /// # name: demo
    /// # profile_version: "1.0.0"
    /// # node_kinds:
    /// #   req:
    /// #     id_pattern: "^REQ-\\d+$"
    /// # edge_kinds: {}
    /// # "#)?, "demo")?;
    /// # use latt_core::graph::LatticeGraph;
    /// # use latt_core::types::Provenance;
    /// # let mut graph = LatticeGraph::new();
    /// # graph.add_node("REQ-1", "req", serde_json::Map::new(), Provenance::new("reqs.md", 3))?;
    /// let schema = schema_graph(&profile);
    /// let counts = schema.join_counts(&AdjacencyIndex::build(&graph), &graph);
    /// assert_eq!(counts.kind_counts, [("req".to_string(), 1)]);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn join_counts(
        &self,
        index: &AdjacencyIndex<'_>,
        graph: &LatticeGraph,
    ) -> SchemaJoinCounts {
        let kind_counts: Vec<(String, usize)> = self
            .kinds
            .iter()
            .map(|kind| {
                let count = graph.iter_nodes().filter(|n| n.kind == *kind).count();
                (kind.clone(), count)
            })
            .collect();

        let triple_counts: Vec<(AllowedTriple, usize)> = self
            .triples
            .iter()
            .map(|triple| {
                let count = graph
                    .iter_nodes()
                    .filter(|n| n.kind == triple.src_kind)
                    .map(|n| {
                        index
                            .edges_from(&n.id)
                            .iter()
                            .filter(|e| {
                                e.kind == triple.edge_kind
                                    && graph
                                        .node(&e.tgt)
                                        .is_some_and(|tgt| tgt.kind == triple.tgt_kind)
                            })
                            .count()
                    })
                    .sum();
                (triple.clone(), count)
            })
            .collect();

        SchemaJoinCounts {
            kind_counts,
            triple_counts,
        }
    }
}

#[cfg(test)]
mod schema_tests {
    use super::{AdjacencyIndex, schema_graph};
    use crate::graph::{EdgeSpec, LatticeGraph};
    use crate::profile::load_profile_value;
    use crate::types::Provenance;
    use serde_json::Map;

    fn prov() -> Provenance {
        Provenance::new("test.md", 1)
    }

    fn test_profile() -> crate::profile::Profile {
        let yaml = r#"
name: test
profile_version: "1.0.0"
node_kinds:
  requirement:
    id_pattern: "REQ-.*"
  test:
    id_pattern: "T-.*"
  risk:
    id_pattern: "RISK-.*"
edge_kinds:
  verifies:
    allowed:
      - [test, requirement]
  mitigates:
    allowed:
      - [requirement, risk]
"#;
        let value: serde_norway::Value = serde_norway::from_str(yaml).unwrap();
        load_profile_value(value, "test").unwrap()
    }

    #[test]
    fn schema_graph_extracts_kinds_in_profile_order() {
        let profile = test_profile();
        let sg = schema_graph(&profile);
        assert_eq!(sg.kinds, vec!["requirement", "test", "risk"]);
    }

    #[test]
    fn schema_graph_extracts_allowed_triples_sorted() {
        let profile = test_profile();
        let sg = schema_graph(&profile);
        assert_eq!(sg.triples.len(), 2);
        assert_eq!(sg.triples[0].edge_kind, "mitigates");
        assert_eq!(sg.triples[0].src_kind, "requirement");
        assert_eq!(sg.triples[0].tgt_kind, "risk");
        assert_eq!(sg.triples[1].edge_kind, "verifies");
        assert_eq!(sg.triples[1].src_kind, "test");
        assert_eq!(sg.triples[1].tgt_kind, "requirement");
    }

    #[test]
    fn schema_join_counts_populated_graph() {
        let profile = test_profile();
        let sg = schema_graph(&profile);
        let mut g = LatticeGraph::new();
        for i in 0..5 {
            g.add_node(format!("REQ-{i}"), "requirement", Map::new(), prov())
                .unwrap();
        }
        for i in 0..8 {
            g.add_node(format!("T-{i}"), "test", Map::new(), prov())
                .unwrap();
        }
        for i in 0..8 {
            g.add_edge(
                EdgeSpec {
                    src: format!("T-{i}"),
                    tgt: format!("REQ-{}", i % 5),
                    kind: "verifies".into(),
                    attrs: Map::new(),
                },
                prov(),
            );
        }
        let idx = AdjacencyIndex::build(&g);
        let counts = sg.join_counts(&idx, &g);
        assert_eq!(counts.kind_counts[0], ("requirement".into(), 5));
        assert_eq!(counts.kind_counts[1], ("test".into(), 8));
        assert_eq!(counts.kind_counts[2], ("risk".into(), 0));
        let verifies_count = counts
            .triple_counts
            .iter()
            .find(|(t, _)| t.edge_kind == "verifies")
            .unwrap();
        assert_eq!(verifies_count.1, 8);
    }

    #[test]
    fn schema_join_counts_zero_triples_reported() {
        let profile = test_profile();
        let sg = schema_graph(&profile);
        let g = LatticeGraph::new();
        let idx = AdjacencyIndex::build(&g);
        let counts = sg.join_counts(&idx, &g);
        assert_eq!(counts.kind_counts.len(), 3);
        for (_, count) in &counts.kind_counts {
            assert_eq!(*count, 0);
        }
        assert_eq!(counts.triple_counts.len(), 2);
        for (_, count) in &counts.triple_counts {
            assert_eq!(*count, 0);
        }
    }
}
