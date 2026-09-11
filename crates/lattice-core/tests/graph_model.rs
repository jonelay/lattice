//! The `graph-model` capability's scenarios, against the Rust core.
//!
//! Named for the spec, not for the module: each test below is one `#### Scenario`
//! in `openspec/specs/graph-model/spec.md`, so a spec change has an obvious
//! landing site here.
//!
//! Five of that capability's scenarios are absent, for two different reasons.
//!
//! Three are settled by the language rather than by this code: "Provenance is
//! required" asserts a `TypeError` from a missing Python argument, and "Mutating
//! a read does not affect the graph" (once for nodes, once for pathway order)
//! asserts that a caller cannot write through what it reads. Provenance is a
//! non-defaulted parameter here, and the readers hand out shared references that
//! cannot be mutated at all, so a test would assert a property of Rust.
//!
//! The other two, "Orphan finding includes provenance" and "Config finding is not
//! attributed to a source line", are about what validation emits rather than what
//! the graph stores. They are covered in `pinned_behaviours.rs`, which is where the
//! validator's untested paths live.

use lattice_core::document::ingest_document;
use lattice_core::graph::{EdgeSpec, LatticeGraph};
use lattice_core::types::{Issue, Provenance, Severity};
use serde_json::{Map, Value, json};

fn attrs(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
        .collect()
}

fn prov(file: &str, line: i64) -> Provenance {
    Provenance::new(file, line)
}

// Requirement: Add node with provenance

#[test]
fn add_a_valid_node() {
    let mut graph = LatticeGraph::new();
    graph
        .add_node(
            "REQ-0701",
            "req",
            attrs(&[("text", "...")]),
            prov("REQUIREMENTS.md", 47),
        )
        .unwrap();

    let node = graph.node("REQ-0701").expect("node was added");
    assert_eq!(node.kind, "req");
    assert_eq!(node.attrs["text"], json!("..."));
    assert_eq!(node.provenance, prov("REQUIREMENTS.md", 47));
}

// Requirement: Add edge with attrs and provenance

#[test]
fn add_a_valid_edge() {
    let mut graph = LatticeGraph::new();
    graph.add_edge(
        EdgeSpec {
            src: "test_fem".into(),
            tgt: "REQ-0704".into(),
            kind: "verifies".into(),
            attrs: attrs(&[("confidence", "high")]),
        },
        prov("tests/test_fem.py", 9),
    );

    let edges: Vec<_> = graph.iter_edges().collect();
    assert_eq!(edges.len(), 1);
    assert_eq!(
        (edges[0].src.as_str(), edges[0].tgt.as_str()),
        ("test_fem", "REQ-0704")
    );
    assert_eq!(edges[0].kind, "verifies");
    assert_eq!(edges[0].attrs["confidence"], "high");
}

// Requirement: Duplicate node semantics

#[test]
fn add_node_reports_a_repeated_id() {
    let mut graph = LatticeGraph::new();
    graph
        .add_node("REQ-0701", "req", Map::new(), prov("a.md", 1))
        .unwrap();

    let duplicate = graph
        .add_node("REQ-0701", "req", Map::new(), prov("b.md", 2))
        .expect_err("the repeat must be reported, not accepted");

    assert_eq!(duplicate.node_id, "REQ-0701");
    assert_eq!(duplicate.existing_provenance, prov("a.md", 1));
    assert_eq!(duplicate.new_provenance, prov("b.md", 2));
}

#[test]
fn duplicate_node_id_same_kind() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "REQ-0701", "kind": "req", "attrs": {},
             "provenance": {"file": "a.md", "line": 1}},
            {"id": "REQ-0701", "kind": "req", "attrs": {},
             "provenance": {"file": "b.md", "line": 2}},
        ],
    }));

    assert_eq!(graph.iter_nodes().count(), 1);
    let issues = graph.adapter_issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "PARSE_ERROR");
    assert_eq!(issues[0].severity, Severity::Error);
    assert_eq!(issues[0].node_id.as_deref(), Some("REQ-0701"));
    // Both provenances: the first inside the message, the second as the finding's own.
    assert!(
        issues[0].message.contains("a.md:1"),
        "message was {:?}",
        issues[0].message
    );
    assert_eq!(issues[0].provenance, prov("b.md", 2));
}

#[test]
fn duplicate_node_id_different_kind() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "X-1", "kind": "req", "attrs": {},
             "provenance": {"file": "a.md", "line": 1}},
            {"id": "X-1", "kind": "test", "attrs": {},
             "provenance": {"file": "b.md", "line": 2}},
        ],
    }));

    assert_eq!(graph.node("X-1").unwrap().kind, "req");
    let issues = graph.adapter_issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].node_id.as_deref(), Some("X-1"));
    assert!(issues[0].message.contains("a.md:1"));
    assert_eq!(issues[0].provenance, prov("b.md", 2));
}

#[test]
fn the_surviving_node_is_the_first_declared() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "REQ-0701", "kind": "req", "attrs": {"text": "A"},
             "provenance": {"file": "a.md", "line": 1}},
            {"id": "REQ-0701", "kind": "req", "attrs": {"text": "B"},
             "provenance": {"file": "b.md", "line": 2}},
        ],
    }));

    let node = graph.node("REQ-0701").unwrap();
    assert_eq!(node.attrs["text"], json!("A"));
    assert_eq!(node.provenance, prov("a.md", 1));
}

// Requirement: Dangling edge targets

#[test]
fn edge_added_before_target_node() {
    let mut graph = LatticeGraph::new();
    graph.add_edge(
        EdgeSpec {
            src: "test_x".into(),
            tgt: "REQ-0701".into(),
            kind: "verifies".into(),
            attrs: Map::new(),
        },
        prov("t.py", 1),
    );

    assert_eq!(graph.iter_edges().count(), 1);
    // The edge did not conjure its endpoints: validation, not the builder,
    // reports the unresolved reference.
    assert!(!graph.has_node("REQ-0701"));
    assert!(!graph.has_node("test_x"));
    assert_eq!(graph.iter_nodes().count(), 0);
}

// Requirement: Parallel edges are distinct

#[test]
fn repeated_edge_keeps_both_provenances() {
    let mut graph = LatticeGraph::new();
    for line in [1, 9] {
        graph.add_edge(
            EdgeSpec {
                src: "A".into(),
                tgt: "B".into(),
                kind: "derives".into(),
                attrs: Map::new(),
            },
            prov("a.md", line),
        );
    }

    let provs: Vec<_> = graph.iter_edges().map(|e| e.provenance.clone()).collect();
    assert_eq!(provs, vec![prov("a.md", 1), prov("a.md", 9)]);
}

// Requirement: Pathway storage on the graph

#[test]
fn pathway_set_and_read() {
    let mut graph = LatticeGraph::new();
    graph
        .set_pathway("phase", order(&["R0", "CB", "M0", "M1"]), "M0")
        .unwrap();

    let pathway = graph.pathway("phase").expect("pathway was set");
    assert_eq!(pathway.order, order(&["R0", "CB", "M0", "M1"]));
    assert_eq!(pathway.current, "M0");
    assert!(pathway.is_member("CB"));
    assert!(pathway.is_after("M1"));
    assert!(!pathway.is_after("R0"));
    // A non-member is neither member nor after; the two answers are distinct.
    assert!(!pathway.is_member("ZZ"));
    assert!(!pathway.is_after("ZZ"));
}

#[test]
fn no_pathway_set() {
    let graph = LatticeGraph::new();
    assert!(graph.pathway("phase").is_none());
}

#[test]
fn current_position_must_be_a_member() {
    let mut graph = LatticeGraph::new();
    assert!(
        graph
            .set_pathway("phase", order(&["M0", "M1"]), "M9")
            .is_err()
    );
}

#[test]
fn positions_must_be_unique() {
    let mut graph = LatticeGraph::new();
    assert!(
        graph
            .set_pathway("phase", order(&["M0", "M1", "M0"]), "M0")
            .is_err()
    );
}

// Requirement: Document order is significant

// Ingest order, which is what "first occurrence wins" is defined against.

#[test]
fn nodes_and_edges_keep_document_order() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "C", "kind": "req", "attrs": {}, "provenance": {"file": "a.md", "line": 3}},
            {"id": "A", "kind": "req", "attrs": {}, "provenance": {"file": "a.md", "line": 1}},
            {"id": "B", "kind": "req", "attrs": {}, "provenance": {"file": "a.md", "line": 2}},
        ],
        "edges": [
            {"src": "B", "tgt": "C", "kind": "derives", "provenance": {"file": "a.md", "line": 2}},
            {"src": "A", "tgt": "B", "kind": "derives", "provenance": {"file": "a.md", "line": 1}},
        ],
    }));

    let ids: Vec<_> = graph.iter_nodes().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, vec!["C", "A", "B"]);
    let pairs: Vec<_> = graph
        .iter_edges()
        .map(|e| (e.src.as_str(), e.tgt.as_str()))
        .collect();
    assert_eq!(pairs, vec![("B", "C"), ("A", "B")]);
}

// Requirement: Dangling edge targets

#[test]
fn an_edge_endpoint_is_not_a_node() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "A", "kind": "req", "attrs": {}, "provenance": {"file": "a.md", "line": 1}},
        ],
        "edges": [
            {"src": "A", "tgt": "GHOST", "kind": "derives",
             "provenance": {"file": "a.md", "line": 1}},
        ],
    }));

    let ids: Vec<_> = graph.iter_nodes().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, vec!["A"]);
    assert!(!graph.has_node("GHOST"));
}

fn order(positions: &[&str]) -> Vec<String> {
    positions.iter().map(|p| (*p).to_string()).collect()
}

fn ingest(document: Value) -> LatticeGraph {
    ingest_document(document).expect("document is well-formed")
}

// Requirement: Parallel edges are distinct

#[test]
fn one_pair_may_carry_several_edge_kinds() {
    let mut graph = LatticeGraph::new();
    graph
        .add_node("REQ-1", "req", Map::new(), Provenance::new("r.md", 1))
        .unwrap();
    graph
        .add_node("REQ-2", "req", Map::new(), Provenance::new("r.md", 2))
        .unwrap();
    for (kind, line) in [("derives", 3), ("verifies", 4)] {
        graph.add_edge(
            EdgeSpec {
                src: "REQ-1".into(),
                tgt: "REQ-2".into(),
                kind: kind.into(),
                attrs: Map::new(),
            },
            Provenance::new("r.md", line),
        );
    }

    let kinds: Vec<&str> = graph.iter_edges().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["derives", "verifies"], "neither replaces the other");
}

// Requirement: Adapter issue channel

#[test]
fn adapter_issues_are_collected_in_the_order_they_arrive() {
    let mut graph = LatticeGraph::new();
    for code in ["PARSE_ERROR", "SOURCE_MISSING"] {
        graph.add_issue(Issue::new(
            Severity::Warning,
            code,
            "m",
            Provenance::new("r.md", 1),
            None,
        ));
    }
    let codes: Vec<&str> = graph
        .adapter_issues()
        .iter()
        .map(|i| i.code.as_str())
        .collect();
    assert_eq!(codes, ["PARSE_ERROR", "SOURCE_MISSING"]);
}
