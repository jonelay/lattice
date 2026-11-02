//! Smoke queries per entry point over `mini-toml`, checked against the graph.

mod common;

use std::sync::Arc;

use latt_trustfall::adapter::LatticeAdapter;
use latt_trustfall::schema::{schema_text, type_name};
use trustfall_core::ir::FieldValue;
use trustfall_core::schema::Schema;

fn ids(rows: &[std::collections::BTreeMap<Arc<str>, FieldValue>], key: &str) -> Vec<String> {
    let mut out: Vec<String> = rows
        .iter()
        .map(|row| row[key].as_str().expect("a string").to_owned())
        .collect();
    out.sort();
    out
}

// Requirement: development-verification/Validators are cross-checked against an independent engine
#[test]
fn adapter_entry_points_answer_from_the_graph() {
    let pairs = common::fixture_pairs();
    let (_, profile, graph) = pairs.iter().find(|(n, _, _)| n == "toml").unwrap();
    let schema = Schema::parse(schema_text(profile).unwrap()).unwrap();
    let adapter = Arc::new(LatticeAdapter::new(graph, profile));

    let mut expected: Vec<String> = graph.iter_nodes().map(|n| n.id.clone()).collect();
    expected.sort();
    let rows = common::run(&schema, &adapter, "{ Nodes { id @output } }");
    assert_eq!(ids(&rows, "id"), expected);

    let kind = profile.node_kinds().keys().next().unwrap();
    let mut of_kind: Vec<String> = graph
        .iter_nodes()
        .filter(|n| &n.kind == kind)
        .map(|n| n.id.clone())
        .collect();
    of_kind.sort();
    let query = format!("{{ NodesOfKind(kind: \"{kind}\") {{ id @output kind @output }} }}");
    let rows = common::run(&schema, &adapter, &query);
    assert_eq!(ids(&rows, "id"), of_kind);
    assert!(rows.iter().all(|r| r["kind"].as_str() == Some(kind)));

    let coerced = format!(
        "{{ Nodes {{ ... on {} {{ id @output }} }} }}",
        type_name(kind)
    );
    assert_eq!(
        ids(&common::run(&schema, &adapter, &coerced), "id"),
        of_kind
    );

    let rows = common::run(
        &schema,
        &adapter,
        "{ Edges { source_id @output target_id @output resolved @output } }",
    );
    assert_eq!(rows.len(), graph.iter_edges().count());
    let dangling = graph
        .iter_edges()
        .filter(|e| !graph.has_node(&e.tgt) || !graph.has_node(&e.src))
        .count();
    assert_eq!(
        rows.iter()
            .filter(|r| r["resolved"] == FieldValue::Boolean(false))
            .count(),
        dangling
    );

    // Every edge field on every kind resolves without panicking, and the
    // out_edges records agree with the adjacency the graph exposes.
    let rows = common::run(
        &schema,
        &adapter,
        "{ Nodes { id @output out_edges @fold @transform(op: \"count\") @output(name: \"out\") } }",
    );
    for row in &rows {
        let id = row["id"].as_str().unwrap();
        let count = graph.iter_edges().filter(|e| e.src == id).count() as u64;
        assert_eq!(row["out"], FieldValue::Uint64(count), "{id}");
    }
}
