//! The `adapter-contract` capability's scenarios, against ingest.
//!
//! The failure modes that need a real subprocess live in `cli.rs`, where the
//! exit code they produce is observable. Everything here is about the document
//! itself: what a well-formed one carries through, and what shape is refused
//! rather than half-read.

mod common;

use common::ingest;
use lattice_core::document::{CONTRACT_VERSION, ingest_document, parse_document};
use lattice_core::types::Severity;
use serde_json::{Value, json};

/// The message of an ingest that was expected to be refused.
fn refused(document: Value) -> String {
    let rendered = document.to_string();
    match ingest_document(document) {
        Ok(_) => panic!("document ingested but should have been refused:\n{rendered}"),
        Err(e) => e.0,
    }
}

fn node(id: &str, kind: &str, line: i64) -> Value {
    json!({"id": id, "kind": kind, "attrs": {},
           "provenance": {"file": "r.md", "line": line}})
}

// Requirement: Contract document declares its version

#[test]
fn the_shipped_version_ingests() {
    let graph = ingest(json!({"contract_version": CONTRACT_VERSION, "nodes": []}));
    assert_eq!(graph.iter_nodes().count(), 0);
}

#[test]
fn an_unsupported_version_is_refused_naming_what_is_supported() {
    let error = refused(json!({"contract_version": "2.0", "nodes": []}));
    assert!(
        error.contains("unsupported contract version '2.0'"),
        "{error}"
    );
    assert!(error.contains(CONTRACT_VERSION), "{error}");
}

#[test]
fn a_document_with_no_version_is_refused() {
    assert!(refused(json!({"nodes": []})).contains("missing 'contract_version'"));
}

// Requirement: Document schema

#[test]
fn nodes_and_edges_ingest_with_their_provenance() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [node("REQ-1", "req", 3)],
        "edges": [{"src": "REQ-1", "tgt": "REQ-2", "kind": "derives",
                   "provenance": {"file": "r.md", "line": 7}}],
    }));
    let stored = graph.node("REQ-1").expect("the node was added");
    assert_eq!(stored.kind, "req");
    assert_eq!(stored.provenance.line, 3);
    let edge = graph.iter_edges().next().expect("the edge was added");
    assert_eq!(edge.provenance.line, 7);
}

#[test]
fn nested_attrs_survive_ingest_unflattened() {
    let graph = ingest(json!({
        "contract_version": "1.0",
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
        "contract_version": "1.0",
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
            "contract_version": "1.0",
            "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                       "provenance": {"file": "r.md", "line": line}}],
        });
        assert!(
            refused(document).contains("'line' must be an integer"),
            "line {line} must be refused"
        );
    }
}

#[test]
fn a_top_level_value_that_is_not_an_object_is_refused() {
    assert!(refused(json!([])).contains("expected an object"));
}

#[test]
fn attrs_that_are_not_an_object_are_refused() {
    let document = json!({
        "contract_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": ["text"],
                   "provenance": {"file": "r.md", "line": 1}}],
    });
    assert!(refused(document).contains("'attrs' must be an object"));
}

// Requirement: Adapter issue channel

#[test]
fn an_adapter_issue_keeps_its_severity_for_a_code_the_core_does_not_implement() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "issues": [{"severity": "info", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(graph.adapter_issues()[0].severity, Severity::Info);
}

#[test]
fn an_adapter_issues_node_id_travels_through_ingest() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": "REQ-1"}],
    }));
    assert_eq!(graph.adapter_issues()[0].node_id.as_deref(), Some("REQ-1"));
}

// Requirement: Document schema

#[test]
fn an_unknown_severity_is_refused_rather_than_defaulted() {
    let document = json!({
        "contract_version": "1.0",
        "issues": [{"severity": "critical", "code": "X", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    });
    assert!(refused(document).contains("unknown severity 'critical'"));
}

#[test]
fn a_node_id_that_is_not_a_string_is_refused() {
    let document = json!({
        "contract_version": "1.0",
        "issues": [{"severity": "warning", "code": "X", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": 3}],
    });
    assert!(refused(document).contains("'node_id' must be a string or null"));
}

// Requirement: Duplicate node IDs are resolved at ingest

#[test]
fn a_third_occurrence_is_reported_as_well_as_the_second() {
    let graph = ingest(json!({
        "contract_version": "1.0",
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
    assert_eq!(graph.node("REQ-1").unwrap().provenance.line, 1);
}

#[test]
fn the_adapters_own_issues_read_before_the_duplicate_findings() {
    let graph = ingest(json!({
        "contract_version": "1.0",
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

// Requirement: Axis storage on the graph

#[test]
fn an_axis_ingests_and_answers_membership_and_position() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "axes": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
    }));
    let axis = graph.axis("phase").expect("the axis was attached");
    assert!(axis.is_member("M0"));
    assert!(!axis.is_member("M9"));
    assert!(axis.is_after("M4"));
    assert!(!axis.is_after("CB"));
    assert!(
        !axis.is_after("M0"),
        "the current position is not after itself"
    );
}

// Requirement: Axis validity is the adapter's responsibility

#[test]
fn an_axis_whose_current_is_off_its_order_is_refused() {
    // It would place every bound finding both before and after itself. The
    // adapter read the declaration and can name the file, so this is its bug.
    let document = json!({
        "contract_version": "1.0",
        "axes": [{"name": "phase", "order": ["CB", "M0"], "current": "M9"}],
    });
    assert!(refused(document).contains("current position 'M9' is not in"));
}

#[test]
fn an_axis_with_repeated_positions_is_refused() {
    let document = json!({
        "contract_version": "1.0",
        "axes": [{"name": "phase", "order": ["M0", "M0"], "current": "M0"}],
    });
    assert!(refused(document).contains("positions are not unique"));
}

#[test]
fn an_axis_order_that_is_not_a_list_of_strings_is_refused() {
    let document = json!({
        "contract_version": "1.0",
        "axes": [{"name": "phase", "order": ["M0", 4], "current": "M0"}],
    });
    assert!(refused(document).contains("'order' must be a list of strings"));
}

// Requirement: Adapter failure is a broken setup, not a finding

#[test]
fn parse_reads_a_document_and_refuses_silence() {
    assert!(parse_document(r#"{"contract_version": "1.0"}"#).is_ok());
    for empty in ["", "   \n\t "] {
        let error = parse_document(empty).expect_err("silence is not an empty register");
        assert_eq!(error.0, "adapter emitted no output");
    }
    let error = parse_document("{oops").expect_err("malformed JSON is refused");
    assert!(
        error
            .0
            .starts_with("could not parse adapter output as JSON")
    );
}

// Requirement: Adapter is a program emitting a serialized graph

#[test]
fn a_document_carrying_only_issues_is_not_a_failure() {
    // An adapter that parsed nothing but has something to say about why is
    // reporting, not failing. Refusing here would turn a report into exit 2.
    let graph = ingest(json!({
        "contract_version": "1.0",
        "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(graph.iter_nodes().count(), 0);
    assert_eq!(graph.adapter_issues().len(), 1);
}

// Requirement: Resolved profile document

/// The profile file parsed independently of the loader, as plain JSON data.
fn profile_as_json(path: &str) -> Value {
    let text = std::fs::read_to_string(path).unwrap();
    serde_norway::from_str(&text).unwrap()
}

#[test]
fn resolved_document_round_trips_every_declared_field() {
    for path in [
        "../../profiles/requirements-rm.yaml",
        "../../profiles/toml.yaml",
    ] {
        let profile = lattice_core::profile::load_profile(std::path::Path::new(path)).unwrap();
        let text = lattice_core::profile::resolved_document(&profile).unwrap();
        let mut doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["resolved_schema"], "1", "{path}");
        doc.as_object_mut().unwrap().remove("resolved_schema");
        assert_eq!(doc, profile_as_json(path), "{path}");
    }
}

#[test]
fn resolved_document_keeps_raw_pattern_sources_and_adapter_namespace() {
    let profile = lattice_core::profile::load_profile(std::path::Path::new(
        "../../profiles/requirements-rm.yaml",
    ))
    .unwrap();
    let text = lattice_core::profile::resolved_document(&profile).unwrap();
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["node_kinds"]["req"]["id_pattern"], "^REQ-\\d{4}$");
    assert_eq!(doc["node_kinds"]["need"]["summary_attr"], "text");
    assert_eq!(
        doc["adapter"]["paths"]["requirements"],
        "docs/internal/REQUIREMENTS.md"
    );
    assert_eq!(doc["adapter"]["cited_path_prefixes"][0], "src/");
}

// Requirement: Contract 1.1 extends the severity vocabulary with hint

#[test]
fn a_hint_severity_issue_survives_ingest_at_1_1() {
    let graph = ingest(json!({
        "contract_version": "1.1",
        "issues": [{"severity": "hint", "code": "SUGGESTION", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(graph.adapter_issues()[0].severity, Severity::Hint);
}

#[test]
fn a_1_0_document_still_ingests_beside_1_1() {
    ingest(json!({"contract_version": "1.0", "nodes": []}));
    ingest(json!({"contract_version": "1.1", "nodes": []}));
}

#[test]
fn an_unknown_severity_is_a_schema_failure_not_a_hint() {
    let error = refused(json!({
        "contract_version": "1.1",
        "issues": [{"severity": "suggestion", "code": "X", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert!(error.contains("unknown severity 'suggestion'"), "{error}");
}
