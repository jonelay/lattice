//! The `validation` capability's scenarios, against the Rust validator.
//!
//! Written from `openspec/specs/validation/spec.md`. A handful of these overlap
//! with `pinned_behaviours.rs`, which asks a different question — that file pins
//! observed behaviour on paths the real-data gate never exercises, this one holds
//! the core to what the contract says. A case can pass one and fail the other.

mod common;

use common::{ingest, profile_from};
use lattice_core::graph::LatticeGraph;
use lattice_core::profile::Profile;
use lattice_core::types::{Issue, Provenance, Severity};
use lattice_core::validate::{resolve_adapter_issues, validate};
use serde_json::{Value, json};

/// A profile with the kinds the scenarios name: `need`, `req`, `test`.
const KINDS_PROFILE: &str = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  need:
    id_pattern: "^NEED-\\d+$"
  req:
    id_pattern: "^REQ-\\d{4}$"
    attrs:
      text: {type: string, required: true}
      status: {type: enum, values: [done, partial, todo, blocked]}
      tags: {type: list, items: string}
      count: {type: int}
  test:
    id_pattern: "^T-\\d+$"
edge_kinds:
  verifies:
    allowed: [[test, req]]
  fulfills:
    allowed: [[req, need]]
  derives:
    allowed: [[req, req]]
"#;

fn kinds_profile() -> Profile {
    profile_from(KINDS_PROFILE).expect("the scenarios' profile loads")
}

/// A `req` node carrying the given attrs, complete enough not to trip other checks.
fn req(id: &str, attrs: Value) -> Value {
    json!({"id": id, "kind": "req", "attrs": attrs,
           "provenance": {"file": "REQS.md", "line": 42}})
}

fn codes(issues: &[Issue]) -> Vec<&str> {
    issues.iter().map(|i| i.code.as_str()).collect()
}

fn find<'a>(issues: &'a [Issue], code: &str) -> &'a Issue {
    issues
        .iter()
        .find(|i| i.code == code)
        .unwrap_or_else(|| panic!("expected a {code} finding, got {:?}", codes(issues)))
}

// Requirement: Issue model

#[test]
fn an_issue_carries_severity_code_provenance_and_message() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    let orphan = find(&issues, "ORPHAN_NODE");
    assert_eq!(orphan.severity, Severity::Warning);
    assert_eq!(orphan.provenance, Provenance::new("REQS.md", 42));
    assert!(orphan.message.contains("REQ-0001"));
}

#[test]
fn a_node_scoped_finding_carries_its_node_id() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-1", json!({"text": "t"}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert_eq!(find(&issues, "ID_FORMAT").node_id.as_deref(), Some("REQ-1"));
}

// Requirement: Built-in validators

#[test]
fn a_coverage_gap_names_the_uncovered_node() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  {"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}}],
        "edges": [{"src": "T-1", "tgt": "REQ-0002", "kind": "verifies",
                   "provenance": {"file": "t.py", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);

    let coverage = find(&issues, "COVERAGE");
    assert_eq!(coverage.severity, Severity::Warning);
    assert!(
        coverage.message.contains("REQ-0001"),
        "{}",
        coverage.message
    );
}

#[test]
fn two_coverage_rules_report_only_the_edge_kind_that_is_missing() {
    let profile = profile_from(&coverage_profile(&["verifies", "fulfills"])).unwrap();
    let issues = validate(&covered_by("verifies"), &profile, false);

    let coverage = coverage_for(&issues, "REQ-0001");
    assert_eq!(coverage.len(), 1);
    assert!(
        coverage[0].message.contains("fulfills"),
        "{}",
        coverage[0].message
    );
}

#[test]
fn each_coverage_rule_is_checked_independently() {
    let profile = profile_from(&coverage_profile(&["verifies", "fulfills"])).unwrap();
    let issues = validate(&covered_by("derives"), &profile, false);

    let coverage = coverage_for(&issues, "REQ-0001");
    assert_eq!(coverage.len(), 2, "one per configured edge_kind");
    assert!(coverage.iter().any(|i| i.message.contains("verifies")));
    assert!(coverage.iter().any(|i| i.message.contains("fulfills")));
}

#[test]
fn a_coverage_config_naming_an_undeclared_kind_checks_nothing() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n  - COVERAGE:\n      target_kind: nosuch\n\
         \x20     edge_kind: verifies\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let issues = validate(&covered_by("derives"), &profile, false);

    assert!(find(&issues, "CONFIG_ERROR").message.contains("nosuch"));
    assert!(!issues.iter().any(|i| i.code == "COVERAGE"));
}

#[test]
fn one_bad_coverage_config_does_not_suppress_the_next() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n\
         \x20 - COVERAGE:\n      target_kind: nosuch\n      edge_kind: verifies\n\
         \x20 - COVERAGE:\n      target_kind: req\n      edge_kind: fulfills\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let issues = validate(&covered_by("derives"), &profile, false);

    assert!(issues.iter().any(|i| i.code == "CONFIG_ERROR"));
    let coverage = coverage_for(&issues, "REQ-0001");
    assert_eq!(coverage.len(), 1);
    assert!(coverage[0].message.contains("fulfills"));
}

/// The `COVERAGE` findings raised against one node. `covered_by` declares a
/// second `req` node for the edge to come from, and it draws its own findings.
fn coverage_for<'a>(issues: &'a [Issue], node_id: &str) -> Vec<&'a Issue> {
    issues
        .iter()
        .filter(|i| i.code == "COVERAGE" && i.node_id.as_deref() == Some(node_id))
        .collect()
}

/// `KINDS_PROFILE` plus one `COVERAGE` entry per named edge kind, all on `req`.
fn coverage_profile(edge_kinds: &[&str]) -> String {
    let mut yaml = format!("{KINDS_PROFILE}validations:\n");
    for edge_kind in edge_kinds {
        yaml.push_str(&format!(
            "  - COVERAGE:\n      target_kind: req\n      edge_kind: {edge_kind}\n"
        ));
    }
    yaml
}

/// `REQ-0001` with one incoming edge of the named kind, so exactly one coverage
/// rule over `verifies`/`fulfills` is satisfied.
fn covered_by(edge_kind: &str) -> LatticeGraph {
    ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"})),
                  {"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}}],
        "edges": [{"src": if edge_kind == "verifies" { "T-1" } else { "REQ-0002" },
                   "tgt": "REQ-0001", "kind": edge_kind,
                   "provenance": {"file": "t.py", "line": 1}}],
    }))
}

#[test]
fn an_id_not_matching_its_pattern_is_reported_with_the_node_provenance() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-1", json!({"text": "t"}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    let id_format = find(&issues, "ID_FORMAT");
    assert_eq!(id_format.provenance, Provenance::new("REQS.md", 42));
    assert_eq!(id_format.severity, Severity::Error);
}

#[test]
fn an_edge_between_disallowed_kinds_is_attributed_to_its_source() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "NEED-1", "kind": "need", "attrs": {},
                   "provenance": {"file": "n.md", "line": 1}},
                  req("REQ-0001", json!({"text": "t"}))],
        "edges": [{"src": "NEED-1", "tgt": "REQ-0001", "kind": "verifies",
                   "provenance": {"file": "n.md", "line": 1}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    let constraint = find(&issues, "EDGE_CONSTRAINT");
    assert_eq!(constraint.node_id.as_deref(), Some("NEED-1"));
    assert_eq!(constraint.severity, Severity::Error);
}

#[test]
fn a_dangling_reference_is_attributed_to_its_source() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0604", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0604", "tgt": "RISK-001", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 7}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert_eq!(
        find(&issues, "DANGLING_REF").node_id.as_deref(),
        Some("REQ-0604")
    );
}

#[test]
fn a_missing_required_attr_is_reported() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    let required = find(&issues, "ATTR_REQUIRED");
    assert!(required.message.contains("text"), "{}", required.message);
}

#[test]
fn an_attr_of_the_wrong_type_is_reported() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "count": "seven"}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "ATTR_TYPE").message.contains("count"));
}

#[test]
fn an_enum_value_outside_the_declared_list_is_reported() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "status": "unknown"}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "ATTR_ENUM").message.contains("unknown"));
}

#[test]
fn a_list_element_of_the_wrong_type_is_reported() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "tags": ["a", 42]}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    // The finding names the element's position and its declared type, not the
    // value: `42` is what the register holds, and quoting it adds nothing.
    let items = find(&issues, "ATTR_LIST_ITEMS");
    assert!(items.message.contains("element 1"), "{}", items.message);
    assert!(
        items.message.contains("expected type 'string'"),
        "{}",
        items.message
    );
}

#[test]
fn the_axis_codes_take_a_profile_override() {
    // AXIS_INVALID is adapter-emitted, so the override is the only thing core
    // does to it. A code with no shipped default must still be overridable.
    let yaml = format!("{KINDS_PROFILE}validations:\n  - AXIS_INVALID:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "issues": [{"severity": "warning", "code": "AXIS_INVALID", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "AXIS_INVALID").severity,
        Severity::Info
    );
}

// Requirement: Strict mode

#[test]
fn strict_promotes_warnings_but_not_infos() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "issues": [{"severity": "info", "code": "NOTE", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    let profile = kinds_profile();

    let relaxed = validate(&graph, &profile, false);
    assert_eq!(find(&relaxed, "ORPHAN_NODE").severity, Severity::Warning);
    assert!(!relaxed.iter().any(|i| i.severity == Severity::Error));

    let strict = validate(&graph, &profile, true);
    assert_eq!(find(&strict, "ORPHAN_NODE").severity, Severity::Error);
    assert_eq!(find(&strict, "NOTE").severity, Severity::Info);
}

#[test]
fn strict_does_not_promote_an_axis_demoted_finding() {
    let graph = axis_graph("M4".into());
    let issues = validate(&graph, &axis_profile(), true);

    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Info
    );
    // The demotion is the whole point: nothing here makes the run fail.
    assert!(
        !issues.iter().any(|i| i.severity == Severity::Error),
        "{:?}",
        issues
    );
}

// Requirement: Severity override in profile

#[test]
fn a_profile_override_applies_to_a_built_in_code() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - ORPHAN_NODE:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "ORPHAN_NODE").severity,
        Severity::Info
    );
}

#[test]
fn a_profile_override_applies_to_an_adapter_code() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - PARSE_ERROR:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "issues": [{"severity": "warning", "code": "PARSE_ERROR", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 12}, "node_id": null}],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "PARSE_ERROR").severity,
        Severity::Info
    );
}

/// An issue arriving at hint from outside core cannot be promoted. Core has no
/// shipped default for the code, so the pre-pass cannot see it coming — only the
/// severity it actually arrives at says it is advice.
#[test]
fn an_override_promoting_an_externally_emitted_hint_is_a_config_error() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - ADAPTER_ADVICE:\n      severity: error\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.1",
        "issues": [{"severity": "hint", "code": "ADAPTER_ADVICE", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 12}, "node_id": null}],
    }));
    let issues = validate(&graph, &profile, false);

    assert_eq!(find(&issues, "ADAPTER_ADVICE").severity, Severity::Hint);
    let config_error = find(&issues, "CONFIG_ERROR");
    assert!(
        config_error.message.contains("ADAPTER_ADVICE"),
        "{}",
        config_error.message
    );
}

#[test]
fn a_demoting_override_on_an_externally_emitted_hint_is_silent() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - ADAPTER_ADVICE:\n      severity: hint\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.1",
        "issues": [{"severity": "hint", "code": "ADAPTER_ADVICE", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 12}, "node_id": null}],
    }));
    let issues = validate(&graph, &profile, false);

    assert_eq!(find(&issues, "ADAPTER_ADVICE").severity, Severity::Hint);
    assert!(!codes(&issues).contains(&"CONFIG_ERROR"), "{issues:?}");
}

/// One CONFIG_ERROR per offending code, however many findings arrive under it:
/// the fault is the override, and repeating it per finding would bury the
/// findings it reports about.
#[test]
fn a_refused_override_promotion_is_reported_once_per_code() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - ADAPTER_ADVICE:\n      severity: error\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.1",
        "issues": [{"severity": "hint", "code": "ADAPTER_ADVICE", "message": "one",
                    "provenance": {"file": "REQS.md", "line": 12}, "node_id": null},
                   {"severity": "hint", "code": "ADAPTER_ADVICE", "message": "two",
                    "provenance": {"file": "REQS.md", "line": 13}, "node_id": null}],
    }));
    let issues = validate(&graph, &profile, false);

    assert_eq!(
        issues.iter().filter(|i| i.code == "CONFIG_ERROR").count(),
        1,
        "{issues:?}"
    );
}

// Requirement: Adapter issue channel

#[test]
fn an_adapter_issue_appears_beside_the_graph_findings() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "bad row",
                    "provenance": {"file": "REQS.md", "line": 12}, "node_id": null}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    assert!(codes(&issues).contains(&"PARSE_ERROR"));
    assert!(codes(&issues).contains(&"ORPHAN_NODE"));
    assert_eq!(
        find(&issues, "PARSE_ERROR").provenance,
        Provenance::new("REQS.md", 12)
    );
}

// Requirement: Suppress findings cascading from an unknown kind

#[test]
fn an_unknown_kind_does_not_cascade_into_edge_constraints() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "M-1", "kind": "mystery", "attrs": {},
                   "provenance": {"file": "m.md", "line": 1}},
                  req("REQ-0001", json!({"text": "t"}))],
        "edges": [{"src": "M-1", "tgt": "REQ-0001", "kind": "verifies",
                   "provenance": {"file": "m.md", "line": 1}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    assert_eq!(
        find(&issues, "UNKNOWN_KIND").node_id.as_deref(),
        Some("M-1")
    );
    assert!(
        !issues.iter().any(|i| i.code == "EDGE_CONSTRAINT"),
        "{:?}",
        codes(&issues)
    );
}

// Requirement: Axis severity resolution

/// `KINDS_PROFILE` with a `phase` axis bound to the adapter code
/// `OBLIGATION_UNBACKED` through each `req` node's `trigger` attr.
fn axis_profile() -> Profile {
    let yaml = format!(
        "{KINDS_PROFILE}axes: [phase]\nvalidations:\n  - OBLIGATION_UNBACKED:\n\
         \x20     axis: phase\n      position_attr: trigger\n"
    );
    profile_from(&yaml).expect("the axis profile loads")
}

/// One `req` node whose `trigger` is as given, carrying one bound finding, on an
/// axis whose current position is `M0`.
///
/// A second node and an edge join it, so the bound finding is the graph's only
/// one — otherwise an incidental `ORPHAN_NODE` would answer the strict case
/// instead of the demotion under test.
fn axis_graph(trigger: Value) -> LatticeGraph {
    let mut attrs = json!({"text": "t"});
    if !trigger.is_null() {
        attrs["trigger"] = trigger;
    }
    ingest(json!({
        "contract_version": "1.0",
        "axes": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "nodes": [req("REQ-0001", attrs), req("REQ-0002", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0001", "tgt": "REQ-0002", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 42}}],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED",
                    "message": "m", "provenance": {"file": "REQS.md", "line": 42},
                    "node_id": "REQ-0001"}],
    }))
}

#[test]
fn a_finding_at_or_before_the_current_position_keeps_its_severity() {
    for position in ["M0", "CB"] {
        let issues = validate(&axis_graph(position.into()), &axis_profile(), false);
        assert_eq!(
            find(&issues, "OBLIGATION_UNBACKED").severity,
            Severity::Warning,
            "position {position} is due"
        );
    }
}

#[test]
fn a_finding_after_the_current_position_is_demoted() {
    let issues = validate(&axis_graph("M4".into()), &axis_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Info
    );
}

#[test]
fn a_position_value_not_on_the_axis_is_demoted() {
    let position = "subscribe DbD (precedes M0 wiring)";
    let issues = validate(&axis_graph(position.into()), &axis_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Info
    );
}

#[test]
fn a_non_string_position_leaves_severity_unchanged() {
    // Demotion is a positive claim that a finding is not yet due. A value the
    // axis cannot hold has proven nothing, so quieting it would be a drop.
    for position in [json!(0), json!(["M4"]), json!({"phase": "M4"})] {
        let issues = validate(&axis_graph(position.clone()), &axis_profile(), false);
        assert_eq!(
            find(&issues, "OBLIGATION_UNBACKED").severity,
            Severity::Warning,
            "position {position} is not resolvable"
        );
    }
}

#[test]
fn an_absent_position_attr_leaves_severity_unchanged() {
    let issues = validate(&axis_graph(Value::Null), &axis_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Warning
    );
}

#[test]
fn an_unbound_code_is_untouched_by_the_axis_pass() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "axes": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        // Not yet due on the axis, so a bound finding here would be demoted.
        "nodes": [req("REQ-0001", json!({"text": "t", "trigger": "M4"}))],
    }));
    let issues = validate(&graph, &axis_profile(), false);
    assert_eq!(find(&issues, "ORPHAN_NODE").severity, Severity::Warning);
}

// Requirement: Findings the axis pass cannot resolve

#[test]
fn a_bound_finding_without_a_node_id_keeps_its_severity() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "axes": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 42}, "node_id": null}],
    }));
    let issues = validate(&graph, &axis_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Warning
    );
}

#[test]
fn a_binding_the_graph_cannot_satisfy_reports_once_against_the_profile() {
    // Same document as the axis cases but with no `axes` array at all.
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "trigger": "M4"}))],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 42}, "node_id": "REQ-0001"},
                   {"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m2",
                    "provenance": {"file": "REQS.md", "line": 43}, "node_id": "REQ-0001"}],
    }));
    let issues = validate(&graph, &axis_profile(), false);

    let unresolved: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.code == "AXIS_UNRESOLVED")
        .collect();
    assert_eq!(unresolved.len(), 1, "one per bound code, not per finding");
    assert_eq!(unresolved[0].provenance, Provenance::new("<profile>", 0));
    assert!(unresolved[0].message.contains("OBLIGATION_UNBACKED"));
    // Nothing was demoted: the pairing is the finding, not the register.
    assert!(
        issues
            .iter()
            .filter(|i| i.code == "OBLIGATION_UNBACKED")
            .all(|i| i.severity == Severity::Warning)
    );
}

#[test]
fn two_codes_bound_to_one_missing_axis_each_report() {
    let yaml = format!(
        "{KINDS_PROFILE}axes: [phase]\nvalidations:\n\
         \x20 - OBLIGATION_UNBACKED:\n      axis: phase\n      position_attr: trigger\n\
         \x20 - ORPHAN_NODE:\n      axis: phase\n      position_attr: trigger\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 42}, "node_id": "REQ-0001"}],
    }));
    let issues = validate(&graph, &profile, false);

    let unresolved: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.code == "AXIS_UNRESOLVED")
        .collect();
    assert_eq!(unresolved.len(), 2);
    assert!(
        unresolved
            .iter()
            .any(|i| i.message.contains("OBLIGATION_UNBACKED"))
    );
    assert!(unresolved.iter().any(|i| i.message.contains("ORPHAN_NODE")));
}

// Requirement: Severity resolution is shared by every command

#[test]
fn validate_and_the_adapter_issue_channel_agree_on_a_demoted_severity() {
    let graph = axis_graph("M4".into());
    let profile = axis_profile();

    let from_validate = find(&validate(&graph, &profile, false), "OBLIGATION_UNBACKED").severity;
    let adapter_issues = resolve_adapter_issues(&graph, &profile);
    let from_summary = find(&adapter_issues, "OBLIGATION_UNBACKED").severity;

    assert_eq!(from_validate, Severity::Info);
    assert_eq!(from_validate, from_summary);
}

// Checks that must stay silent, and configuration faults the scenarios above do
// not reach. A validator that fires on valid input is as wrong as one that
// misses, and neither shows up in a test that only counts findings.

// Requirement: Built-in validators
// Requirement: Orphan detection

#[test]
fn a_well_formed_node_produces_no_finding_but_the_orphan_one() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "status": "done",
                                         "tags": ["a"], "count": 3}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert_eq!(codes(&issues), ["ORPHAN_NODE"], "{issues:?}");
}

#[test]
fn a_connected_node_is_not_an_orphan() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0001", "tgt": "REQ-0002", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 1}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(
        !issues.iter().any(|i| i.code == "ORPHAN_NODE"),
        "{issues:?}"
    );
}

// Requirement: Built-in validators

#[test]
fn an_allowed_endpoint_pair_produces_no_constraint_finding() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}},
                  req("REQ-0001", json!({"text": "t"}))],
        "edges": [{"src": "T-1", "tgt": "REQ-0001", "kind": "verifies",
                   "provenance": {"file": "t.py", "line": 1}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(
        !issues.iter().any(|i| i.code == "EDGE_CONSTRAINT"),
        "{issues:?}"
    );
}

#[test]
fn an_edge_of_an_undeclared_kind_is_reported() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0001", "tgt": "REQ-0002", "kind": "invents",
                   "provenance": {"file": "REQS.md", "line": 1}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "UNKNOWN_KIND").message.contains("invents"));
}

#[test]
fn a_boolean_is_not_accepted_where_an_int_is_declared() {
    // Python's bool is an int; conflating the two would let `count: true` pass.
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "count": true}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "ATTR_TYPE").message.contains("count"));
}

#[test]
fn a_coverage_config_naming_an_undeclared_edge_kind_is_a_config_error() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n  - COVERAGE:\n      target_kind: req\n\
         \x20     edge_kind: nosuch\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let issues = validate(&covered_by("derives"), &profile, false);
    assert!(find(&issues, "CONFIG_ERROR").message.contains("nosuch"));
    assert!(!issues.iter().any(|i| i.code == "COVERAGE"));
}

#[test]
fn a_coverage_config_missing_a_key_is_a_config_error() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - COVERAGE:\n      edge_kind: derives\n");
    let profile = profile_from(&yaml).unwrap();
    let issues = validate(&covered_by("derives"), &profile, false);
    assert!(
        issues.iter().any(|i| i.code == "CONFIG_ERROR"),
        "{issues:?}"
    );
}

// Requirement: Coverage check

#[test]
fn a_dangling_edge_source_is_not_counted_as_coverage() {
    // The edge exists, but nothing declared its source. Counting it would report
    // a requirement as verified by a test the register never declares.
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "edges": [{"src": "T-9", "tgt": "REQ-0001", "kind": "verifies",
                   "provenance": {"file": "t.py", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);
    assert_eq!(coverage_for(&issues, "REQ-0001").len(), 1);
}

// Requirement: Deterministic output ordering

/// Findings as JSON, which is the form the ordering contract is stated over.
fn render(issues: &[Issue]) -> String {
    lattice_core::output::output_result(issues, "json").expect("json is a known format")
}

#[test]
fn findings_do_not_depend_on_the_order_the_register_was_written_in() {
    let one = json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-1", json!({})), req("REQ-0002", json!({"text": "t"}))],
    });
    let two = json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0002", json!({"text": "t"})), req("REQ-1", json!({}))],
    });
    let profile = kinds_profile();
    assert_eq!(
        render(&validate(&ingest(one), &profile, false)),
        render(&validate(&ingest(two), &profile, false))
    );
}

// Requirement: Findings the axis pass cannot resolve

#[test]
fn a_bound_finding_naming_a_node_the_graph_lacks_keeps_its_severity() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "axes": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 42}, "node_id": "REQ-9999"}],
    }));
    let issues = validate(&graph, &axis_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Warning
    );
}

// Requirement: Axis severity resolution

#[test]
fn axis_resolution_does_not_depend_on_collection_order() {
    let issues = |first_line: i64, second_line: i64| {
        let graph = ingest(json!({
            "contract_version": "1.0",
            "axes": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
            "nodes": [req("REQ-0001", json!({"text": "t", "trigger": "M4"})),
                      req("REQ-0002", json!({"text": "t", "trigger": "M0"}))],
            "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "a",
                        "provenance": {"file": "r.md", "line": first_line},
                        "node_id": "REQ-0001"},
                       {"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "b",
                        "provenance": {"file": "r.md", "line": second_line},
                        "node_id": "REQ-0002"}],
        }));
        let resolved = validate(&graph, &axis_profile(), false);
        let mut by_message: Vec<(String, Severity)> = resolved
            .iter()
            .filter(|i| i.code == "OBLIGATION_UNBACKED")
            .map(|i| (i.message.clone(), i.severity))
            .collect();
        by_message.sort();
        by_message
    };
    assert_eq!(issues(1, 2), issues(2, 1));
    assert_eq!(issues(1, 2)[0].1, Severity::Info, "REQ-0001 is not yet due");
    assert_eq!(issues(1, 2)[1].1, Severity::Warning, "REQ-0002 is");
}

// Requirement: Strict mode

#[test]
fn strict_still_promotes_a_finding_that_is_due() {
    let issues = validate(&axis_graph("M0".into()), &axis_profile(), true);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Error
    );
}

// Requirement: Hint severity tier

#[test]
fn a_demotion_to_hint_is_honoured() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - ORPHAN_NODE:\n      severity: hint\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "ORPHAN_NODE").severity,
        Severity::Hint
    );
}

#[test]
fn strict_does_not_promote_a_hint_finding() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - ORPHAN_NODE:\n      severity: hint\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
    }));
    let issues = validate(&graph, &profile, true);

    assert_eq!(find(&issues, "ORPHAN_NODE").severity, Severity::Hint);
    assert!(
        !issues.iter().any(|i| i.severity == Severity::Error),
        "{:?}",
        issues
    );
}

// Requirement: Per-kind orphan exemption

/// The kinds profile with `test` exempted from ORPHAN_NODE.
fn orphan_ok_profile() -> Profile {
    let yaml = KINDS_PROFILE.replace(
        "  test:\n    id_pattern: \"^T-\\\\d+$\"",
        "  test:\n    id_pattern: \"^T-\\\\d+$\"\n    orphan_ok: true",
    );
    assert_ne!(yaml, KINDS_PROFILE, "the replacement must have applied");
    profile_from(&yaml).unwrap()
}

#[test]
fn an_orphan_ok_kind_raises_no_orphan_node() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}}],
    }));
    let issues = validate(&graph, &orphan_ok_profile(), false);
    assert!(
        !issues.iter().any(|i| i.code == "ORPHAN_NODE"),
        "{:?}",
        codes(&issues)
    );
}

#[test]
fn orphan_ok_on_one_kind_leaves_the_others_checked() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}},
                  req("REQ-0001", json!({"text": "t"}))],
    }));
    let issues = validate(&graph, &orphan_ok_profile(), false);
    let orphan = find(&issues, "ORPHAN_NODE");
    assert_eq!(orphan.node_id.as_deref(), Some("REQ-0001"));
}

// Requirement: Coverage evidence state

/// `REQ-0001` unverified; `T-1` attributed to `REQ-0002`, so no candidate is left.
fn fully_attributed() -> LatticeGraph {
    ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"})),
                  {"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}}],
        "edges": [{"src": "T-1", "tgt": "REQ-0002", "kind": "verifies",
                   "provenance": {"file": "t.py", "line": 1}}],
    }))
}

/// `REQ-0001` unverified beside an unattributed `T-1`.
fn with_unattributed_test() -> LatticeGraph {
    ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  {"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}}],
    }))
}

#[test]
fn coverage_state_is_unverified_when_every_candidate_is_attributed() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let issues = validate(&fully_attributed(), &profile, false);

    let coverage = coverage_for(&issues, "REQ-0001");
    assert_eq!(coverage[0].state.as_deref(), Some("unverified"));
    assert!(!issues.iter().any(|i| i.code == "COVERAGE_UNKNOWN"));
}

#[test]
fn coverage_state_is_unknown_when_an_unattributed_candidate_exists() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let issues = validate(&with_unattributed_test(), &profile, false);

    let coverage = coverage_for(&issues, "REQ-0001");
    assert_eq!(coverage[0].state.as_deref(), Some("unknown"));

    let unknown = find(&issues, "COVERAGE_UNKNOWN");
    assert_eq!(unknown.severity, Severity::Hint);
    assert!(unknown.message.contains('1'), "{}", unknown.message);
    assert!(unknown.message.contains("test"), "{}", unknown.message);
    assert!(unknown.message.contains("verifies"), "{}", unknown.message);
}

#[test]
fn coverage_state_never_invents_a_finding_for_a_verified_node() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  {"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}},
                  {"id": "T-2", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 9}}],
        "edges": [{"src": "T-1", "tgt": "REQ-0001", "kind": "verifies",
                   "provenance": {"file": "t.py", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);

    assert!(coverage_for(&issues, "REQ-0001").is_empty());
    // The unattributed population is still one fact worth one hint.
    assert_eq!(find(&issues, "COVERAGE_UNKNOWN").severity, Severity::Hint);
}

#[test]
fn coverage_state_reports_the_population_once_per_config() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"})),
                  req("REQ-0003", json!({"text": "t"})),
                  {"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}},
                  {"id": "T-2", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 2}},
                  {"id": "T-3", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 3}}],
    }));
    let issues = validate(&graph, &profile, false);

    let unknowns: Vec<_> = issues
        .iter()
        .filter(|i| i.code == "COVERAGE_UNKNOWN")
        .collect();
    assert_eq!(unknowns.len(), 1);
    assert!(unknowns[0].message.contains('3'), "{}", unknowns[0].message);
}

#[test]
fn coverage_state_hint_names_the_base_population() {
    // 1 of 3 tests unattributed: without the base, the hint reads as "the
    // test layer is not wired" — the misreading observed on live data.
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"})),
                  {"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}},
                  {"id": "T-2", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 2}},
                  {"id": "T-3", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 3}}],
        "edges": [{"src": "T-1", "tgt": "REQ-0002", "kind": "verifies",
                   "provenance": {"file": "t.py", "line": 1}},
                  {"src": "T-2", "tgt": "REQ-0002", "kind": "verifies",
                   "provenance": {"file": "t.py", "line": 2}}],
    }));
    let issues = validate(&graph, &profile, false);

    let unknown = find(&issues, "COVERAGE_UNKNOWN");
    assert!(unknown.message.contains("1 of 3"), "{}", unknown.message);
    assert!(unknown.message.contains("33.3%"), "{}", unknown.message);
}

// Requirement: Severity override in profile

#[test]
fn an_override_promoting_a_hint_default_code_is_a_config_error() {
    let mut yaml = coverage_profile(&["verifies"]);
    yaml.push_str("  - COVERAGE_UNKNOWN:\n      severity: warning\n");
    let profile = profile_from(&yaml).unwrap();
    let issues = validate(&with_unattributed_test(), &profile, false);

    assert_eq!(find(&issues, "COVERAGE_UNKNOWN").severity, Severity::Hint);
    let config_error = find(&issues, "CONFIG_ERROR");
    assert!(
        config_error.message.contains("COVERAGE_UNKNOWN"),
        "{}",
        config_error.message
    );
}
