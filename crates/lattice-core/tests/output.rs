//! The `output` and `trace-report` capabilities' scenarios.
//!
//! `trace_baseline.rs` pins the whole trace path to the Python core's bytes on
//! one real register. It cannot show that the *specified* order is what produced
//! them: phase-sweep happens to declare its nodes in an order the sort agrees
//! with. These cases construct the disagreements.

mod common;

use std::collections::BTreeMap;

use common::{ingest, profile_from};
use lattice_core::output::output_result;
use lattice_core::trace::build_trace_report;
use lattice_core::types::{
    Issue, PathReport, Provenance, Severity, StatusRollup, SummaryReport, TraceReport,
};
use lattice_core::validate::validate;
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

// Requirement: Trace entry ordering

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
    for format in ["plain", "json", "rich"] {
        assert_eq!(
            output_result(&trace_of(forwards.clone()), format).unwrap(),
            output_result(&trace_of(backwards.clone()), format).unwrap(),
            "{format} depends on the order the register was written in"
        );
    }
}

// Requirement: Trace report structure

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

    assert_eq!(report.unattachable_findings.len(), 1);
    assert_eq!(report.unattachable_findings[0].code, "PARSE_ERROR");

    // The footer section is what makes it visible in plain output.
    let plain = output_result(&report, "plain").unwrap();
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

    let json: Value = serde_json::from_str(&output_result(&report, "json").unwrap()).unwrap();
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
    let json: Value = serde_json::from_str(&output_result(&report, "json").unwrap()).unwrap();
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
    let rendered = output_result(&report, "json").unwrap();
    let json: Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(json["entries"][1]["edges"][0]["tgt"], "UN-1");
    assert_eq!(json["entries"][1]["edges"][0]["kind"], "derives");
    assert_eq!(json["entries"][1]["edges"][0]["attrs"]["confidence"], 0.75);
    assert_eq!(json["entries"][1]["edges"][0]["provenance"]["file"], "r.md");
    assert_eq!(json["entries"][1]["edges"][0]["provenance"]["line"], 9);
}

// Requirement: Trace report plain format

/// A profile with non-phase-sweep vocabulary proves the column is data-driven.
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

    let plain = output_result(&report, "plain").unwrap();
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
    let plain = output_result(&report, "plain").unwrap();
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

// Requirement: Key attr scalars render as JSON scalars

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
    let plain = output_result(&report, "plain").unwrap();
    assert!(
        plain.contains("42"),
        "int attr value appears in trace: {plain}"
    );
}

// Requirement: Three output formats

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
        output_result(&vec![issue], "plain").unwrap(),
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
    let rendered = output_result(&vec![issue], "json").unwrap();
    let parsed: Value = serde_json::from_str(&rendered).unwrap();
    let finding = &parsed["findings"][0];
    for key in ["severity", "code", "file", "line", "message", "node_id"] {
        assert!(finding.get(key).is_some(), "{key} missing from {rendered}");
    }
}

// Requirement: Output dispatcher

#[test]
fn an_unknown_format_is_rejected_for_every_payload() {
    let report = trace_of(json!({"interface_version": "1.0"}));
    assert!(output_result(&report, "yaml").is_err());
    assert!(output_result(&summary(), "yaml").is_err());
    assert!(output_result(&Vec::<Issue>::new(), "yaml").is_err());
}

// Requirement: Deterministic output ordering

/// Two groups over three status columns, one of which no node carries.
///
/// Every group carries every status key, which is how the rollup is built — a
/// declared status with no nodes is a zero column, not an absent one.
fn summary() -> SummaryReport {
    SummaryReport::Configured(StatusRollup {
        group_key: "file".to_string(),
        status_keys: ["blocked", "done", "todo"].map(String::from).to_vec(),
        groups: vec![
            (
                "a.md".to_string(),
                counts(&[("blocked", 0), ("done", 2), ("todo", 1), ("total", 3)]),
            ),
            (
                "b.md".to_string(),
                counts(&[("blocked", 0), ("done", 1), ("todo", 0), ("total", 1)]),
            ),
        ],
    })
}

fn counts(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

#[test]
fn the_rollup_renders_one_row_per_group_in_plain() {
    assert_eq!(
        output_result(&summary(), "plain").unwrap(),
        "a.md: blocked=0 done=2 todo=1 total=3\nb.md: blocked=0 done=1 todo=0 total=1"
    );
}

// Requirement: Summary subcommand

#[test]
fn the_rollup_carries_a_column_for_a_status_no_row_holds() {
    let rendered = output_result(&summary(), "json").unwrap();
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
    let rendered = output_result(&summary(), "rich").unwrap();
    let header = rendered.lines().next().unwrap();
    // Python's capitalize lowercases the tail, and the label is cut to five.
    assert!(header.contains("Block"), "{header}");
    assert!(header.contains("Total"), "{header}");
    assert!(header.starts_with("File "), "{header}");
    assert!(rendered.lines().last().unwrap().starts_with("TOTAL"));
}

// Requirement: Deterministic output ordering

#[test]
fn every_format_is_byte_stable_across_runs() {
    for format in ["plain", "json", "rich"] {
        assert_eq!(
            output_result(&summary(), format).unwrap(),
            output_result(&summary(), format).unwrap()
        );
    }
}

// Requirement: Hint severity rendering

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
        output_result(&vec![issue.clone()], "plain").unwrap(),
        "HINT COVERAGE_UNKNOWN <profile>:0 m"
    );
    let parsed: Value =
        serde_json::from_str(&output_result(&vec![issue.clone()], "json").unwrap()).unwrap();
    assert_eq!(parsed["findings"][0]["severity"], "hint");
    let rich = output_result(&vec![issue], "rich").unwrap();
    assert!(rich.contains("HINT"), "{rich}");
    assert!(rich.contains("1 hint(s)"), "{rich}");
}

// Requirement: Findings JSON carries state when present

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
    let parsed: Value =
        serde_json::from_str(&output_result(&vec![issue], "json").unwrap()).unwrap();
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
    let parsed: Value =
        serde_json::from_str(&output_result(&vec![issue], "json").unwrap()).unwrap();
    assert!(parsed["findings"][0].get("state").is_none());
}

// Requirement: Suggestion rendering

/// A suggestion is a finding like any other. It carries the source node's
/// provenance, so acting on it sends the reviewer to the line they would edit,
/// and it adds no field to the findings JSON — an optional, advisory overlay
/// must not widen a schema every consumer reads.
#[test]
fn a_suggestion_renders_as_a_hint_in_all_three_formats() {
    let issue = Issue::new(
        Severity::Hint,
        "SUGGESTED_EDGE",
        "'T-1' verifies 'REQ-0001' (score 0.8312, lattice-suggest 0.1.0): cosine 0.8312",
        Provenance::new("tests/test_torque.py", 42),
        Some("T-1".into()),
    );

    assert_eq!(
        output_result(&vec![issue.clone()], "plain").unwrap(),
        "HINT SUGGESTED_EDGE tests/test_torque.py:42 \
'T-1' verifies 'REQ-0001' (score 0.8312, lattice-suggest 0.1.0): cosine 0.8312"
    );

    let rich = output_result(&vec![issue.clone()], "rich").unwrap();
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
        serde_json::from_str(&output_result(&vec![issue], "json").unwrap()).unwrap();
    let other: Value =
        serde_json::from_str(&output_result(&vec![ordinary], "json").unwrap()).unwrap();

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

    let plain = output_result(&vec![issue], "plain").unwrap();
    assert!(
        plain.starts_with("HINT SUGGESTION_UNRESOLVED <suggestions>:0"),
        "{plain}"
    );
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

// Requirement: Suppressed finding rendering

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
        serde_json::from_str(&output_result(&vec![unsuppressed], "json").unwrap()).unwrap();
    let after: Value =
        serde_json::from_str(&output_result(&vec![suppressed_vacancy()], "json").unwrap()).unwrap();
    let entry = &after["findings"][0];
    assert_eq!(entry["suppressed"], true);
    for key in ["severity", "code", "file", "line", "message", "node_id"] {
        assert_eq!(entry[key], before["findings"][0][key], "{key}");
    }
    assert_eq!(entry["severity"], "error");
}

#[test]
fn an_unsuppressed_finding_carries_no_suppressed_key() {
    let rendered = output_result(&vec![unsuppressed_unreferenced()], "json").unwrap();
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
    let parsed: Value = serde_json::from_str(&output_result(&issues, "json").unwrap()).unwrap();
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
        output_result(&issues, "plain").unwrap(),
        "WARNING UNREFERENCED needs.md:8 node 'UN-2' has no incoming edges"
    );
    assert_eq!(
        output_result(&vec![suppressed_vacancy()], "plain").unwrap(),
        ""
    );
}

#[test]
fn rich_omits_a_suppressed_finding_and_counts_only_what_it_shows() {
    let issues = vec![suppressed_vacancy(), unsuppressed_unreferenced()];
    let rich = output_result(&issues, "rich").unwrap();
    assert!(rich.contains("UNREFERENCED"), "{rich}");
    assert!(!rich.contains("VACANCY"), "{rich}");
    assert!(rich.contains("1 warning(s)"), "{rich}");
    assert!(!rich.contains("error(s)"), "{rich}");
    assert_eq!(
        output_result(&vec![suppressed_vacancy()], "rich").unwrap(),
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
    assert_eq!(output_result(&vec![hint.clone()], "plain").unwrap(), "");
    assert_eq!(
        output_result(&vec![hint.clone()], "rich").unwrap(),
        "No findings."
    );
    let parsed: Value = serde_json::from_str(&output_result(&vec![hint], "json").unwrap()).unwrap();
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
    attached.provenance = Provenance::new("r.md", 1);
    let mut footer = unsuppressed_unreferenced();
    footer.node_id = None;
    footer.suppressed = true;
    let report = build_trace_report(&profile, &graph, vec![attached, footer], "0.0.0");

    let plain = output_result(&report, "plain").unwrap();
    assert!(!plain.contains("VACANCY"), "{plain}");
    assert!(!plain.contains("Unattachable"), "{plain}");
    assert!(plain.contains("findings:0"), "{plain}");
    let rich = output_result(&report, "rich").unwrap();
    assert!(rich.contains("no findings"), "{rich}");
    let parsed: Value = serde_json::from_str(&output_result(&report, "json").unwrap()).unwrap();
    let entry = parsed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "UN-1")
        .unwrap();
    assert_eq!(entry["findings"][0]["suppressed"], true);
    assert_eq!(parsed["unattachable_findings"][0]["suppressed"], true);
}
