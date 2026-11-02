//! A provenance line is an unsigned 32-bit number in the core, the adapter crate and
//! on the wire: ingest refuses what does not fit rather than narrowing it.

mod common;

use common::{MINIMAL_PROFILE, ingest, profile_from};
use latt_core::document::ingest_document;
use latt_core::service::{Origin, Provenance};
use latt_core::validate::validate;
use serde_json::{Value, json};

/// A document with one provenance at `line`, placed on a node, an edge, a finding
/// or an attr location.
fn document_with_line(location: &str, line: &Value) -> Value {
    let at = json!({"file": "r.md", "line": line});
    let one = json!({"file": "r.md", "line": 1});
    let mut node = json!({"id": "REQ-1", "kind": "req", "attrs": {"title": "t"},
                          "provenance": one});
    let mut edges = json!([]);
    let mut findings = json!([]);
    match location {
        "node" => node["provenance"] = at,
        "attr" => node["attr_provenance"] = json!({"title": at}),
        "edge" => {
            edges = json!([{"src": "REQ-1", "tgt": "REQ-1", "kind": "derives",
                            "provenance": at}]);
        }
        "finding" => {
            findings = json!([{"severity": "warning", "code": "PARSE_ERROR",
                               "message": "m", "provenance": at, "node_id": null}]);
        }
        other => panic!("unknown location {other}"),
    }
    json!({"interface_version": "1.4", "nodes": [node], "edges": edges,
           "findings": findings})
}

/// The line ingest stored at `location`.
fn stored_line(location: &str, line: &Value) -> u32 {
    let graph = ingest(document_with_line(location, line));
    match location {
        "node" => graph
            .iter_nodes()
            .next()
            .unwrap()
            .provenance
            .line()
            .map_or(0, u32::from),
        "attr" => graph
            .iter_nodes()
            .next()
            .unwrap()
            .provenance_of("title")
            .line()
            .map_or(0, u32::from),
        "edge" => graph
            .iter_edges()
            .next()
            .unwrap()
            .provenance
            .line()
            .map_or(0, u32::from),
        "finding" => graph.adapter_issues()[0]
            .origin
            .location()
            .expect("a file origin")
            .line()
            .map_or(0, u32::from),
        other => panic!("unknown location {other}"),
    }
}

/// The contract error for a document expected to be refused.
fn refusal(location: &str, line: &Value) -> String {
    match ingest_document(document_with_line(location, line)) {
        Ok(_) => panic!("{location} line {line} ingested but should have been refused"),
        Err(e) => e.message().to_owned(),
    }
}

const LOCATIONS: [&str; 4] = ["node", "edge", "finding", "attr"];

// Requirement: adapter-contract/Document schema

#[test]
fn zero_and_the_largest_32_bit_line_are_accepted_everywhere() {
    for location in LOCATIONS {
        assert_eq!(stored_line(location, &json!(0)), 0, "{location}");
        assert_eq!(
            stored_line(location, &json!(4_294_967_295_u64)),
            u32::MAX,
            "{location}"
        );
    }
}

#[test]
fn a_line_beyond_32_bits_is_refused_everywhere_naming_the_line() {
    for location in LOCATIONS {
        for line in [
            json!(4_294_967_296_u64),
            json!(9_223_372_036_854_775_808_u64),
            json!(u64::MAX),
        ] {
            let error = refusal(location, &line);
            assert!(
                error.contains(&format!("got {line}")) && error.contains("'line'"),
                "{location} {line}: {error}"
            );
        }
    }
}

#[test]
fn a_negative_line_is_refused_everywhere() {
    for location in LOCATIONS {
        let error = refusal(location, &json!(-1));
        assert!(
            error.contains("'line' must not be negative, got -1"),
            "{location}: {error}"
        );
    }
}

// Requirement: graph-service/A finding's provenance admits additions

#[test]
fn provenance_new_round_trips_a_u32_line() {
    let provenance = Provenance::new("REQUIREMENTS.md", 47);
    let line = provenance.line().map(u32::from);
    assert_eq!((provenance.file(), line), ("REQUIREMENTS.md", Some(47)));
    assert_eq!(line.map(i64::from), Some(47));
}

#[test]
fn a_whole_file_provenance_has_no_line() {
    let provenance = Provenance::new("REQUIREMENTS.md", 0);
    assert_eq!(provenance.line(), None);
    assert_eq!(provenance.to_string(), "REQUIREMENTS.md:0");
}

#[test]
fn the_line_read_from_a_validate_report_is_some_nonzero() {
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.4",
        "nodes": [{"id": "BAD", "kind": "req", "attrs": {},
                   "provenance": {"file": "r.md", "line": 4_294_967_295_u64}}],
    }));
    let issues = validate(&graph, &profile, false);
    let issue = issues
        .iter()
        .find(|issue| issue.node_id.as_deref() == Some("BAD"))
        .expect("an id-pattern finding on BAD");
    let line: Option<u32> = issue
        .origin
        .location()
        .expect("a file origin")
        .line()
        .map(u32::from);
    assert_eq!(line, Some(u32::MAX));
}

// Requirement: graph-service/A finding names its origin

#[test]
fn a_profile_finding_has_a_profile_origin() {
    let profile = profile_from(&format!(
        "{MINIMAL_PROFILE}validations:\n  - SUPPRESS: {{code: UNTRACED, node_ids: [REQ-9]}}\n"
    ))
    .unwrap();
    let graph = ingest(json!({"interface_version": "1.4", "nodes": []}));
    let issues = validate(&graph, &profile, false);
    let unused = issues
        .iter()
        .find(|issue| issue.code == "SUPPRESS_UNUSED")
        .expect("the SUPPRESS entry matched nothing");
    assert_eq!(unused.origin, Origin::Profile);
    assert_eq!(unused.origin.location(), None);
    assert_eq!(unused.origin.to_string(), "<profile>:0");
}

#[test]
fn a_provenance_is_a_file_origin() {
    let origin = Origin::from(Provenance::new("r.md", 3));
    assert_eq!(origin.location().map(Provenance::file), Some("r.md"));
    assert_eq!(origin.to_string(), "r.md:3");
    let whole_file = Origin::from(Provenance::new_file("r.md"));
    assert_eq!(whole_file.location().and_then(Provenance::line), None);
    assert_eq!(whole_file.to_string(), "r.md:0");
}
