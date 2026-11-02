//! The `output` and `trace-report` capabilities' scenarios.
//!
//! `trace_baseline.rs` pins the whole trace path to the Python core's bytes on
//! one real register. It cannot show that the *specified* order is what produced
//! them: the standing fixture happens to declare its nodes in an order the sort
//! agrees with. These cases construct the disagreements.

mod common;

use std::collections::BTreeMap;

use common::{ingest, profile_from};
use latt_core::output::{Format, ParseFormatError, output_result};
use latt_core::summary::build_summary;
use latt_core::trace::build_trace_report;
use latt_core::types::{
    CivilDate, Issue, Note, PathReport, Provenance, Severity, SummaryReport, TraceReport, Until,
};
use latt_core::validate::validate;
use serde_json::{Value, json};

/// `need` is declared before `req`, and `verifies` before `derives`, so the
/// specified orders and the declaration orders disagree in both directions.
const ORDER_PROFILE: &str = r#"
name: ordering
profile_version: "1.0.0"
node_kinds:
  need:
    id_pattern: "^UN-\\d+$"
  req:
    id_pattern: "^REQ-\\d+$"
edge_kinds:
  verifies:
    allowed: [[req, need]]
  derives:
    allowed: [[req, need]]
"#;

fn node(id: &str, kind: &str, line: i64) -> Value {
    json!({"id": id, "kind": kind, "attrs": {},
           "provenance": {"file": "r.md", "line": line}})
}

fn edge(src: &str, tgt: &str, kind: &str, line: i64) -> Value {
    json!({"src": src, "tgt": tgt, "kind": kind,
           "provenance": {"file": "r.md", "line": line}})
}

/// A trace report over the given document, with no findings attached.
fn trace_of(document: Value) -> TraceReport {
    let profile = profile_from(ORDER_PROFILE).unwrap();
    let graph = ingest(document);
    build_trace_report(&profile, &graph, Vec::new(), "0.0.0")
}

// Requirement: trace-report/Trace entry ordering

#[test]
fn entries_group_by_declared_kind_then_by_id() {
    // Declared last in the document, and `need` is declared first in the profile.
    let report = trace_of(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-2", "req", 1), node("REQ-1", "req", 2), node("UN-1", "need", 3)],
    }));
    let ids: Vec<&str> = report.entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["UN-1", "REQ-1", "REQ-2"]);
}

#[test]
fn edge_targets_are_ordered_lexicographically_within_a_kind() {
    let report = trace_of(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 1), node("UN-1", "need", 2),
                  node("UN-2", "need", 3), node("UN-3", "need", 4)],
        "edges": [edge("REQ-1", "UN-3", "derives", 5),
                  edge("REQ-1", "UN-1", "derives", 6),
                  edge("REQ-1", "UN-2", "derives", 7)],
    }));
    let entry = report.entries.iter().find(|e| e.id == "REQ-1").unwrap();
    let targets: Vec<&str> = entry.edges.iter().map(|edge| edge.tgt.as_str()).collect();
    assert_eq!(targets, ["UN-1", "UN-2", "UN-3"]);
}

#[test]
fn edge_kinds_are_ordered_lexicographically_not_by_declaration() {
    let report = trace_of(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 1), node("UN-1", "need", 2)],
        "edges": [edge("REQ-1", "UN-1", "verifies", 3),
                  edge("REQ-1", "UN-1", "derives", 4)],
    }));
    let entry = report.entries.iter().find(|e| e.id == "REQ-1").unwrap();
    let kinds: Vec<&str> = entry.edges.iter().map(|edge| edge.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["derives", "verifies"],
        "profile declares verifies first"
    );
}

#[test]
fn repeated_targets_are_preserved_and_ordered_by_the_referencing_line() {
    let report = trace_of(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 1), node("UN-1", "need", 2)],
        "edges": [edge("REQ-1", "UN-1", "derives", 9),
                  edge("REQ-1", "UN-1", "derives", 4)],
    }));
    let entry = report.entries.iter().find(|e| e.id == "REQ-1").unwrap();
    let targets: Vec<&str> = entry.edges.iter().map(|edge| edge.tgt.as_str()).collect();
    assert_eq!(targets, ["UN-1", "UN-1"], "not deduplicated");
}

#[test]
fn ordering_is_independent_of_build_order() {
    let forwards = json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 1), node("UN-1", "need", 2), node("UN-2", "need", 3)],
        "edges": [edge("REQ-1", "UN-1", "derives", 4), edge("REQ-1", "UN-2", "verifies", 5)],
    });
    let backwards = json!({
        "interface_version": "1.0",
        "nodes": [node("UN-2", "need", 3), node("UN-1", "need", 2), node("REQ-1", "req", 1)],
        "edges": [edge("REQ-1", "UN-2", "verifies", 5), edge("REQ-1", "UN-1", "derives", 4)],
    });
    for &format in Format::ALL {
        assert_eq!(
            output_result(&trace_of(forwards.clone()), format),
            output_result(&trace_of(backwards.clone()), format),
            "{format} depends on the order the register was written in"
        );
    }
}

// Requirement: trace-report/Trace report structure

#[test]
fn a_finding_on_a_ghost_node_is_unattachable_and_invents_no_entry() {
    let profile = profile_from(ORDER_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 1)],
        "edges": [edge("REQ-1", "UN-9", "derives", 2)],
    }));
    let issues = validate(&graph, &profile, false);
    let report = build_trace_report(&profile, &graph, issues, "0.0.0");

    let dangling = report
        .unattachable_findings
        .iter()
        .chain(report.entries.iter().flat_map(|e| &e.findings))
        .filter(|f| f.code == "VACANCY")
        .count();
    assert_eq!(dangling, 1, "the finding appears exactly once");
    // It names REQ-1, which does exist, so it attaches rather than going to the footer.
    let req = report.entries.iter().find(|e| e.id == "REQ-1").unwrap();
    assert!(req.findings.iter().any(|f| f.code == "VACANCY"));
    assert!(!report.entries.iter().any(|e| e.id == "UN-9"));
}

#[test]
fn a_finding_with_no_node_id_goes_to_the_footer() {
    let profile = profile_from(ORDER_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [node("REQ-1", "req", 1)],
        "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "m",
                    "provenance": {"file": "r.md", "line": 3}, "node_id": null}],
    }));
    let issues = validate(&graph, &profile, false);
    let report = build_trace_report(&profile, &graph, issues, "0.0.0");

    let unattachable: Vec<&str> = report
        .unattachable_findings
        .iter()
        .map(|f| f.code.as_str())
        .filter(|code| *code != "EMPTY_KIND")
        .collect();
    assert_eq!(unattachable, ["PARSE_ERROR"]);

    // The footer section is what makes it visible in plain output.
    let plain = output_result(&report, Format::Plain);
    assert!(plain.contains("Unattachable findings:"), "{plain}");
    assert!(plain.contains("WARNING PARSE_ERROR r.md:3 m"), "{plain}");
}

#[test]
fn the_header_carries_both_version_axes_and_the_profile_name() {
    let report = trace_of(json!({"interface_version": "1.0"}));
    assert_eq!(report.header["profile"], "ordering");
    assert_eq!(report.header["profile_version"], "1.0.0");
    assert_eq!(report.header["lattice_version"], "0.0.0");
    assert_eq!(report.header["trace_version"], "2");

    let json: Value = serde_json::from_str(&output_result(&report, Format::Json)).unwrap();
    for key in ["header", "entries", "unattachable_findings", "pathways"] {
        assert!(json.get(key).is_some(), "{key} is a public top-level field");
    }
}

#[test]
fn trace_json_carries_pathways() {
    let report = trace_of(json!({
        "interface_version": "1.2",
        "pathways": [{"name": "stage", "order": ["one", "two"], "current": "one"}],
    }));
    let json: Value = serde_json::from_str(&output_result(&report, Format::Json)).unwrap();
    assert_eq!(
        json["pathways"],
        json!([{"name": "stage", "order": ["one", "two"], "current": "one"}])
    );
}

#[test]
fn trace_edges_carry_kind_attrs_and_provenance_as_objects() {
    let report = trace_of(json!({
        "interface_version": "1.2",
        "nodes": [node("REQ-1", "req", 1), node("UN-1", "need", 2)],
        "edges": [{"src": "REQ-1", "tgt": "UN-1", "kind": "derives",
                   "attrs": {"confidence": 0.75},
                   "provenance": {"file": "r.md", "line": 9}}],
    }));
    let rendered = output_result(&report, Format::Json);
    let json: Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(json["entries"][1]["edges"][0]["tgt"], "UN-1");
    assert_eq!(json["entries"][1]["edges"][0]["kind"], "derives");
    assert_eq!(json["entries"][1]["edges"][0]["attrs"]["confidence"], 0.75);
    assert_eq!(json["entries"][1]["edges"][0]["provenance"]["file"], "r.md");
    assert_eq!(json["entries"][1]["edges"][0]["provenance"]["line"], 9);
}

// Requirement: output/Trace report plain format

/// A profile with non-default vocabulary proves the column is data-driven.
const SUMMARY_ATTR_PROFILE: &str = r#"
name: custom
profile_version: "1.0.0"
node_kinds:
  widget:
    id_pattern: "^W-\\d+$"
    summary_attr: label
    attrs:
      label: {type: string, required: true}
  part:
    id_pattern: "^P-\\d+$"
edge_kinds:
  uses:
    allowed: [[widget, part]]
"#;

#[test]
fn trace_key_attr_column_is_filled_from_the_profiles_summary_attr() {
    let profile = profile_from(SUMMARY_ATTR_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "W-1", "kind": "widget",
             "attrs": {"label": "Gizmo"},
             "provenance": {"file": "r.md", "line": 1}},
            {"id": "P-1", "kind": "part",
             "attrs": {},
             "provenance": {"file": "r.md", "line": 2}},
        ],
    }));
    let report = build_trace_report(&profile, &graph, Vec::new(), "0.0.0");

    let plain = output_result(&report, Format::Plain);
    assert!(
        plain.contains("Gizmo"),
        "summary attr value appears: {plain}"
    );

    let w_entry = report.entries.iter().find(|e| e.id == "W-1").unwrap();
    assert_eq!(w_entry.summary_attr.as_deref(), Some("label"));

    let p_entry = report.entries.iter().find(|e| e.id == "P-1").unwrap();
    assert_eq!(
        p_entry.summary_attr, None,
        "kind with no summary_attr is blank"
    );
}

#[test]
fn path_report_rejects_a_found_path_without_nodes() {
    assert!(PathReport::found("A".to_owned(), "B".to_owned(), vec![], vec![]).is_err());
}

#[test]
fn path_report_rejects_mismatched_edge_count() {
    assert!(
        PathReport::found(
            "A".to_owned(),
            "C".to_owned(),
            vec!["A".to_owned(), "B".to_owned(), "C".to_owned()],
            vec!["e1".to_owned()],
        )
        .is_err()
    );
}

#[test]
fn path_report_rejects_wrong_first_node() {
    assert!(
        PathReport::found(
            "A".to_owned(),
            "B".to_owned(),
            vec!["X".to_owned(), "B".to_owned()],
            vec!["e1".to_owned()],
        )
        .is_err()
    );
}

#[test]
fn path_report_rejects_wrong_last_node() {
    assert!(
        PathReport::found(
            "A".to_owned(),
            "B".to_owned(),
            vec!["A".to_owned(), "X".to_owned()],
            vec!["e1".to_owned()],
        )
        .is_err()
    );
}

#[test]
fn path_report_accepts_self_loop() {
    assert!(
        PathReport::found("A".to_owned(), "A".to_owned(), vec!["A".to_owned()], vec![],).is_ok()
    );
}

/// Every field names itself, so the line reads in any order: no padding, no
/// position to remember. The free-text summary attr goes last, whole.
#[test]
fn plain_trace_and_at_lines_label_every_field() {
    let profile = profile_from(SUMMARY_ATTR_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "W-1", "kind": "widget",
             "attrs": {"label": "A gizmo whose label runs past thirty characters"},
             "provenance": {"file": "r.md", "line": 1}},
            {"id": "P-1", "kind": "part",
             "attrs": {},
             "provenance": {"file": "r.md", "line": 2}},
        ],
        "edges": [
            {"src": "W-1", "tgt": "P-1", "kind": "uses", "attrs": {},
             "provenance": {"file": "r.md", "line": 1}},
        ],
    }));
    let expected = "id:W-1 kind:widget edges:1 findings:0 \
                    label:A gizmo whose label runs past thirty characters\n\
                    id:P-1 kind:part edges:0 findings:0";

    let report = build_trace_report(&profile, &graph, Vec::new(), "0.0.0");
    assert_eq!(output_result(&report, Format::Plain), expected);

    let at = latt_core::query::at(
        &graph,
        &profile,
        Vec::new(),
        std::path::Path::new("/nonexistent"),
        "r.md",
        "0.0.0",
        &[],
    )
    .unwrap();
    assert_eq!(output_result(&at, Format::Plain), expected);
}

#[test]
fn plain_trace_line_keeps_a_multi_line_summary_on_one_line() {
    let profile = profile_from(SUMMARY_ATTR_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "W-1", "kind": "widget",
             "attrs": {"label": "First line\n\n\tsecond  line\r\n"},
             "provenance": {"file": "r.md", "line": 1}},
        ],
        "edges": [],
    }));
    let report = build_trace_report(&profile, &graph, Vec::new(), "0.0.0");
    assert_eq!(
        output_result(&report, Format::Plain),
        "id:W-1 kind:widget edges:0 findings:0 label:First line second line"
    );
}

#[test]
fn summary_attr_renders_blank_when_the_node_omits_the_configured_attr() {
    let profile = profile_from(SUMMARY_ATTR_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "W-1", "kind": "widget",
             "attrs": {},
             "provenance": {"file": "r.md", "line": 1}},
        ],
    }));
    let report = build_trace_report(&profile, &graph, Vec::new(), "0.0.0");
    let plain = output_result(&report, Format::Plain);
    // The key-attr column should be blank spaces, not a crash or "null".
    assert!(
        !plain.contains("null"),
        "missing attr should not render as 'null': {plain}"
    );
    assert!(
        !plain.contains("None"),
        "missing attr should not render as 'None': {plain}"
    );
}

// Requirement: output/Key attr scalars render as JSON scalars

#[test]
fn summary_attr_renders_an_int_typed_attr() {
    let yaml = r#"
name: inttest
profile_version: "1.0.0"
node_kinds:
  item:
    id_pattern: "^I-\\d+$"
    summary_attr: count
    attrs:
      count: {type: int}
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [
            {"id": "I-1", "kind": "item",
             "attrs": {"count": 42},
             "provenance": {"file": "r.md", "line": 1}},
        ],
    }));
    let report = build_trace_report(&profile, &graph, Vec::new(), "0.0.0");
    let plain = output_result(&report, Format::Plain);
    assert!(
        plain.contains("42"),
        "int attr value appears in trace: {plain}"
    );
}

// Requirement: output/Three output formats

#[test]
fn findings_render_one_line_each_in_plain() {
    let issue = Issue::new(
        Severity::Warning,
        "UNREFERENCED",
        "node 'REQ-9999' has no incoming edges",
        Provenance::new("REQS.md", 42),
        Some("REQ-9999".into()),
    );
    assert_eq!(
        output_result(&vec![issue], Format::Plain),
        "WARNING UNREFERENCED REQS.md:42 node 'REQ-9999' has no incoming edges"
    );
}

#[test]
fn a_finding_in_json_carries_every_documented_field() {
    let issue = Issue::new(
        Severity::Warning,
        "UNREFERENCED",
        "m",
        Provenance::new("REQS.md", 42),
        Some("REQ-9999".into()),
    );
    let rendered = output_result(&vec![issue], Format::Json);
    let parsed: Value = serde_json::from_str(&rendered).unwrap();
    let finding = &parsed["findings"][0];
    for key in ["severity", "code", "file", "line", "message", "node_id"] {
        assert!(finding.get(key).is_some(), "{key} missing from {rendered}");
    }
}

// Requirement: output/Deterministic output ordering

/// Two groups over three status columns, one of which no node carries.
///
/// Every group carries every status key, which is how the rollup is built. A
/// declared status with no nodes is a zero column, not an absent one.
fn summary() -> SummaryReport {
    let profile = profile_from(
        r#"
name: rollup
profile_version: "1.0.0"
node_kinds:
  goal:
    id_pattern: "^G-\\d+$"
    attrs:
      status: { type: enum, values: [blocked, done, todo], required: true }
      file: { type: string, required: true }
edge_kinds: {}
validations:
  - SUMMARY:
      node_kind: goal
      status_attr: status
      group_by_attr: file
"#,
    )
    .unwrap();
    let goal = |id: &str, file: &str, status: &str| {
        json!({"id": id, "kind": "goal", "attrs": {"file": file, "status": status},
               "provenance": {"file": file, "line": 1}})
    };
    let graph = ingest(json!({
        "interface_version": "1.2",
        "nodes": [goal("G-1", "a.md", "done"), goal("G-2", "a.md", "done"),
                  goal("G-3", "a.md", "todo"), goal("G-4", "b.md", "done")],
        "edges": [], "pathways": [], "findings": []
    }));
    let report = build_summary(&profile, &graph).unwrap();
    let SummaryReport::Configured(rollup) = &report else {
        panic!("a SUMMARY config selects the status rollup");
    };
    let row = |pairs: &[(&str, i64)]| -> BTreeMap<String, i64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    };
    assert_eq!(rollup.group_key, "file");
    assert_eq!(rollup.status_keys, ["blocked", "done", "todo"]);
    assert_eq!(
        rollup.groups,
        [
            (
                "a.md".to_string(),
                row(&[("blocked", 0), ("done", 2), ("todo", 1), ("total", 3)])
            ),
            (
                "b.md".to_string(),
                row(&[("blocked", 0), ("done", 1), ("todo", 0), ("total", 1)])
            ),
        ]
    );
    report
}

#[test]
fn the_rollup_renders_one_row_per_group_in_plain() {
    assert_eq!(
        output_result(&summary(), Format::Plain),
        "a.md: blocked=0 done=2 todo=1 total=3\nb.md: blocked=0 done=1 todo=0 total=1"
    );
}

// Requirement: coverage-query/Summary subcommand

#[test]
fn the_rollup_carries_a_column_for_a_status_no_row_holds() {
    let rendered = output_result(&summary(), Format::Json);
    let parsed: Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(
        parsed["groups"][0]["blocked"], 0,
        "a declared status keeps its column"
    );
    assert_eq!(parsed["groups"][0]["file"], "a.md");
    assert_eq!(parsed["totals"]["done"], 3);
    assert_eq!(parsed["totals"]["total"], 4);
}

#[test]
fn the_rollup_rich_table_capitalises_and_truncates_its_column_labels() {
    let rendered = output_result(&summary(), Format::Rich);
    let header = rendered.lines().next().unwrap();
    // Python's capitalize lowercases the tail, and the label is cut to five.
    assert!(header.contains("Block"), "{header}");
    assert!(header.contains("Total"), "{header}");
    assert!(header.starts_with("File "), "{header}");
    assert!(rendered.lines().last().unwrap().starts_with("TOTAL"));
}

// Requirement: output/Deterministic output ordering

#[test]
fn every_format_is_byte_stable_across_runs() {
    for &format in Format::ALL {
        assert_eq!(
            output_result(&summary(), format),
            output_result(&summary(), format)
        );
    }
}

// Requirement: output/Hint severity rendering

#[test]
fn a_hint_finding_renders_in_all_three_formats() {
    let issue = Issue::new(
        Severity::Hint,
        "COVERAGE_UNKNOWN",
        "m",
        Provenance::new("<profile>", 0),
        None,
    );
    assert_eq!(
        output_result(&vec![issue.clone()], Format::Plain),
        "HINT COVERAGE_UNKNOWN <profile>:0 m"
    );
    let parsed: Value =
        serde_json::from_str(&output_result(&vec![issue.clone()], Format::Json)).unwrap();
    assert_eq!(parsed["findings"][0]["severity"], "hint");
    let rich = output_result(&vec![issue], Format::Rich);
    assert!(rich.contains("HINT"), "{rich}");
    assert!(rich.contains("1 hint(s)"), "{rich}");
}

// Requirement: output/Findings JSON carries state when present

#[test]
fn a_state_carrying_finding_serializes_its_state() {
    let mut issue = Issue::new(
        Severity::Warning,
        "COVERAGE",
        "m",
        Provenance::new("REQS.md", 42),
        Some("REQ-0001".into()),
    );
    issue.state = Some("unknown".into());
    let parsed: Value = serde_json::from_str(&output_result(&vec![issue], Format::Json)).unwrap();
    assert_eq!(parsed["findings"][0]["state"], "unknown");
}

#[test]
fn a_stateless_finding_carries_no_state_key() {
    let issue = Issue::new(
        Severity::Warning,
        "UNREFERENCED",
        "m",
        Provenance::new("REQS.md", 42),
        None,
    );
    let parsed: Value = serde_json::from_str(&output_result(&vec![issue], Format::Json)).unwrap();
    assert!(parsed["findings"][0].get("state").is_none());
}

// Requirement: output/Suggestion rendering

/// A suggestion is a finding like any other. It carries the source node's
/// provenance, so acting on it sends the reviewer to the line they would edit,
/// and it adds no field to the findings JSON. An optional, advisory overlay
/// must not widen a schema every consumer reads.
#[test]
fn a_suggestion_renders_as_a_hint_in_all_three_formats() {
    let issue = Issue::new(
        Severity::Hint,
        "SUGGESTED_EDGE",
        "'T-1' verifies 'REQ-0001' (score 0.8312, latt-suggest 0.1.0): cosine 0.8312",
        Provenance::new("tests/test_torque.py", 42),
        Some("T-1".into()),
    );

    assert_eq!(
        output_result(&vec![issue.clone()], Format::Plain),
        "HINT SUGGESTED_EDGE tests/test_torque.py:42 \
'T-1' verifies 'REQ-0001' (score 0.8312, latt-suggest 0.1.0): cosine 0.8312"
    );

    let rich = output_result(&vec![issue.clone()], Format::Rich);
    assert!(rich.contains("HINT"), "{rich}");
    assert!(rich.contains("1 hint(s)"), "{rich}");

    let ordinary = Issue::new(
        Severity::Warning,
        "UNREFERENCED",
        "m",
        Provenance::new("reqs.md", 3),
        Some("REQ-0001".into()),
    );
    let suggestion: Value =
        serde_json::from_str(&output_result(&vec![issue], Format::Json)).unwrap();
    let other: Value = serde_json::from_str(&output_result(&vec![ordinary], Format::Json)).unwrap();

    assert_eq!(suggestion["findings"][0]["severity"], "hint");
    assert_eq!(suggestion["findings"][0]["code"], "SUGGESTED_EDGE");
    assert_eq!(
        keys(&suggestion["findings"][0]),
        keys(&other["findings"][0]),
        "the overlay widened the findings schema"
    );
}

/// An unresolved suggestion renders as a hint too: a stale scratch document is
/// not a fault in the register, and must never be able to gate a run.
#[test]
fn an_unresolved_suggestion_renders_as_a_hint() {
    let issue = Issue::new(
        Severity::Hint,
        "SUGGESTION_UNRESOLVED",
        "suggestion from 'p' proposes 'verifies' T-1 -> REQ-9: REQ-9 does not resolve",
        Provenance::new("<suggestions>", 0),
        None,
    );

    let plain = output_result(&vec![issue], Format::Plain);
    assert!(
        plain.starts_with("HINT SUGGESTION_UNRESOLVED <suggestions>:0"),
        "{plain}"
    );
}

/// An unranked count renders as a finding in every format, with no new field:
/// an unranked run must be visible to whichever format a caller reads.
#[test]
fn an_unranked_count_renders_as_a_hint_in_all_three_formats() {
    let issue = Issue::new(
        Severity::Hint,
        "SUGGESTION_UNRANKED",
        "suggestion producer 'p' left 3 candidate source(s) unranked; \
         its absent suggestions are not a ranking",
        Provenance::new("<suggestions>", 0),
        None,
    );

    let plain = output_result(&vec![issue.clone()], Format::Plain);
    assert!(
        plain.starts_with("HINT SUGGESTION_UNRANKED <suggestions>:0"),
        "{plain}"
    );
    let rich = output_result(&vec![issue.clone()], Format::Rich);
    assert!(rich.contains("SUGGESTION_UNRANKED"), "{rich}");
    assert!(rich.contains("1 hint(s)"), "{rich}");
    let json: Value = serde_json::from_str(&output_result(&vec![issue], Format::Json)).unwrap();
    assert_eq!(json["findings"][0]["code"], "SUGGESTION_UNRANKED");
    assert_eq!(json["findings"][0]["severity"], "hint");
}

/// The field names of one findings entry, sorted.
fn keys(entry: &Value) -> Vec<String> {
    let mut names: Vec<String> = entry
        .as_object()
        .expect("a findings entry is an object")
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

// Requirement: output/Suppressed finding rendering

fn suppressed_vacancy() -> Issue {
    let mut issue = Issue::new(
        Severity::Error,
        "VACANCY",
        "edge 'UN-1'->'UN-9' (kind 'derives'): target 'UN-9' does not exist",
        Provenance::new("needs.md", 4),
        Some("UN-1".into()),
    );
    issue.suppressed = true;
    issue
}

fn unsuppressed_unreferenced() -> Issue {
    Issue::new(
        Severity::Warning,
        "UNREFERENCED",
        "node 'UN-2' has no incoming edges",
        Provenance::new("needs.md", 8),
        Some("UN-2".into()),
    )
}

#[test]
fn json_retains_a_suppressed_finding_with_every_field() {
    let mut unsuppressed = suppressed_vacancy();
    unsuppressed.suppressed = false;
    let before: Value =
        serde_json::from_str(&output_result(&vec![unsuppressed], Format::Json)).unwrap();
    let after: Value =
        serde_json::from_str(&output_result(&vec![suppressed_vacancy()], Format::Json)).unwrap();
    let entry = &after["findings"][0];
    assert_eq!(entry["suppressed"], true);
    for key in ["severity", "code", "file", "line", "message", "node_id"] {
        assert_eq!(entry[key], before["findings"][0][key], "{key}");
    }
    assert_eq!(entry["severity"], "error");
}

#[test]
fn an_unsuppressed_finding_carries_no_suppressed_key() {
    let rendered = output_result(&vec![unsuppressed_unreferenced()], Format::Json);
    let parsed: Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(
        keys(&parsed["findings"][0]),
        ["code", "file", "line", "message", "node_id", "severity"]
    );
}

#[test]
fn suppressed_findings_keep_their_place_in_json_ordering() {
    // needs.md:4 sorts before needs.md:8, suppressed or not.
    let issues = vec![unsuppressed_unreferenced(), suppressed_vacancy()];
    let parsed: Value = serde_json::from_str(&output_result(&issues, Format::Json)).unwrap();
    let findings = parsed["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0]["code"], "VACANCY");
    assert_eq!(findings[0]["suppressed"], true);
    assert_eq!(findings[1]["code"], "UNREFERENCED");
    assert!(findings[1].get("suppressed").is_none());
}

#[test]
fn plain_omits_a_suppressed_finding() {
    let issues = vec![suppressed_vacancy(), unsuppressed_unreferenced()];
    assert_eq!(
        output_result(&issues, Format::Plain),
        "WARNING UNREFERENCED needs.md:8 node 'UN-2' has no incoming edges"
    );
    assert_eq!(
        output_result(&vec![suppressed_vacancy()], Format::Plain),
        ""
    );
}

#[test]
fn rich_omits_a_suppressed_finding_and_counts_only_what_it_shows() {
    let issues = vec![suppressed_vacancy(), unsuppressed_unreferenced()];
    let rich = output_result(&issues, Format::Rich);
    assert!(rich.contains("UNREFERENCED"), "{rich}");
    assert!(!rich.contains("VACANCY"), "{rich}");
    assert!(rich.contains("1 warning(s)"), "{rich}");
    assert!(!rich.contains("error(s)"), "{rich}");
    assert_eq!(
        output_result(&vec![suppressed_vacancy()], Format::Rich),
        "No findings."
    );
}

#[test]
fn a_suppressed_hint_follows_the_suppression_rule() {
    let mut hint = Issue::new(
        Severity::Hint,
        "COVERAGE_UNKNOWN",
        "m",
        Provenance::new("<profile>", 0),
        None,
    );
    hint.suppressed = true;
    assert_eq!(output_result(&vec![hint.clone()], Format::Plain), "");
    assert_eq!(
        output_result(&vec![hint.clone()], Format::Rich),
        "No findings."
    );
    let parsed: Value = serde_json::from_str(&output_result(&vec![hint], Format::Json)).unwrap();
    assert_eq!(parsed["findings"][0]["severity"], "hint");
    assert_eq!(parsed["findings"][0]["suppressed"], true);
}

#[test]
fn trace_formats_follow_the_suppression_rule() {
    let profile = profile_from(ORDER_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [node("UN-1", "need", 1), node("UN-2", "need", 2)],
    }));
    let mut attached = suppressed_vacancy();
    attached.origin = Provenance::new("r.md", 1).into();
    let mut footer = unsuppressed_unreferenced();
    footer.node_id = None;
    footer.suppressed = true;
    let report = build_trace_report(&profile, &graph, vec![attached, footer], "0.0.0");

    let plain = output_result(&report, Format::Plain);
    assert!(!plain.contains("VACANCY"), "{plain}");
    assert!(!plain.contains("Unattachable"), "{plain}");
    assert!(plain.contains("findings:0"), "{plain}");
    let rich = output_result(&report, Format::Rich);
    assert!(rich.contains("no findings"), "{rich}");
    let parsed: Value = serde_json::from_str(&output_result(&report, Format::Json)).unwrap();
    let entry = parsed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "UN-1")
        .unwrap();
    assert_eq!(entry["findings"][0]["suppressed"], true);
    assert_eq!(parsed["unattachable_findings"][0]["suppressed"], true);
}

// Requirement: output/Three output formats

#[test]
fn json_escapes_non_ascii_as_utf16_units() {
    let issue = Issue::new(
        Severity::Warning,
        "UNREFERENCED",
        "caf\u{e9} \u{1F600}",
        Provenance::new("REQS.md", 1),
        None,
    );
    let rendered = output_result(&vec![issue], Format::Json);
    assert!(rendered.is_ascii(), "{rendered}");
    assert!(
        rendered.contains(r"caf\u00e9 \ud83d\ude00"),
        "escape is lower-case, four digits per UTF-16 unit:\n{rendered}"
    );
    let parsed: Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(parsed["findings"][0]["message"], "caf\u{e9} \u{1F600}");
}

// Requirement: output/Noted finding rendering

fn noted_vacancy(expired: bool, by: Option<&str>) -> Issue {
    let mut issue = suppressed_vacancy();
    issue.suppressed = false;
    let mut note = Note::new("Lands with M2.");
    note.until = Some(Until::Date(CivilDate::parse("2026-12-31").unwrap()));
    note.by = by.map(str::to_owned);
    note.expired = expired;
    issue.note = Some(note);
    issue
}

#[test]
fn note_serialized() {
    let parsed: Value = serde_json::from_str(&output_result(
        &vec![noted_vacancy(false, None)],
        Format::Json,
    ))
    .unwrap();
    assert_eq!(
        parsed["findings"][0]["note"],
        json!({"text": "Lands with M2.", "until": "2026-12-31"})
    );
}

#[test]
fn note_expired_serialized() {
    let parsed: Value = serde_json::from_str(&output_result(
        &vec![noted_vacancy(true, None)],
        Format::Json,
    ))
    .unwrap();
    assert_eq!(parsed["findings"][0]["note"]["expired"], true);
}

#[test]
fn note_pathway_until_serialized_as_written() {
    let mut issue = noted_vacancy(false, None);
    issue.note.as_mut().unwrap().until = Some(Until::Pathway {
        pathway: "stage".into(),
        position: "M2".into(),
    });
    let parsed: Value = serde_json::from_str(&output_result(&vec![issue], Format::Json)).unwrap();
    assert_eq!(
        parsed["findings"][0]["note"]["until"],
        json!({"pathway": "stage", "position": "M2"})
    );
}

#[test]
fn note_absent_leaves_the_entry_unchanged() {
    let text = output_result(&vec![unsuppressed_unreferenced()], Format::Json);
    assert!(!text.contains("note"), "{text}");
    let parsed: Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<&String> = parsed["findings"][0].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["code", "file", "line", "message", "node_id", "severity"]
    );
}

#[test]
fn note_plain_line() {
    let plain = output_result(&vec![noted_vacancy(false, Some("jone"))], Format::Plain);
    assert_eq!(plain.lines().count(), 1, "{plain}");
    assert!(
        plain.ends_with("| note: Lands with M2. [jone, until 2026-12-31]"),
        "{plain}"
    );
    let expired = output_result(&vec![noted_vacancy(true, None)], Format::Plain);
    assert!(
        expired.ends_with("| note: (expired) Lands with M2. [until 2026-12-31]"),
        "{expired}"
    );
}

#[test]
fn note_rich_line() {
    let rich = output_result(&vec![noted_vacancy(false, Some("jone"))], Format::Rich);
    let lines: Vec<&str> = rich.lines().collect();
    assert!(lines[0].contains("VACANCY"), "{rich}");
    assert_eq!(lines[1], "  note: Lands with M2. [jone, until 2026-12-31]");
}

#[test]
fn note_on_a_suppressed_finding_stays_out_of_plain() {
    let mut issue = noted_vacancy(false, None);
    issue.suppressed = true;
    assert!(!output_result(&vec![issue], Format::Plain).contains("note"));
}

// Requirement: trace-report/Trace findings carry their note

#[test]
fn trace_note_on_an_entry_finding() {
    let profile = profile_from(ORDER_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [node("UN-1", "need", 1)],
    }));
    let mut attached = noted_vacancy(false, None);
    attached.origin = Provenance::new("r.md", 1).into();
    let report = build_trace_report(&profile, &graph, vec![attached], "0.0.0");
    let parsed: Value = serde_json::from_str(&output_result(&report, Format::Json)).unwrap();
    assert_eq!(
        parsed["entries"][0]["findings"][0]["note"]["text"],
        "Lands with M2."
    );
    let plain = output_result(&report, Format::Plain);
    assert!(plain.contains("| note: Lands with M2."), "{plain}");
}

// Requirement: graph-service/Supported service shapes admit additions
// The validate report wraps its findings, but renders as the bare list.
#[test]
fn validate_report_renders_its_findings_and_scope() {
    use latt_core::service::{InputContext, Report, Request, View, execute};
    let profile = profile_from(ORDER_PROFILE).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.2",
        "nodes": [node("REQ-1", "req", 1), node("UN-1", "need", 2)],
        "edges": [edge("REQ-1", "UN-9", "verifies", 3)],
        "pathways": [], "findings": []
    }));
    let context = InputContext::default();
    let view = View {
        graph: &graph,
        profile: &profile,
        context: &context,
    };
    for strict in [false, true] {
        let response = execute(view, Request::Validate { strict }).unwrap();
        let Report::Validate(report) = &response.payload else {
            panic!("validate answers with a validate report");
        };
        let findings = validate(&graph, &profile, strict);
        assert!(!findings.is_empty());
        assert_eq!(report.findings, findings);
        for &format in Format::ALL {
            let bare = output_result(&findings, format);
            let rendered = output_result(&response.payload, format);
            assert_eq!(output_result(report, format), rendered);
            if format == Format::Json {
                // The findings as the bare list renders them, and the scope beside.
                let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();
                let bare: serde_json::Value = serde_json::from_str(&bare).unwrap();
                assert_eq!(rendered["findings"], bare["findings"]);
                assert_eq!(rendered["scope"]["nodes"]["req"], 1);
                assert_eq!(rendered["scope"]["nodes"]["need"], 1);
                assert_eq!(rendered["scope"]["edges"]["verifies"], 1);
                assert_eq!(rendered["scope"]["files"], 1);
            } else {
                assert_eq!(rendered, bare);
            }
        }
    }
}

// Requirement: graph-service/Supported service shapes admit additions
#[test]
fn format_parses_from_its_spelling_with_a_typed_error() {
    assert_eq!("json".parse::<Format>(), Ok(Format::Json));
    let error: ParseFormatError = "xml".parse::<Format>().unwrap_err();
    assert_eq!(error.value(), "xml");
    assert_eq!(
        error.to_string(),
        "unknown format 'xml': expected plain, json or rich"
    );
    for &format in Format::ALL {
        assert_eq!(format.as_str().parse::<Format>(), Ok(format));
    }
}
