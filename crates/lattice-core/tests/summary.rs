//! The `Summary subcommand` requirement's zero-config scenarios.
//!
//! A profile with no `SUMMARY` config gets a structural report (node and edge
//! counts by kind, finding tallies by code and severity) rather than exit 2.
//! The configured rollup's own scenarios stay in `cli.rs` and `output.rs`.

mod common;

use common::{Case, MINIMAL_PROFILE, code, ingest, profile_from, stderr, stdout};
use lattice_core::output::output_result;
use lattice_core::summary::build_summary;
use lattice_core::types::{Severity, SummaryReport};
use serde_json::{Value, json};

/// Two nodes, one edge between them, and one dangling edge: a `VACANCY` error
/// and an `UNREFERENCED` warning for the structural tallies to count.
const DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 1}},
            {"id": "REQ-2", "kind": "req", "attrs": {},
             "provenance": {"file": "r.md", "line": 2}}],
  "edges": [{"src": "REQ-1", "tgt": "REQ-2", "kind": "derives",
             "provenance": {"file": "r.md", "line": 1}},
            {"src": "REQ-2", "tgt": "REQ-9", "kind": "derives",
             "provenance": {"file": "r.md", "line": 2}}]}"#;

fn structural() -> SummaryReport {
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    let graph = ingest(serde_json::from_str(DOCUMENT).unwrap());
    build_summary(&profile, &graph).expect("no config is not an error")
}

// Requirement: Summary subcommand

#[test]
fn a_profile_with_no_summary_config_gets_a_structural_report() {
    let SummaryReport::Structural(report) = structural() else {
        panic!("no SUMMARY config selects the structural report");
    };
    assert_eq!(report.node_counts.get("req"), Some(&2));
    assert_eq!(report.edge_counts.get("derives"), Some(&2));
    let tally = |code: &str| {
        report
            .finding_counts
            .iter()
            .find(|t| t.code == code)
            .map(|t| (t.severity, t.count))
    };
    assert_eq!(tally("VACANCY"), Some((Severity::Error, 1)));
    assert_eq!(tally("UNREFERENCED"), Some((Severity::Warning, 1)));
    assert_eq!(tally("UNTRACED"), None, "every node has an outgoing edge");
}

#[test]
fn a_declared_kind_no_node_carries_still_counts_as_zero() {
    let profile = profile_from(
        "name: t\nprofile_version: \"1.0.0\"\n\
         node_kinds:\n  tst:\n    id_pattern: \"^TST-\\\\d+$\"\n\
         edge_kinds:\n  derives:\n    allowed: [[tst, tst]]\n",
    )
    .unwrap();
    let graph = ingest(json!({"interface_version": "1.0", "nodes": []}));
    let SummaryReport::Structural(report) = build_summary(&profile, &graph).unwrap() else {
        panic!("structural")
    };
    assert_eq!(report.node_counts.get("tst"), Some(&0));
    assert_eq!(report.edge_counts.get("derives"), Some(&0));
    assert!(report.finding_counts.is_empty());
}

#[test]
fn a_suppressed_finding_is_left_out_of_the_tallies() {
    let profile = profile_from(&format!(
        "{MINIMAL_PROFILE}validations:\n  - SUPPRESS:\n      code: VACANCY\n"
    ))
    .unwrap();
    let graph = ingest(serde_json::from_str(DOCUMENT).unwrap());
    let SummaryReport::Structural(report) = build_summary(&profile, &graph).unwrap() else {
        panic!("structural")
    };
    assert!(
        !report.finding_counts.iter().any(|t| t.code == "VACANCY"),
        "the profile said not to report it: {:?}",
        report.finding_counts
    );
}

#[test]
fn the_structural_report_renders_one_line_per_count_in_plain() {
    assert_eq!(
        output_result(&structural(), "plain").unwrap(),
        "node req 2\n\
         edge derives 2\n\
         finding UNREFERENCED warning 1\n\
         finding VACANCY error 1"
    );
}

#[test]
fn the_structural_report_nests_its_three_tables_in_json() {
    let rendered = output_result(&structural(), "json").unwrap();
    let parsed: Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(parsed["node_counts"]["req"], 2);
    assert_eq!(parsed["edge_counts"]["derives"], 2);
    assert_eq!(
        parsed["finding_counts"],
        json!([
            {"code": "UNREFERENCED", "count": 1, "severity": "warning"},
            {"code": "VACANCY", "count": 1, "severity": "error"},
        ])
    );
}

#[test]
fn the_structural_report_renders_three_sections_in_rich() {
    let rendered = output_result(&structural(), "rich").unwrap();
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(lines[0], "Nodes");
    assert!(lines.contains(&"Edges"));
    assert!(lines.contains(&"Findings"));
    assert!(
        lines
            .iter()
            .any(|l| l.contains("VACANCY") && l.contains("error")),
        "{rendered}"
    );
}

// Scenario: Zero-config structural report

#[test]
fn summary_with_no_config_exits_zero_and_prints_the_structural_report() {
    let case = Case::emitting(DOCUMENT);
    let output = case.run(&["summary", "--format", "plain"]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("node req 2"), "{out}");
    assert!(out.contains("edge derives 2"), "{out}");
    assert!(
        out.contains("finding VACANCY error 1"),
        "an error-severity finding is tallied, not exited on: {out}"
    );
}

// Scenario: Zero-config JSON format

#[test]
fn summary_with_no_config_emits_the_three_tables_as_json() {
    let case = Case::emitting(DOCUMENT);
    let output = case.run(&["summary", "--format", "json"]);

    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let parsed: Value = serde_json::from_str(&stdout(&output)).expect("parseable");
    assert!(parsed.get("node_counts").is_some());
    assert!(parsed.get("edge_counts").is_some());
    assert!(parsed.get("finding_counts").is_some());
}

// Scenario: Adapter errors surface after the rollup

#[test]
fn summary_with_no_config_still_exits_on_an_error_severity_adapter_issue() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let case = Case::emitting(document);
    let output = case.run(&["summary", "--format", "plain"]);

    assert_eq!(code(&output), 1);
    assert!(
        stdout(&output).contains("finding PARSE_ERROR error 1"),
        "{}",
        stdout(&output)
    );
    assert!(stderr(&output).contains("ERROR PARSE_ERROR"));
}
