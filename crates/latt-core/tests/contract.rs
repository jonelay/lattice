//! The `adapter-contract` capability's scenarios, against ingest.
//!
//! The failure modes that need a real subprocess live in `cli.rs`, where the
//! exit code they produce is observable. Everything here is about the document
//! itself: what a well-formed one carries through, and what shape is refused
//! rather than half-read.

mod common;

use common::ingest;
use latt_core::document::{INTERFACE_VERSION, ingest_document, parse_document};
use latt_core::types::Severity;
use serde_json::{Value, json};

/// The message of an ingest that was expected to be refused.
fn refused(document: Value) -> String {
    let rendered = document.to_string();
    match ingest_document(document) {
        Ok(_) => panic!("document ingested but should have been refused:\n{rendered}"),
        Err(e) => e.message().to_owned(),
    }
}

fn node(id: &str, kind: &str, line: i64) -> Value {
    json!({"id": id, "kind": kind, "attrs": {},
           "provenance": {"file": "r.md", "line": line}})
}

// Requirement: adapter-contract/Interface document declares its version

#[test]
fn the_shipped_version_ingests() {
    let graph = ingest(json!({"interface_version": INTERFACE_VERSION, "nodes": []}));
    assert_eq!(graph.iter_nodes().count(), 0);
}

#[test]
fn legacy_version_ingests_when_interface_version_is_absent() {
    let graph = ingest(json!({"contract_version": "1.2", "nodes": []}));
    assert_eq!(graph.iter_nodes().count(), 0);
}

#[test]
fn interface_version_takes_precedence_over_legacy_version() {
    let error = refused(json!({
        "interface_version": "2.0",
        "contract_version": "1.2",
        "nodes": [],
    }));
    assert!(
        error.contains("unsupported interface version '2.0'"),
        "{error}"
    );
}

#[test]
fn interface_1_2_accepts_findings() {
    let graph = ingest(json!({
        "interface_version": "1.2",
        "findings": [{"severity": "hint", "code": "SUGGESTION", "message": "m",
                      "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(graph.adapter_issues()[0].severity, Severity::Hint);
}

#[test]
fn an_unsupported_version_is_refused_naming_what_is_supported() {
    let error = refused(json!({"interface_version": "2.0", "nodes": []}));
    assert!(
        error.contains("unsupported interface version '2.0'"),
        "{error}"
    );
    assert!(error.contains(INTERFACE_VERSION), "{error}");
}

#[test]
fn a_document_with_no_version_is_refused() {
    assert!(refused(json!({"nodes": []})).contains("missing 'interface_version'"));
}

// Requirement: adapter-contract/Document schema

#[test]
fn nodes_and_edges_ingest_with_their_provenance() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 3)],
        "edges": [{"src": "REQ-1", "tgt": "REQ-2", "kind": "derives",
                   "provenance": {"file": "r.md", "line": 7}}],
    }));
    let stored = graph.node("REQ-1").expect("the node was added");
    assert_eq!(stored.kind, "req");
    assert_eq!(stored.provenance.line().map(u32::from), Some(3));
    let edge = graph.iter_edges().next().expect("the edge was added");
    assert_eq!(edge.provenance.line().map(u32::from), Some(7));
}

#[test]
fn edge_attrs_survive_ingest_and_default_to_empty() {
    let graph = ingest(json!({
        "interface_version": "1.2",
        "edges": [
            {"src": "REQ-1", "tgt": "REQ-2", "kind": "derives",
             "attrs": {"confidence": 0.75, "basis": ["text", "label"]},
             "provenance": {"file": "r.md", "line": 7}},
            {"src": "REQ-2", "tgt": "REQ-3", "kind": "derives",
             "provenance": {"file": "r.md", "line": 8}},
        ],
    }));
    let edges: Vec<_> = graph.iter_edges().collect();
    assert_eq!(edges[0].attrs["confidence"], 0.75);
    assert_eq!(edges[0].attrs["basis"][1], "label");
    assert!(edges[1].attrs.is_empty());
}

#[test]
fn edge_attrs_that_are_not_an_object_are_refused() {
    let document = json!({
        "interface_version": "1.2",
        "edges": [{"src": "REQ-1", "tgt": "REQ-2", "kind": "derives",
                   "attrs": ["confidence"],
                   "provenance": {"file": "r.md", "line": 7}}],
    });
    assert!(refused(document).contains("'attrs' must be an object, got list"));
}

#[test]
fn nested_attrs_survive_ingest_unflattened() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req",
                   "attrs": {"meta": {"owner": "jl", "tags": ["a", "b"]}},
                   "provenance": {"file": "r.md", "line": 1}}],
    }));
    let attrs = &graph.node("REQ-1").unwrap().attrs;
    assert_eq!(attrs["meta"]["owner"], "jl");
    assert_eq!(attrs["meta"]["tags"][1], "b");
}

#[test]
fn a_node_without_provenance_is_refused() {
    let document = json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {}}],
    });
    assert!(refused(document).contains("missing 'provenance'"));
}

#[test]
fn a_provenance_line_that_is_not_an_integer_is_refused() {
    // A boolean is an int in Python, and a boolean line number is a schema
    // failure rather than a line 1 the core should invent.
    for line in [json!(true), json!("3"), json!(1.5)] {
        let document = json!({
            "interface_version": "1.0",
            "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                       "provenance": {"file": "r.md", "line": line}}],
        });
        assert!(
            refused(document).contains("'line' must be an integer"),
            "line {line} must be refused"
        );
    }
}

// Requirement: adapter-contract/Document schema
#[test]
fn a_negative_provenance_line_is_refused() {
    // A line below zero names no place in the file, and would sort ahead of
    // every real line.
    let document = json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                   "provenance": {"file": "r.md", "line": -3}}],
    });
    assert!(refused(document).contains("'line' must not be negative, got -3"));
}

#[test]
fn a_top_level_value_that_is_not_an_object_is_refused() {
    assert!(refused(json!([])).contains("expected an object"));
}

#[test]
fn attrs_that_are_not_an_object_are_refused() {
    let document = json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": ["text"],
                   "provenance": {"file": "r.md", "line": 1}}],
    });
    assert!(refused(document).contains("'attrs' must be an object"));
}

// Requirement: validation/Adapter issue channel

#[test]
fn an_adapter_issue_keeps_its_severity_for_a_code_the_core_does_not_implement() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "issues": [{"severity": "info", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(graph.adapter_issues()[0].severity, Severity::Info);
}

#[test]
fn an_adapter_issues_node_id_travels_through_ingest() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": "REQ-1"}],
    }));
    assert_eq!(graph.adapter_issues()[0].node_id.as_deref(), Some("REQ-1"));
}

// Requirement: adapter-contract/Document schema

#[test]
fn an_unknown_severity_is_refused_rather_than_defaulted() {
    let document = json!({
        "interface_version": "1.0",
        "issues": [{"severity": "critical", "code": "X", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    });
    assert!(refused(document).contains("unknown severity 'critical'"));
}

#[test]
fn a_node_id_that_is_not_a_string_is_refused() {
    let document = json!({
        "interface_version": "1.0",
        "issues": [{"severity": "warning", "code": "X", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": 3}],
    });
    assert!(refused(document).contains("'node_id' must be a string or null"));
}

// Requirement: adapter-contract/Duplicate node IDs are resolved at ingest

#[test]
fn a_third_occurrence_is_reported_as_well_as_the_second() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 1), node("REQ-1", "req", 2), node("REQ-1", "req", 3)],
    }));
    let duplicates: Vec<_> = graph
        .adapter_issues()
        .iter()
        .filter(|i| i.code == "PARSE_ERROR")
        .collect();
    assert_eq!(
        duplicates.len(),
        2,
        "every repeat is a finding, not just the first"
    );
    // Each names where the survivor was declared, not where the previous repeat was.
    assert!(
        duplicates
            .iter()
            .all(|i| i.message.contains("first at r.md:1"))
    );
    assert_eq!(
        graph
            .node("REQ-1")
            .unwrap()
            .provenance
            .line()
            .map(u32::from),
        Some(1)
    );
}

#[test]
fn the_adapters_own_issues_read_before_the_duplicate_findings() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 1), node("REQ-1", "req", 2)],
        "issues": [{"severity": "warning", "code": "SOURCE_MISSING", "message": "m",
                    "provenance": {"file": "r.md", "line": 9}, "node_id": null}],
    }));
    let codes: Vec<&str> = graph
        .adapter_issues()
        .iter()
        .map(|i| i.code.as_str())
        .collect();
    assert_eq!(codes, ["SOURCE_MISSING", "PARSE_ERROR"]);
}

fn declared_edge(src: &str, tgt: &str, declared_by: Value) -> Value {
    json!({"src": src, "tgt": tgt, "kind": "uses",
           "provenance": {"file": "r.md", "line": 0}, "declared_by": declared_by})
}

fn targets(graph: &latt_core::graph::LatticeGraph) -> Vec<&str> {
    graph.iter_edges().map(|e| e.tgt.as_str()).collect()
}

fn duplicate_messages(graph: &latt_core::graph::LatticeGraph) -> Vec<&str> {
    graph
        .adapter_issues()
        .iter()
        .filter(|i| i.message.starts_with("duplicate node"))
        .map(|i| i.message.as_str())
        .collect()
}

// Requirement: adapter-contract/Document order is significant

#[test]
fn a_duplicate_entrys_declared_edges_are_not_read() {
    let graph = ingest(json!({
        "interface_version": "1.3",
        "nodes": [node("A", "req", 1), node("A", "req", 2)],
        "edges": [declared_edge("A", "X", json!(0)), declared_edge("A", "Y", json!(1)),
                  declared_edge("A", "Z", json!(1))],
    }));
    assert_eq!(targets(&graph), ["X"], "the first entry's edge is kept");
    assert_eq!(
        duplicate_messages(&graph),
        ["duplicate node 'A': first at r.md:1; its 2 edges were not read"]
    );
}

#[test]
fn a_duplicate_entrys_single_dropped_edge_is_named_in_the_singular() {
    let graph = ingest(json!({
        "interface_version": "1.3",
        "nodes": [node("A", "req", 1), node("A", "req", 2)],
        "edges": [declared_edge("A", "Y", json!(1))],
    }));
    assert!(targets(&graph).is_empty());
    assert_eq!(
        duplicate_messages(&graph),
        ["duplicate node 'A': first at r.md:1; its 1 edge was not read"]
    );
}

#[test]
fn declared_by_names_the_declaring_entry_not_the_source() {
    // The repeated entry B declares an edge whose source is C; it goes unread.
    let graph = ingest(json!({
        "interface_version": "1.3",
        "nodes": [node("C", "cap", 1), node("B", "req", 2), node("B", "req", 3)],
        "edges": [declared_edge("C", "B", json!(1)), declared_edge("C", "B", json!(2))],
    }));
    assert_eq!(graph.iter_edges().count(), 1);
}

fn undeclared_edge(src: &str, tgt: &str) -> Value {
    json!({"src": src, "tgt": tgt, "kind": "uses", "provenance": {"file": "r.md", "line": 2}})
}

#[test]
fn an_edge_without_declared_by_is_read_even_from_a_duplicated_id() {
    let graph = ingest(json!({
        "interface_version": "1.3",
        "nodes": [node("A", "req", 1), node("A", "req", 2)],
        "edges": [undeclared_edge("A", "Y")],
    }));
    assert_eq!(targets(&graph), ["Y"]);
    assert_eq!(
        duplicate_messages(&graph),
        [
            "duplicate node 'A': first at r.md:1; 1 edge from 'A' has no declared_by \
          and was read, though this entry may have declared it"
        ],
        "the core cannot attribute the edge, and says so rather than claim nothing was dropped"
    );
}

#[test]
fn unattributed_edges_are_counted_beside_dropped_ones_and_before_declared_by_existed() {
    let graph = ingest(json!({
        "interface_version": "1.3",
        "nodes": [node("A", "req", 1), node("A", "req", 2), node("B", "req", 3)],
        "edges": [declared_edge("A", "X", json!(1)), declared_edge("A", "W", json!(0)),
                  undeclared_edge("A", "Y"), undeclared_edge("A", "Z"),
                  undeclared_edge("B", "V")],
    }));
    assert_eq!(targets(&graph), ["W", "Y", "Z", "V"]);
    assert_eq!(
        duplicate_messages(&graph),
        [
            "duplicate node 'A': first at r.md:1; its 1 edge was not read; 2 edges from 'A' \
          have no declared_by and were read, though this entry may have declared them"
        ],
        "an edge attributed to the first entry, or from another ID, is not counted"
    );

    // A document older than declared_by cannot attribute any edge.
    let graph = ingest(json!({
        "interface_version": "1.2",
        "nodes": [node("A", "req", 1), node("A", "req", 2)],
        "edges": [undeclared_edge("A", "Y")],
    }));
    assert_eq!(targets(&graph), ["Y"]);
    assert_eq!(duplicate_messages(&graph).len(), 1);
    assert!(duplicate_messages(&graph)[0].ends_with("though this entry may have declared it"));
}

// Requirement: adapter-contract/Document schema

#[test]
fn declared_by_out_of_range_is_refused() {
    let error = refused(json!({
        "interface_version": "1.3",
        "nodes": [node("A", "req", 1)],
        "edges": [declared_edge("A", "X", json!(1))],
    }));
    assert!(
        error.contains("edge 0: 'declared_by' 1 names no node entry"),
        "{error}"
    );
}

#[test]
fn declared_by_that_is_not_a_non_negative_integer_is_refused() {
    for value in [json!(-1), json!("0"), json!(0.5), json!(true), json!(null)] {
        let error = refused(json!({
            "interface_version": "1.3",
            "nodes": [node("A", "req", 1)],
            "edges": [declared_edge("A", "X", value.clone())],
        }));
        assert!(
            error.contains("'declared_by' must be a non-negative integer"),
            "{value}: {error}"
        );
    }
}

// Requirement: adapter-contract/Interface document declares its version

#[test]
fn interface_1_3_ingests() {
    let graph = ingest(json!({
        "interface_version": "1.3",
        "nodes": [node("A", "req", 1)],
        "edges": [declared_edge("A", "X", json!(0))],
    }));
    assert_eq!(targets(&graph), ["X"]);
}

// Requirement: graph-model/Pathway storage on the graph

#[test]
fn a_pathway_ingests_and_answers_membership_and_position() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
    }));
    let pathway = graph.pathway("phase").expect("the pathway was attached");
    assert!(pathway.is_member("M0"));
    assert!(!pathway.is_member("M9"));
    assert!(pathway.is_after("M4"));
    assert!(!pathway.is_after("CB"));
    assert!(
        !pathway.is_after("M0"),
        "the current position is not after itself"
    );
}

// Requirement: adapter-contract/Pathway validity is the adapter's responsibility

#[test]
fn a_pathway_whose_current_is_off_its_order_is_refused() {
    // It would place every bound finding both before and after itself. The
    // adapter read the declaration and can name the file, so this is its bug.
    let document = json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0"], "current": "M9"}],
    });
    assert!(refused(document).contains("current position 'M9' is not in"));
}

#[test]
fn a_pathway_with_repeated_positions_is_refused() {
    let document = json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["M0", "M0"], "current": "M0"}],
    });
    assert!(refused(document).contains("positions are not unique"));
}

#[test]
fn a_pathway_order_that_is_not_a_list_of_strings_is_refused() {
    let document = json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["M0", 4], "current": "M0"}],
    });
    assert!(refused(document).contains("'order' must be a list of strings"));
}

// Requirement: adapter-contract/Adapter failure is a broken setup, not a finding

#[test]
fn parse_reads_a_document_and_refuses_silence() {
    assert!(parse_document(r#"{"interface_version": "1.0"}"#).is_ok());
    for empty in ["", "   \n\t "] {
        let error = parse_document(empty).expect_err("silence is not an empty register");
        assert_eq!(error.message(), "adapter emitted no output");
    }
    let error = parse_document("{oops").expect_err("malformed JSON is refused");
    assert!(
        error
            .message()
            .starts_with("could not parse adapter output as JSON")
    );
}

// Requirement: adapter-contract/Adapter is a program emitting a serialized graph

#[test]
fn a_document_carrying_only_issues_is_not_a_failure() {
    // An adapter that parsed nothing but has something to say about why is
    // reporting, not failing. Refusing here would turn a report into exit 2.
    let graph = ingest(json!({
        "interface_version": "1.0",
        "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(graph.iter_nodes().count(), 0);
    assert_eq!(graph.adapter_issues().len(), 1);
}

// Requirement: adapter-contract/Resolved profile document

/// The profile file parsed independently of the loader, as plain JSON data.
fn profile_as_json(path: &str) -> Value {
    let text = std::fs::read_to_string(path).unwrap();
    serde_norway::from_str(&text).unwrap()
}

#[test]
fn resolved_document_round_trips_every_declared_field() {
    for path in ["../../profiles/openspec.yaml", "../../profiles/toml.yaml"] {
        let profile = latt_core::profile::load_profile(std::path::Path::new(path)).unwrap();
        let text = latt_core::profile::resolved_document(&profile).unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["resolved_schema"], "1", "{path}");
        doc.as_object_mut().unwrap().remove("resolved_schema");
        assert_eq!(doc, profile_as_json(path), "{path}");
    }
}

#[test]
fn resolved_document_keeps_raw_pattern_sources_and_adapter_namespace() {
    let profile =
        latt_core::profile::load_profile(std::path::Path::new("../../profiles/openspec.yaml"))
            .unwrap();
    let text = latt_core::profile::resolved_document(&profile).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        doc["node_kinds"]["requirement"]["id_pattern"],
        "^[a-z0-9-]+/\\S(.*\\S)?$"
    );
    assert_eq!(doc["node_kinds"]["requirement"]["summary_attr"], "title");
    assert_eq!(doc["adapter"]["paths"]["spec_dir"], "openspec/specs");
}

// Requirement: adapter-contract/Interface versions remain ingest-compatible

#[test]
fn a_hint_severity_issue_survives_ingest_at_1_1() {
    let graph = ingest(json!({
        "interface_version": "1.1",
        "issues": [{"severity": "hint", "code": "SUGGESTION", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(graph.adapter_issues()[0].severity, Severity::Hint);
}

#[test]
fn older_documents_still_ingest_beside_1_2() {
    ingest(json!({"interface_version": "1.0", "nodes": []}));
    ingest(json!({"interface_version": "1.1", "nodes": []}));
    ingest(json!({"interface_version": "1.2", "nodes": []}));
}

#[test]
fn an_unknown_severity_is_a_schema_failure_not_a_hint() {
    let error = refused(json!({
        "interface_version": "1.1",
        "issues": [{"severity": "suggestion", "code": "X", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert!(error.contains("unknown severity 'suggestion'"), "{error}");
}

#[test]
fn malformed_interface_version_does_not_fall_back_to_legacy() {
    for version in [serde_json::Value::Null, json!(12)] {
        let error = refused(json!({"interface_version": version, "contract_version": "1.2"}));
        assert!(error.contains("interface_version"), "{error}");
        assert!(error.contains("must be a string"), "{error}");
    }
    ingest(json!({"interface_version": "1.2", "contract_version": null}));
}

// Requirement: adapter-contract/Document keys are closed

/// Every way a document can declare a supported version: the three accepted
/// versions, and the legacy key that stands in when `interface_version` is absent.
const VERSIONS: [(&str, &str); 4] = [
    ("interface_version", "1.0"),
    ("interface_version", "1.1"),
    ("interface_version", "1.2"),
    ("contract_version", "1.2"),
];

/// A document declaring `version` as `key`, carrying `rest`'s keys beside it.
fn versioned((key, version): (&str, &str), rest: Value) -> Value {
    let mut document = rest;
    document
        .as_object_mut()
        .expect("rest is an object")
        .insert(key.to_owned(), json!(version));
    document
}

fn finding(code: &str) -> Value {
    json!({"severity": "error", "code": code, "message": "m",
           "provenance": {"file": "r.md", "line": 1}, "node_id": null})
}

#[test]
fn closed_keys_refuse_an_unknown_top_level_key_under_every_version() {
    for version in VERSIONS {
        let error = refused(versioned(version, json!({"node": []})));
        assert!(
            error.contains("document: unknown key 'node'"),
            "{version:?}: {error}"
        );
    }
}

#[test]
fn closed_keys_are_checked_after_the_version() {
    let error = refused(json!({"interface_version": "9.9", "node": []}));
    assert!(
        error.contains("unsupported interface version '9.9'"),
        "{error}"
    );
}

#[test]
fn closed_keys_name_every_unknown_key_in_sorted_order() {
    let error = refused(json!({"interface_version": "1.2", "zeta": 1, "axes": []}));
    assert!(error.contains("unknown keys 'axes', 'zeta'"), "{error}");
}

#[test]
fn closed_keys_refuse_an_unknown_key_on_any_nested_object() {
    let provenance = json!({"file": "r.md", "line": 1});
    let cases = [
        (
            "node 0: unknown key 'attr'",
            json!({"nodes": [{"id": "REQ-1", "kind": "req", "attrs": {}, "attr": {},
                              "provenance": provenance}]}),
        ),
        (
            "edge 0: unknown key 'weight'",
            json!({"edges": [{"src": "A", "tgt": "B", "kind": "k", "weight": 1,
                              "provenance": provenance}]}),
        ),
        (
            "pathway 0: unknown key 'step'",
            json!({"pathways": [{"name": "m", "order": ["a"], "current": "a", "step": 1}]}),
        ),
        (
            "finding 0: unknown key 'hint'",
            json!({"findings": [{"severity": "info", "code": "X", "message": "m", "hint": "",
                                 "provenance": provenance, "node_id": null}]}),
        ),
        (
            "node 0 provenance: unknown key 'column'",
            json!({"nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                              "provenance": {"file": "r.md", "line": 1, "column": 3}}]}),
        ),
    ];
    for version in VERSIONS {
        for (expected, rest) in &cases {
            let error = refused(versioned(version, rest.clone()));
            assert!(error.contains(expected), "{version:?}: {error}");
        }
    }
}

#[test]
fn closed_keys_refuse_a_node_with_attr_for_attrs() {
    let error = refused(json!({"interface_version": "1.2",
        "nodes": [{"id": "REQ-1", "kind": "req", "attr": {},
                   "provenance": {"file": "r.md", "line": 1}}]}));
    assert!(error.contains("node 0: unknown key 'attr'"), "{error}");
}

#[test]
fn closed_keys_leave_the_contents_of_attrs_open() {
    let graph = ingest(json!({"interface_version": "1.2",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {"anything": {"nested": 1}},
                   "provenance": {"file": "r.md", "line": 1}}],
        "edges": [{"src": "REQ-1", "tgt": "REQ-1", "kind": "k", "attrs": {"free": true},
                   "provenance": {"file": "r.md", "line": 1}}]}));
    assert_eq!(graph.node("REQ-1").unwrap().attrs["anything"]["nested"], 1);
}

#[test]
fn closed_keys_admit_a_finding_state() {
    let mut entry = finding("X");
    entry["state"] = json!("unverified");
    let graph = ingest(json!({"interface_version": "1.2", "findings": [entry]}));
    assert_eq!(
        graph.adapter_issues()[0].state.as_deref(),
        Some("unverified")
    );
}

#[test]
fn alias_issues_is_consumed_when_findings_is_empty_under_every_version() {
    for version in VERSIONS {
        let graph = ingest(versioned(
            version,
            json!({"findings": [], "issues": [finding("FROM_ISSUES")]}),
        ));
        let codes: Vec<&str> = graph
            .adapter_issues()
            .iter()
            .map(|i| i.code.as_str())
            .collect();
        assert_eq!(codes, ["FROM_ISSUES"], "{version:?}");
    }
}

#[test]
fn alias_findings_is_consumed_when_issues_is_empty_under_every_version() {
    for version in VERSIONS {
        let graph = ingest(versioned(
            version,
            json!({"issues": [], "findings": [finding("FROM_FINDINGS")]}),
        ));
        let codes: Vec<&str> = graph
            .adapter_issues()
            .iter()
            .map(|i| i.code.as_str())
            .collect();
        assert_eq!(codes, ["FROM_FINDINGS"], "{version:?}");
    }
}

#[test]
fn alias_arrays_both_populated_are_refused_under_every_version() {
    for version in VERSIONS {
        let error = refused(versioned(
            version,
            json!({"issues": [finding("A")], "findings": [finding("B")]}),
        ));
        assert!(
            error.contains("'findings' and 'issues' are both non-empty"),
            "{version:?}: {error}"
        );
    }
}

#[test]
fn alias_arrays_are_both_validated() {
    let error = refused(json!({"interface_version": "1.2", "findings": [], "issues": null}));
    assert!(
        error.contains("'issues' must be a list, got null"),
        "{error}"
    );
    let error = refused(json!({"interface_version": "1.2", "findings": {}, "issues": []}));
    assert!(
        error.contains("'findings' must be a list, got object"),
        "{error}"
    );
}

// Requirement: adapter-contract/A node may say where each attr was written

#[test]
fn attr_provenance_must_name_an_attr_the_node_carries() {
    let error = latt_core::document::ingest_document(serde_json::json!({
        "interface_version": "1.4",
        "nodes": [{"id": "R-1", "kind": "req", "attrs": {"a": 1},
                   "provenance": {"file": "r", "line": 1},
                   "attr_provenance": {"b": {"file": "r", "line": 2}}}],
    }))
    .unwrap_err();
    assert_eq!(
        error.message(),
        "node 0 attr_provenance: names attr 'b', which the node's attrs do not hold"
    );
}

#[test]
fn attr_provenance_keys_are_closed_like_any_provenance() {
    let error = latt_core::document::ingest_document(serde_json::json!({
        "interface_version": "1.4",
        "nodes": [{"id": "R-1", "kind": "req", "attrs": {"a": 1},
                   "provenance": {"file": "r", "line": 1},
                   "attr_provenance": {"a": {"file": "r", "line": 2, "col": 3}}}],
    }))
    .unwrap_err();
    assert!(
        error.message().contains("unknown key 'col'"),
        "{}",
        error.message()
    );
}
