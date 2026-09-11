//! The `coverage-report` capability's scenarios: `lattice coverage`, the
//! per-kind edge-direction pivot a consumer would otherwise build from trace
//! JSON. It is a report, not a validation pass, so what these check is the
//! statistics, the three renderings, and the two-valued exit contract.

mod common;

use common::{Case, MINIMAL_PROFILE, code, ingest, profile_from, stderr, stdout};
use lattice_core::output::output_result;
use lattice_core::query::coverage;
use lattice_core::types::CoverageReport;
use serde_json::Value;

/// Three declared kinds, one (`risk`) never instantiated. Among the four
/// `req` nodes: REQ-1 has both directions, REQ-2 only incoming, REQ-3 only
/// outgoing (to a target that was never declared), REQ-4 neither.
const COVERAGE_PROFILE: &str = r#"
name: c
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
  test:
    id_pattern: "^T-\\d+$"
  risk:
    id_pattern: "^RISK-\\d+$"
edge_kinds:
  derives:
    allowed: [[req, req]]
  verifies:
    allowed: [[test, req]]
"#;

const DOCUMENT: &str = r#"{"interface_version": "1.0",
  "nodes": [
    {"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 1}},
    {"id": "REQ-2", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 2}},
    {"id": "REQ-3", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 3}},
    {"id": "REQ-4", "kind": "req", "attrs": {}, "provenance": {"file": "r.md", "line": 4}},
    {"id": "T-1", "kind": "test", "attrs": {}, "provenance": {"file": "t.py", "line": 1}}],
  "edges": [
    {"src": "T-1", "tgt": "REQ-1", "kind": "verifies", "provenance": {"file": "t.py", "line": 1}},
    {"src": "REQ-1", "tgt": "REQ-2", "kind": "derives", "provenance": {"file": "r.md", "line": 1}},
    {"src": "REQ-3", "tgt": "REQ-9", "kind": "derives", "provenance": {"file": "r.md", "line": 3}}]}"#;

fn report() -> CoverageReport {
    let profile = profile_from(COVERAGE_PROFILE).unwrap();
    let graph = ingest(serde_json::from_str(DOCUMENT).unwrap());
    coverage(&graph, &profile)
}

fn row<'a>(report: &'a CoverageReport, kind: &str) -> &'a lattice_core::types::KindCoverage {
    report
        .kinds
        .iter()
        .find(|k| k.kind == kind)
        .unwrap_or_else(|| panic!("no row for {kind} in {:?}", report.kinds))
}

fn case() -> Case {
    Case::with_profile(COVERAGE_PROFILE, &format!("cat <<'DOC'\n{DOCUMENT}\nDOC"))
}

// Requirement: Coverage report computation

#[test]
fn counts_nodes_with_incoming_and_outgoing_edges_per_kind() {
    let report = report();
    let req = row(&report, "req");
    assert_eq!(
        (req.total, req.incoming, req.outgoing),
        (4, 2, 2),
        "{req:?}"
    );
    assert_eq!((req.incoming_pct, req.outgoing_pct), (50.0, 50.0));
    let test = row(&report, "test");
    assert_eq!((test.total, test.incoming, test.outgoing), (1, 0, 1));
    assert_eq!((test.incoming_pct, test.outgoing_pct), (0.0, 100.0));
}

#[test]
fn a_declared_kind_with_no_nodes_is_reported_at_zero() {
    let report = report();
    let risk = row(&report, "risk");
    assert_eq!((risk.total, risk.incoming, risk.outgoing), (0, 0, 0));
    assert_eq!((risk.incoming_pct, risk.outgoing_pct), (0.0, 0.0));
}

#[test]
fn a_dangling_endpoint_is_not_a_node_of_any_kind() {
    // REQ-9 is named by an edge but never declared: it counts toward no total,
    // while REQ-3's outgoing edge to it still counts for REQ-3.
    let report = report();
    let total: i64 = report.kinds.iter().map(|k| k.total).sum();
    assert_eq!(total, 5);
    assert_eq!(row(&report, "req").outgoing, 2);
}

#[test]
fn a_kind_the_profile_does_not_declare_still_appears() {
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    let graph = ingest(serde_json::json!({"interface_version": "1.0",
        "nodes": [{"id": "W-1", "kind": "widget", "attrs": {},
                   "provenance": {"file": "w.md", "line": 1}}]}));
    let report = coverage(&graph, &profile);
    assert_eq!(row(&report, "widget").total, 1);
    assert_eq!(row(&report, "req").total, 0);
}

#[test]
fn percentages_round_to_one_decimal() {
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    let nodes: Vec<Value> = (1..=3)
        .map(|i| {
            serde_json::json!({"id": format!("REQ-{i}"), "kind": "req", "attrs": {},
                "provenance": {"file": "r.md", "line": i}})
        })
        .collect();
    let graph = ingest(serde_json::json!({"interface_version": "1.0",
        "nodes": nodes,
        "edges": [{"src": "REQ-1", "tgt": "REQ-2", "kind": "derives",
                   "provenance": {"file": "r.md", "line": 1}}]}));
    let req = &coverage(&graph, &profile).kinds[0];
    assert_eq!((req.incoming_pct, req.outgoing_pct), (33.3, 33.3));
}

#[test]
fn kinds_are_ordered_by_name() {
    let report = report();
    let names: Vec<&str> = report.kinds.iter().map(|k| k.kind.as_str()).collect();
    assert_eq!(names, ["req", "risk", "test"]);
}

// Requirement: Coverage report output

#[test]
fn json_carries_a_kinds_array_with_six_fields_per_entry() {
    let rendered = output_result(&report(), "json").unwrap();
    let parsed: Value = serde_json::from_str(&rendered).unwrap();
    let kinds = parsed["kinds"].as_array().expect("kinds is an array");
    assert_eq!(kinds.len(), 3);
    assert_eq!(
        kinds[0],
        serde_json::json!({"kind": "req", "total": 4, "incoming": 2, "outgoing": 2,
                           "incoming_pct": 50.0, "outgoing_pct": 50.0})
    );
}

#[test]
fn plain_renders_a_header_row_and_one_row_per_kind() {
    assert_eq!(
        output_result(&report(), "plain").unwrap(),
        "kind total incoming incoming_pct outgoing outgoing_pct\n\
         req 4 2 50.0 2 50.0\n\
         risk 0 0 0.0 0 0.0\n\
         test 1 0 0.0 1 100.0"
    );
}

#[test]
fn rich_renders_aligned_columns_with_percentages_beside_counts() {
    let rendered = output_result(&report(), "rich").unwrap();
    let lines: Vec<&str> = rendered.lines().collect();
    assert!(lines[0].starts_with("Kind"), "{rendered}");
    assert!(lines[1].starts_with('-'), "{rendered}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("req") && l.contains("2 (50.0%)")),
        "{rendered}"
    );
    assert!(lines.iter().any(|l| l.contains("3 kind(s)")), "{rendered}");
}

// Requirement: Coverage report exit codes

#[test]
fn coverage_answers_on_stdout_and_exits_zero() {
    let output = case().run(&["coverage", "--format", "json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let parsed: Value = serde_json::from_str(&stdout(&output)).expect("parseable");
    assert_eq!(parsed["kinds"][0]["kind"], "req");
    assert_eq!(parsed["kinds"][0]["total"], 4);
}

#[test]
fn coverage_takes_no_strict_flag() {
    let refused = case().run(&["coverage", "--strict", "--format", "plain"]);
    assert_ne!(code(&refused), 0, "coverage must not accept --strict");
}

#[test]
fn coverage_never_exits_one_on_an_error_severity_adapter_issue() {
    let document = r#"{"interface_version": "1.0",
      "nodes": [],
      "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                  "provenance": {"file": "a.md", "line": 9}, "node_id": null}]}"#;
    let output = Case::emitting(document).run(&["coverage", "--format", "plain"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("req 0 0 0.0 0 0.0"),
        "{}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("ERROR PARSE_ERROR"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_adapter_that_fails_is_exit_two() {
    let output = Case::running("exit 3").run(&["coverage", "--format", "plain"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(stdout(&output).is_empty());
}
