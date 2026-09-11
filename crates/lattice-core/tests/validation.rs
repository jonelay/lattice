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
      milestones: {type: list, items: date}
      count: {type: int}
      expires: {type: date}
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
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    let unreferenced = find(&issues, "UNREFERENCED");
    assert_eq!(unreferenced.severity, Severity::Warning);
    assert_eq!(unreferenced.provenance, Provenance::new("REQS.md", 42));
    assert!(unreferenced.message.contains("REQ-0001"));
}

#[test]
fn a_node_scoped_finding_carries_its_node_id() {
    let graph = ingest(json!({
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
fn coverage_where_filters_only_matching_targets() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n  - COVERAGE:\n      target_kind: req\n\
         \x20     edge_kind: verifies\n      where:\n        status: {{not: deferred}}\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "status": "todo"})),
                  req("REQ-0002", json!({"text": "t", "status": "deferred"}))],
    }));
    let issues = validate(&graph, &profile, false);

    assert_eq!(coverage_for(&issues, "REQ-0001").len(), 1);
    assert!(coverage_for(&issues, "REQ-0002").is_empty());
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
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
fn a_vacancyerence_is_attributed_to_its_source() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0604", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0604", "tgt": "RISK-001", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 7}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert_eq!(
        find(&issues, "VACANCY").node_id.as_deref(),
        Some("REQ-0604")
    );
}

#[test]
fn a_missing_required_attr_is_reported() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    let required = find(&issues, "ATTR_REQUIRED");
    assert!(required.message.contains("text"), "{}", required.message);
}

#[test]
fn an_attr_of_the_wrong_type_is_reported() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "count": "seven"}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "ATTR_TYPE").message.contains("count"));
}

#[test]
fn an_enum_value_outside_the_declared_list_is_reported() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "status": "unknown"}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "ATTR_ENUM").message.contains("unknown"));
}

#[test]
fn a_list_element_of_the_wrong_type_is_reported() {
    let graph = ingest(json!({
        "interface_version": "1.0",
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
fn valid_date_attrs_and_list_items_pass() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({
            "text": "t",
            "expires": "2028-02-29",
            "milestones": ["2026-09-09", "2030-01-15"]
        }))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(
        !issues
            .iter()
            .any(|i| i.code == "ATTR_TYPE" || i.code == "ATTR_LIST_ITEMS"),
        "valid dates produced type findings: {:?}",
        codes(&issues)
    );
}

#[test]
fn invalid_date_strings_report_the_required_format() {
    for invalid in [
        "2030-02-29",
        "2030-13-01",
        "2030-1-15",
        "2030-01-15T00:00:00Z",
    ] {
        let graph = ingest(json!({
            "interface_version": "1.0",
            "nodes": [req("REQ-0001", json!({"text": "t", "expires": invalid}))],
        }));
        let issues = validate(&graph, &kinds_profile(), false);
        let attr = find(&issues, "ATTR_TYPE");
        assert!(
            attr.message.contains("date (YYYY-MM-DD)"),
            "{}",
            attr.message
        );
    }

    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({
            "text": "t", "milestones": ["2030-01-15", "2030-13-01"]
        }))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    let item = find(&issues, "ATTR_LIST_ITEMS");
    assert!(
        item.message.contains("date (YYYY-MM-DD)"),
        "{}",
        item.message
    );
}

#[test]
fn the_pathway_codes_take_a_profile_override() {
    // PATHWAY_INVALID is adapter-emitted, so the override is the only thing core
    // does to it. A code with no shipped default must still be overridable.
    let yaml = format!("{KINDS_PROFILE}validations:\n  - PATHWAY_INVALID:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "issues": [{"severity": "warning", "code": "PATHWAY_INVALID", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "PATHWAY_INVALID").severity,
        Severity::Info
    );
}

// Requirement: Strict mode

#[test]
fn strict_promotes_warnings_but_not_infos() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "issues": [{"severity": "info", "code": "NOTE", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    let profile = kinds_profile();

    let relaxed = validate(&graph, &profile, false);
    assert_eq!(find(&relaxed, "UNREFERENCED").severity, Severity::Warning);
    assert!(!relaxed.iter().any(|i| i.severity == Severity::Error));

    let strict = validate(&graph, &profile, true);
    assert_eq!(find(&strict, "UNREFERENCED").severity, Severity::Error);
    assert_eq!(find(&strict, "NOTE").severity, Severity::Info);
}

#[test]
fn strict_does_not_promote_a_pathway_demoted_finding() {
    let graph = pathway_graph("M4".into());
    let issues = validate(&graph, &pathway_profile(), true);

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
    let yaml = format!("{KINDS_PROFILE}validations:\n  - UNREFERENCED:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "UNREFERENCED").severity,
        Severity::Info
    );
}

#[test]
fn a_profile_override_applies_to_an_adapter_code() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - PARSE_ERROR:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
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
        "interface_version": "1.1",
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
        "interface_version": "1.1",
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
        "interface_version": "1.1",
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
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "bad row",
                    "provenance": {"file": "REQS.md", "line": 12}, "node_id": null}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    assert!(codes(&issues).contains(&"PARSE_ERROR"));
    assert!(codes(&issues).contains(&"UNREFERENCED"));
    assert_eq!(
        find(&issues, "PARSE_ERROR").provenance,
        Provenance::new("REQS.md", 12)
    );
}

// Requirement: Suppress findings cascading from an unknown kind

#[test]
fn an_unknown_kind_does_not_cascade_into_edge_constraints() {
    let graph = ingest(json!({
        "interface_version": "1.0",
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

// Requirement: Pathway severity resolution

/// `KINDS_PROFILE` with a `phase` pathway bound to the adapter code
/// `OBLIGATION_UNBACKED` through each `req` node's `trigger` attr.
fn pathway_profile() -> Profile {
    let yaml = format!(
        "{KINDS_PROFILE}pathways: [phase]\nvalidations:\n  - OBLIGATION_UNBACKED:\n\
         \x20     pathway: phase\n      position_attr: trigger\n"
    );
    profile_from(&yaml).expect("the pathway profile loads")
}

/// One `req` node whose `trigger` is as given, carrying one bound finding, on an
/// pathway whose current position is `M0`.
///
/// A second node and bidirectional edges join them, so the bound finding is the
/// graph's only one — otherwise an incidental `UNREFERENCED`/`UNTRACED` would
/// answer the strict case instead of the demotion under test.
fn pathway_graph(trigger: Value) -> LatticeGraph {
    let mut attrs = json!({"text": "t"});
    if !trigger.is_null() {
        attrs["trigger"] = trigger;
    }
    ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "nodes": [req("REQ-0001", attrs), req("REQ-0002", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0001", "tgt": "REQ-0002", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 42}},
                  {"src": "REQ-0002", "tgt": "REQ-0001", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 43}}],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED",
                    "message": "m", "provenance": {"file": "REQS.md", "line": 42},
                    "node_id": "REQ-0001"}],
    }))
}

#[test]
fn a_finding_at_or_before_the_current_position_keeps_its_severity() {
    for position in ["M0", "CB"] {
        let issues = validate(&pathway_graph(position.into()), &pathway_profile(), false);
        assert_eq!(
            find(&issues, "OBLIGATION_UNBACKED").severity,
            Severity::Warning,
            "position {position} is due"
        );
    }
}

#[test]
fn a_finding_after_the_current_position_is_demoted() {
    let issues = validate(&pathway_graph("M4".into()), &pathway_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Info
    );
}

#[test]
fn a_position_value_not_on_the_pathway_is_demoted() {
    let position = "subscribe DbD (precedes M0 wiring)";
    let issues = validate(&pathway_graph(position.into()), &pathway_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Info
    );
}

#[test]
fn a_non_string_position_leaves_severity_unchanged() {
    // Demotion is a positive claim that a finding is not yet due. A value the
    // pathway cannot hold has proven nothing, so quieting it would be a drop.
    for position in [json!(0), json!(["M4"]), json!({"phase": "M4"})] {
        let issues = validate(&pathway_graph(position.clone()), &pathway_profile(), false);
        assert_eq!(
            find(&issues, "OBLIGATION_UNBACKED").severity,
            Severity::Warning,
            "position {position} is not resolvable"
        );
    }
}

#[test]
fn an_absent_position_attr_leaves_severity_unchanged() {
    let issues = validate(&pathway_graph(Value::Null), &pathway_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Warning
    );
}

#[test]
fn an_unbound_code_is_untouched_by_the_pathway_pass() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        // Not yet due on the pathway, so a bound finding here would be demoted.
        "nodes": [req("REQ-0001", json!({"text": "t", "trigger": "M4"}))],
    }));
    let issues = validate(&graph, &pathway_profile(), false);
    assert_eq!(find(&issues, "UNREFERENCED").severity, Severity::Warning);
}

// Requirement: Findings the pathway pass cannot resolve

#[test]
fn a_bound_finding_without_a_node_id_keeps_its_severity() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 42}, "node_id": null}],
    }));
    let issues = validate(&graph, &pathway_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Warning
    );
}

#[test]
fn a_binding_the_graph_cannot_satisfy_reports_once_against_the_profile() {
    // Same document as the pathway cases but with no `pathways` array at all.
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "trigger": "M4"}))],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 42}, "node_id": "REQ-0001"},
                   {"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m2",
                    "provenance": {"file": "REQS.md", "line": 43}, "node_id": "REQ-0001"}],
    }));
    let issues = validate(&graph, &pathway_profile(), false);

    let unresolved: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.code == "PATHWAY_UNRESOLVED")
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
fn two_codes_bound_to_one_missing_pathway_each_report() {
    let yaml = format!(
        "{KINDS_PROFILE}pathways: [phase]\nvalidations:\n\
         \x20 - OBLIGATION_UNBACKED:\n      pathway: phase\n      position_attr: trigger\n\
         \x20 - UNREFERENCED:\n      pathway: phase\n      position_attr: trigger\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 42}, "node_id": "REQ-0001"}],
    }));
    let issues = validate(&graph, &profile, false);

    let unresolved: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.code == "PATHWAY_UNRESOLVED")
        .collect();
    assert_eq!(unresolved.len(), 2);
    assert!(
        unresolved
            .iter()
            .any(|i| i.message.contains("OBLIGATION_UNBACKED"))
    );
    assert!(
        unresolved
            .iter()
            .any(|i| i.message.contains("UNREFERENCED"))
    );
}

// Requirement: Severity resolution is shared by every command

#[test]
fn validate_and_the_adapter_issue_channel_agree_on_a_demoted_severity() {
    let graph = pathway_graph("M4".into());
    let profile = pathway_profile();

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
fn a_well_formed_node_produces_no_finding_but_the_directional_orphan_ones() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t", "status": "done",
                                         "tags": ["a"], "count": 3}))],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    let mut found = codes(&issues);
    found.sort();
    assert_eq!(found, ["UNREFERENCED", "UNTRACED"], "{issues:?}");
}

#[test]
fn a_fully_connected_node_gets_no_directional_orphan_finding() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0001", "tgt": "REQ-0002", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 1}},
                  {"src": "REQ-0002", "tgt": "REQ-0001", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 2}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(
        !issues
            .iter()
            .any(|i| i.code == "UNREFERENCED" || i.code == "UNTRACED"),
        "{issues:?}"
    );
}

#[test]
fn a_node_with_only_outgoing_edges_is_unreferenced() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0001", "tgt": "REQ-0002", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 1}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    let unreferenced: Vec<_> = issues.iter().filter(|i| i.code == "UNREFERENCED").collect();
    assert_eq!(unreferenced.len(), 1);
    assert_eq!(unreferenced[0].node_id.as_deref(), Some("REQ-0001"));
    assert!(
        !issues
            .iter()
            .any(|i| i.code == "UNTRACED" && i.node_id.as_deref() == Some("REQ-0001"))
    );
}

#[test]
fn a_node_with_only_incoming_edges_is_untraced() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"})),
                  req("REQ-0002", json!({"text": "t"}))],
        "edges": [{"src": "REQ-0001", "tgt": "REQ-0002", "kind": "derives",
                   "provenance": {"file": "REQS.md", "line": 1}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    let untraced: Vec<_> = issues.iter().filter(|i| i.code == "UNTRACED").collect();
    assert_eq!(untraced.len(), 1);
    assert_eq!(untraced[0].node_id.as_deref(), Some("REQ-0002"));
    assert!(
        !issues
            .iter()
            .any(|i| i.code == "UNREFERENCED" && i.node_id.as_deref() == Some("REQ-0002"))
    );
}

// Requirement: Built-in validators

#[test]
fn an_allowed_endpoint_pair_produces_no_constraint_finding() {
    let graph = ingest(json!({
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
        "interface_version": "1.0",
        "nodes": [req("REQ-1", json!({})), req("REQ-0002", json!({"text": "t"}))],
    });
    let two = json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0002", json!({"text": "t"})), req("REQ-1", json!({}))],
    });
    let profile = kinds_profile();
    assert_eq!(
        render(&validate(&ingest(one), &profile, false)),
        render(&validate(&ingest(two), &profile, false))
    );
}

// Requirement: Findings the pathway pass cannot resolve

#[test]
fn a_bound_finding_naming_a_node_the_graph_lacks_keeps_its_severity() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "m",
                    "provenance": {"file": "REQS.md", "line": 42}, "node_id": "REQ-9999"}],
    }));
    let issues = validate(&graph, &pathway_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Warning
    );
}

// Requirement: Pathway severity resolution

#[test]
fn pathway_resolution_does_not_depend_on_collection_order() {
    let issues = |first_line: i64, second_line: i64| {
        let graph = ingest(json!({
            "interface_version": "1.0",
            "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
            "nodes": [req("REQ-0001", json!({"text": "t", "trigger": "M4"})),
                      req("REQ-0002", json!({"text": "t", "trigger": "M0"}))],
            "issues": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "a",
                        "provenance": {"file": "r.md", "line": first_line},
                        "node_id": "REQ-0001"},
                       {"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "b",
                        "provenance": {"file": "r.md", "line": second_line},
                        "node_id": "REQ-0002"}],
        }));
        let resolved = validate(&graph, &pathway_profile(), false);
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
    let issues = validate(&pathway_graph("M0".into()), &pathway_profile(), true);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Error
    );
}

// Requirement: Hint severity tier

#[test]
fn a_demotion_to_hint_is_honoured() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - UNREFERENCED:\n      severity: hint\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "UNREFERENCED").severity,
        Severity::Hint
    );
}

#[test]
fn strict_does_not_promote_a_hint_finding() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n  - UNREFERENCED:\n      severity: hint\n  - UNTRACED:\n      severity: hint\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
    }));
    let issues = validate(&graph, &profile, true);

    assert_eq!(find(&issues, "UNREFERENCED").severity, Severity::Hint);
    assert_eq!(find(&issues, "UNTRACED").severity, Severity::Hint);
    assert!(
        !issues.iter().any(|i| i.severity == Severity::Error),
        "{:?}",
        issues
    );
}

// Requirement: Per-kind orphan exemption

/// The kinds profile with `test` exempted from orphan findings.
fn orphan_ok_profile() -> Profile {
    let yaml = KINDS_PROFILE.replace(
        "  test:\n    id_pattern: \"^T-\\\\d+$\"",
        "  test:\n    id_pattern: \"^T-\\\\d+$\"\n    orphan_ok: true",
    );
    assert_ne!(yaml, KINDS_PROFILE, "the replacement must have applied");
    profile_from(&yaml).unwrap()
}

#[test]
fn an_orphan_ok_kind_raises_no_directional_orphan_findings() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}}],
    }));
    let issues = validate(&graph, &orphan_ok_profile(), false);
    assert!(
        !issues
            .iter()
            .any(|i| i.code == "UNREFERENCED" || i.code == "UNTRACED"),
        "{:?}",
        codes(&issues)
    );
}

#[test]
fn orphan_ok_on_one_kind_leaves_the_others_checked() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "T-1", "kind": "test", "attrs": {},
                   "provenance": {"file": "t.py", "line": 1}},
                  req("REQ-0001", json!({"text": "t"}))],
    }));
    let issues = validate(&graph, &orphan_ok_profile(), false);
    let unreferenced = find(&issues, "UNREFERENCED");
    assert_eq!(unreferenced.node_id.as_deref(), Some("REQ-0001"));
    let untraced = find(&issues, "UNTRACED");
    assert_eq!(untraced.node_id.as_deref(), Some("REQ-0001"));
}

// Requirement: Coverage evidence state

/// `REQ-0001` unverified; `T-1` attributed to `REQ-0002`, so no candidate is left.
fn fully_attributed() -> LatticeGraph {
    ingest(json!({
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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
        "interface_version": "1.0",
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

// ── CONSTRAINT validation ──────────────────────────────────────────
// Requirement: CONSTRAINT evaluation
// Requirement: CONSTRAINT condition operators

fn constraint_profile(constraint_yaml: &str) -> String {
    format!("{KINDS_PROFILE}validations:\n  - CONSTRAINT:\n{constraint_yaml}")
}

fn constraint_graph(attrs: Value) -> LatticeGraph {
    ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-0001", "kind": "req", "attrs": attrs,
                   "provenance": {"file": "REQS.md", "line": 1}}],
    }))
}

#[test]
fn constraint_expect_passes_no_finding() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        text: {present: true}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello"}));
    let issues = validate(&graph, &profile, false);
    assert!(
        !issues.iter().any(|i| i.code == "CONSTRAINT"),
        "expected no CONSTRAINT finding, got {:?}",
        codes(&issues)
    );
}

#[test]
fn constraint_expect_fails() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        status: {present: true}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello"}));
    let issues = validate(&graph, &profile, false);
    let c = find(&issues, "CONSTRAINT");
    assert_eq!(c.node_id.as_deref(), Some("REQ-0001"));
    assert_eq!(c.severity, Severity::Warning);
}

#[test]
fn constraint_reject_fires() {
    let yaml =
        constraint_profile("      kind: req\n      reject:\n        status: {eq: \"blocked\"}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "blocked"}));
    let issues = validate(&graph, &profile, false);
    find(&issues, "CONSTRAINT");
}

#[test]
fn constraint_reject_passes() {
    let yaml =
        constraint_profile("      kind: req\n      reject:\n        status: {eq: \"blocked\"}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "done"}));
    let issues = validate(&graph, &profile, false);
    assert!(
        !issues.iter().any(|i| i.code == "CONSTRAINT"),
        "expected no CONSTRAINT, got {:?}",
        codes(&issues)
    );
}

#[test]
fn constraint_when_guard_skips() {
    let yaml = constraint_profile(
        "      kind: req\n      when:\n        status: {eq: \"approved\"}\n      expect:\n        count: {present: true}\n",
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "draft"}));
    let issues = validate(&graph, &profile, false);
    assert!(
        !issues.iter().any(|i| i.code == "CONSTRAINT"),
        "when guard should skip, got {:?}",
        codes(&issues)
    );
}

#[test]
fn constraint_when_passes_expect_fails() {
    let yaml = constraint_profile(
        "      kind: req\n      when:\n        status: {eq: \"done\"}\n      expect:\n        count: {present: true}\n",
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "done"}));
    let issues = validate(&graph, &profile, false);
    find(&issues, "CONSTRAINT");
}

#[test]
fn constraint_both_expect_and_reject() {
    let yaml = constraint_profile(
        "      kind: req\n      expect:\n        text: {present: true}\n      reject:\n        status: {eq: \"blocked\"}\n",
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "blocked"}));
    let issues = validate(&graph, &profile, false);
    find(&issues, "CONSTRAINT");
}

#[test]
fn constraint_custom_message() {
    let yaml = constraint_profile(
        "      kind: req\n      expect:\n        count: {present: true}\n      message: \"Requirements must have a count\"\n",
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello"}));
    let issues = validate(&graph, &profile, false);
    assert_eq!(
        find(&issues, "CONSTRAINT").message,
        "Requirements must have a count"
    );
}

#[test]
fn constraint_generated_message_names_attr() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        count: {present: true}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello"}));
    let issues = validate(&graph, &profile, false);
    let msg = &find(&issues, "CONSTRAINT").message;
    assert!(
        msg.contains("count"),
        "generated message should name 'count', got: {msg}"
    );
}

#[test]
fn constraint_kind_mismatch_skips() {
    let yaml =
        constraint_profile("      kind: test\n      expect:\n        status: {present: true}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello"}));
    let issues = validate(&graph, &profile, false);
    assert!(
        !issues.iter().any(|i| i.code == "CONSTRAINT"),
        "kind mismatch should skip"
    );
}

#[test]
fn constraint_multiple_entries() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n\
         \x20 - CONSTRAINT:\n      kind: req\n      expect:\n        count: {{present: true}}\n\
         \x20 - CONSTRAINT:\n      kind: req\n      reject:\n        status: {{eq: \"blocked\"}}\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "blocked"}));
    let issues = validate(&graph, &profile, false);
    let constraints: Vec<_> = issues.iter().filter(|i| i.code == "CONSTRAINT").collect();
    assert_eq!(constraints.len(), 2, "both CONSTRAINT entries should fire");
}

#[test]
fn constraint_op_eq() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        status: {eq: \"done\"}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "done"}));
    assert!(
        !validate(&graph, &profile, false)
            .iter()
            .any(|i| i.code == "CONSTRAINT")
    );

    let graph2 = constraint_graph(json!({"text": "hello", "status": "draft"}));
    find(&validate(&graph2, &profile, false), "CONSTRAINT");
}

#[test]
fn constraint_op_not() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        status: {not: \"blocked\"}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "done"}));
    assert!(
        !validate(&graph, &profile, false)
            .iter()
            .any(|i| i.code == "CONSTRAINT")
    );

    let graph2 = constraint_graph(json!({"text": "hello", "status": "blocked"}));
    find(&validate(&graph2, &profile, false), "CONSTRAINT");
}

#[test]
fn constraint_op_in() {
    let yaml = constraint_profile(
        "      kind: req\n      expect:\n        status: {in: [\"done\", \"partial\"]}\n",
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "status": "partial"}));
    assert!(
        !validate(&graph, &profile, false)
            .iter()
            .any(|i| i.code == "CONSTRAINT")
    );

    let graph2 = constraint_graph(json!({"text": "hello", "status": "blocked"}));
    find(&validate(&graph2, &profile, false), "CONSTRAINT");
}

#[test]
fn constraint_op_matches() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        text: {matches: \"^[A-Z]\"}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "Hello"}));
    assert!(
        !validate(&graph, &profile, false)
            .iter()
            .any(|i| i.code == "CONSTRAINT")
    );

    let graph2 = constraint_graph(json!({"text": "hello"}));
    find(&validate(&graph2, &profile, false), "CONSTRAINT");
}

#[test]
fn constraint_op_present_true() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        count: {present: true}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "count": 0}));
    assert!(
        !validate(&graph, &profile, false)
            .iter()
            .any(|i| i.code == "CONSTRAINT")
    );
}

#[test]
fn constraint_op_present_false() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        count: {present: false}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello"}));
    assert!(
        !validate(&graph, &profile, false)
            .iter()
            .any(|i| i.code == "CONSTRAINT")
    );

    let graph2 = constraint_graph(json!({"text": "hello", "count": 5}));
    find(&validate(&graph2, &profile, false), "CONSTRAINT");
}

#[test]
fn constraint_type_aware_comparison() {
    let yaml = constraint_profile("      kind: req\n      expect:\n        count: {eq: \"42\"}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello", "count": 42}));
    find(&validate(&graph, &profile, false), "CONSTRAINT");
}

#[test]
fn constraint_date_comparison_operators() {
    let cases = [
        ("lt", "2029-12-31", "2030-01-01", "2030-01-01"),
        ("gt", "2030-01-02", "2030-01-01", "2030-01-01"),
        ("lte", "2030-01-01", "2030-01-01", "2030-01-02"),
        ("gte", "2030-01-01", "2030-01-01", "2029-12-31"),
    ];
    for (op, passing, expected, failing) in cases {
        let yaml = constraint_profile(&format!(
            "      kind: req\n      expect:\n        expires: {{{op}: \"{expected}\"}}\n"
        ));
        let profile = profile_from(&yaml).unwrap();
        let graph = constraint_graph(json!({"text": "hello", "expires": passing}));
        assert!(
            !validate(&graph, &profile, false)
                .iter()
                .any(|i| i.code == "CONSTRAINT"),
            "date operator {op} should pass"
        );

        let graph = constraint_graph(json!({"text": "hello", "expires": failing}));
        find(&validate(&graph, &profile, false), "CONSTRAINT");
    }
}

#[test]
fn constraint_int_comparison_operators() {
    let cases = [
        ("lt", 4, 5, 5),
        ("gt", 6, 5, 5),
        ("lte", 5, 5, 6),
        ("gte", 5, 5, 4),
    ];
    for (op, passing, expected, failing) in cases {
        let yaml = constraint_profile(&format!(
            "      kind: req\n      expect:\n        count: {{{op}: {expected}}}\n"
        ));
        let profile = profile_from(&yaml).unwrap();
        let graph = constraint_graph(json!({"text": "hello", "count": passing}));
        assert!(
            !validate(&graph, &profile, false)
                .iter()
                .any(|i| i.code == "CONSTRAINT"),
            "int operator {op} should pass"
        );

        let graph = constraint_graph(json!({"text": "hello", "count": failing}));
        find(&validate(&graph, &profile, false), "CONSTRAINT");
    }
}

#[test]
fn constraint_comparison_fails_for_wrong_order_or_mixed_types() {
    let yaml = constraint_profile("      kind: req\n      expect:\n        count: {lt: 5}\n");
    let profile = profile_from(&yaml).unwrap();

    let wrong_order = constraint_graph(json!({"text": "hello", "count": 6}));
    find(&validate(&wrong_order, &profile, false), "CONSTRAINT");

    let mixed_types = constraint_graph(json!({"text": "hello", "count": "4"}));
    find(&validate(&mixed_types, &profile, false), "CONSTRAINT");
}

// Profile loader tests

#[test]
fn constraint_missing_kind_rejected() {
    let yaml = constraint_profile("      expect:\n        text: {present: true}\n");
    assert!(profile_from(&yaml).is_err());
}

#[test]
fn constraint_missing_expect_and_reject_rejected() {
    let yaml = constraint_profile("      kind: req\n");
    assert!(profile_from(&yaml).is_err());
}

#[test]
fn constraint_unknown_operator_rejected() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        count: {between: [1, 5]}\n");
    assert!(profile_from(&yaml).is_err());
}

#[test]
fn constraint_undeclared_kind_rejected() {
    let yaml =
        constraint_profile("      kind: widget\n      expect:\n        x: {present: true}\n");
    assert!(profile_from(&yaml).is_err());
}

#[test]
fn constraint_invalid_regex_rejected() {
    let yaml = constraint_profile(
        "      kind: req\n      expect:\n        text: {matches: \"[invalid\"}\n",
    );
    assert!(profile_from(&yaml).is_err());
}

#[test]
fn ordering_op_rejects_non_comparable_values() {
    for op in ["lt", "gt", "lte", "gte"] {
        let cases = [
            ("[1, 2]", "list"),
            ("2.5", "float"),
            ("true", "bool"),
            ("null", "null"),
            ("{a: 1}", "mapping"),
        ];
        for (value, label) in cases {
            let yaml = constraint_profile(&format!(
                "      kind: req\n      expect:\n        count: {{{op}: {value}}}\n"
            ));
            assert!(
                profile_from(&yaml).is_err(),
                "{op} should reject {label} value {value}"
            );
        }
    }
}

// Integration: pathway demotion and strict

#[test]
fn constraint_pathway_demotion() {
    let yaml = format!(
        "{KINDS_PROFILE}\
         pathways: [stage]\n\
         validations:\n\
         \x20 - CONSTRAINT:\n\
         \x20     kind: req\n\
         \x20     expect:\n\
         \x20       count: {{present: true}}\n\
         \x20     pathway: stage\n\
         \x20     position_attr: status\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let doc = json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-0001", "kind": "req",
                   "attrs": {"text": "hello", "status": "done"},
                   "provenance": {"file": "REQS.md", "line": 1}}],
        "pathways": [{"name": "stage", "order": ["todo", "done"], "current": "todo"}]
    });
    let graph = ingest(doc);
    let issues = validate(&graph, &profile, false);
    let c = issues.iter().find(|i| i.code == "CONSTRAINT");
    assert!(c.is_some(), "CONSTRAINT should fire");
    assert_eq!(
        c.unwrap().severity,
        Severity::Info,
        "should be demoted to info"
    );
}

#[test]
fn constraint_strict_promotion() {
    let yaml =
        constraint_profile("      kind: req\n      expect:\n        count: {present: true}\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = constraint_graph(json!({"text": "hello"}));
    let issues = validate(&graph, &profile, true);
    let c = find(&issues, "CONSTRAINT");
    assert_eq!(
        c.severity,
        Severity::Error,
        "strict should promote warning to error"
    );
}

// Requirement: Finding suppression

/// `need` nodes with `orphan_ok`, so the only findings are the VACANCYs the
/// dangling `derives` edges produce — one per source node, `node_id` = source.
const SUPPRESS_PROFILE: &str = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  need:
    id_pattern: "^UN-\\d+$"
    orphan_ok: true
edge_kinds:
  derives:
    allowed: [[need, need]]
"#;

fn suppress_profile(validations: &str) -> Profile {
    profile_from(&format!("{SUPPRESS_PROFILE}validations:\n{validations}"))
        .expect("the suppress scenarios' profile loads")
}

/// One VACANCY per listed node: each has a `derives` edge to a target that is
/// never declared.
fn vacancies_for(ids: &[&str]) -> LatticeGraph {
    let nodes: Vec<Value> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            json!({"id": id, "kind": "need", "attrs": {},
                   "provenance": {"file": "needs.md", "line": i + 1}})
        })
        .collect();
    let edges: Vec<Value> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            json!({"src": id, "tgt": "UN-999", "kind": "derives",
                   "provenance": {"file": "needs.md", "line": i + 1}})
        })
        .collect();
    ingest(json!({"interface_version": "1.0", "nodes": nodes, "edges": edges}))
}

fn suppressed_ids(issues: &[Issue], code: &str) -> Vec<String> {
    issues
        .iter()
        .filter(|i| i.code == code && i.suppressed)
        .map(|i| i.node_id.clone().unwrap_or_default())
        .collect()
}

#[test]
fn suppress_all_findings_of_a_code() {
    let profile = suppress_profile("  - SUPPRESS: {code: VACANCY}\n");
    let issues = validate(&vacancies_for(&["UN-1", "UN-2", "UN-3"]), &profile, false);
    let vacancies: Vec<&Issue> = issues.iter().filter(|i| i.code == "VACANCY").collect();
    assert_eq!(vacancies.len(), 3, "suppression reports, never drops");
    assert!(vacancies.iter().all(|i| i.suppressed));
    assert!(!codes(&issues).contains(&"SUPPRESS_UNUSED"));
}

#[test]
fn suppress_specific_node_ids() {
    let profile = suppress_profile("  - SUPPRESS: {code: VACANCY, node_ids: [UN-1, UN-2]}\n");
    let issues = validate(&vacancies_for(&["UN-1", "UN-2", "UN-3"]), &profile, false);
    assert_eq!(suppressed_ids(&issues, "VACANCY"), ["UN-1", "UN-2"]);
    let un3 = issues
        .iter()
        .find(|i| i.node_id.as_deref() == Some("UN-3"))
        .unwrap();
    assert!(!un3.suppressed);
    assert!(!codes(&issues).contains(&"SUPPRESS_UNUSED"));
}

#[test]
fn suppress_all_supersedes_an_id_specific_entry() {
    let profile = suppress_profile(
        "  - SUPPRESS: {code: VACANCY, node_ids: [UN-1]}\n  - SUPPRESS: {code: VACANCY}\n",
    );
    let issues = validate(&vacancies_for(&["UN-1", "UN-2"]), &profile, false);
    assert_eq!(suppressed_ids(&issues, "VACANCY"), ["UN-1", "UN-2"]);
    assert!(!codes(&issues).contains(&"SUPPRESS_UNUSED"));
}

#[test]
fn a_suppressed_finding_keeps_its_resolved_fields() {
    let graph = vacancies_for(&["UN-1"]);
    let plain = validate(&graph, &suppress_profile(""), false);
    let suppressed = validate(
        &graph,
        &suppress_profile("  - SUPPRESS: {code: VACANCY}\n"),
        false,
    );
    let before = find(&plain, "VACANCY");
    let after = find(&suppressed, "VACANCY");
    assert_eq!(after.severity, Severity::Error);
    assert_eq!(after.code, before.code);
    assert_eq!(after.message, before.message);
    assert_eq!(after.provenance, before.provenance);
    assert_eq!(after.node_id, before.node_id);
    assert!(after.suppressed && !before.suppressed);
}

#[test]
fn a_suppressed_finding_does_not_gate() {
    let profile = suppress_profile("  - SUPPRESS: {code: VACANCY}\n");
    let issues = validate(&vacancies_for(&["UN-1", "UN-2"]), &profile, false);
    assert!(issues.iter().all(|i| i.severity == Severity::Error));
    assert!(!issues.iter().any(Issue::gates));
}

#[test]
fn strict_does_not_unsuppress() {
    let profile =
        suppress_profile("  - VACANCY: {severity: warning}\n  - SUPPRESS: {code: VACANCY}\n");
    let issues = validate(&vacancies_for(&["UN-1"]), &profile, true);
    let vacancy = find(&issues, "VACANCY");
    assert_eq!(vacancy.severity, Severity::Error, "strict still promotes");
    assert!(vacancy.suppressed, "but never takes suppression back");
    assert!(!vacancy.gates());
}

#[test]
fn config_error_suppression_is_rejected_at_load() {
    let error = profile_from(&format!(
        "{SUPPRESS_PROFILE}validations:\n  - SUPPRESS: {{code: CONFIG_ERROR}}\n"
    ))
    .expect_err("silencing CONFIG_ERROR is a load error");
    assert!(error.0.contains("CONFIG_ERROR"), "{}", error.0);
}

// Requirement: SUPPRESS_UNUSED built-in code

#[test]
fn suppress_unused_for_an_unmatched_code() {
    let profile = suppress_profile("  - SUPPRESS: {code: TYPO_CODE}\n");
    let issues = validate(&vacancies_for(&["UN-1"]), &profile, false);
    let unused = find(&issues, "SUPPRESS_UNUSED");
    assert_eq!(unused.severity, Severity::Info);
    assert!(unused.message.contains("TYPO_CODE"), "{}", unused.message);
    assert!(
        unused.message.contains("not a built-in code"),
        "{}",
        unused.message
    );
    assert!(
        unused.message.contains("no finding carried it"),
        "{}",
        unused.message
    );
    assert!(!unused.suppressed);
    assert!(!find(&issues, "VACANCY").suppressed, "a typo fails open");
}

#[test]
fn suppress_unused_for_a_dormant_built_in_code() {
    let profile = suppress_profile("  - SUPPRESS: {code: VACANCY}\n");
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "UN-1", "kind": "need", "attrs": {},
                   "provenance": {"file": "needs.md", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);
    assert_eq!(codes(&issues), ["SUPPRESS_UNUSED"]);
    let unused = &issues[0];
    assert!(
        unused.message.contains("'VACANCY' is a built-in code"),
        "{}",
        unused.message
    );
    assert!(
        unused.message.contains("no finding carried it"),
        "{}",
        unused.message
    );
}

#[test]
fn suppress_unused_for_node_ids_the_code_never_named() {
    let profile = suppress_profile("  - SUPPRESS: {code: VACANCY, node_ids: [UN-1]}\n");
    let issues = validate(&vacancies_for(&["UN-2", "UN-3"]), &profile, false);
    let unused = find(&issues, "SUPPRESS_UNUSED");
    assert!(unused.message.contains("\"UN-1\""), "{}", unused.message);
    assert!(
        unused
            .message
            .contains("findings carried it this run, but none for those node_ids"),
        "{}",
        unused.message
    );
    assert!(suppressed_ids(&issues, "VACANCY").is_empty());
}

#[test]
fn suppress_unused_names_only_the_unmatched_node_ids() {
    let profile = suppress_profile("  - SUPPRESS: {code: VACANCY, node_ids: [UN-1, UN-2]}\n");
    let issues = validate(&vacancies_for(&["UN-1"]), &profile, false);
    let unused = find(&issues, "SUPPRESS_UNUSED");
    assert!(unused.message.contains("[\"UN-2\"]"), "{}", unused.message);
    assert_eq!(suppressed_ids(&issues, "VACANCY"), ["UN-1"]);
}

#[test]
fn suppress_unused_names_the_profile_as_provenance() {
    let profile = suppress_profile("  - SUPPRESS: {code: TYPO_CODE}\n");
    let issues = validate(&vacancies_for(&["UN-1"]), &profile, false);
    let unused = find(&issues, "SUPPRESS_UNUSED");
    assert_eq!(unused.node_id, None);
    assert_eq!(unused.provenance, Provenance::new("<profile>", 0));
}

#[test]
fn suppress_unused_is_overridable_and_strict_promotable() {
    let profile = suppress_profile(
        "  - SUPPRESS_UNUSED: {severity: warning}\n  - SUPPRESS: {code: TYPO_CODE}\n",
    );
    let graph = vacancies_for(&[]);
    let lenient = validate(&graph, &profile, false);
    assert_eq!(
        find(&lenient, "SUPPRESS_UNUSED").severity,
        Severity::Warning
    );
    let strict = validate(&graph, &profile, true);
    let unused = find(&strict, "SUPPRESS_UNUSED");
    assert_eq!(unused.severity, Severity::Error);
    assert!(unused.gates());
}

#[test]
fn suppress_unused_is_itself_suppressible_in_one_pass() {
    let profile = suppress_profile(
        "  - SUPPRESS: {code: SUPPRESS_UNUSED}\n  \
         - SUPPRESS: {code: TYPO_ONE}\n  \
         - SUPPRESS: {code: TYPO_TWO}\n",
    );
    let issues = validate(&vacancies_for(&[]), &profile, false);
    let unused: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.code == "SUPPRESS_UNUSED")
        .collect();
    assert_eq!(
        unused.len(),
        2,
        "one per stale entry, none for the self-reference"
    );
    assert!(unused.iter().all(|i| i.suppressed));
    assert!(unused.iter().any(|i| i.message.contains("TYPO_ONE")));
    assert!(unused.iter().any(|i| i.message.contains("TYPO_TWO")));
}

#[test]
fn a_self_referencing_suppress_with_nothing_stale_reports_itself_once() {
    // Deterministic and one-pass: the entry matched nothing, so it is stale;
    // the finding saying so is then suppressed by that same entry.
    let profile = suppress_profile("  - SUPPRESS: {code: SUPPRESS_UNUSED}\n");
    let issues = validate(&vacancies_for(&[]), &profile, false);
    assert_eq!(codes(&issues), ["SUPPRESS_UNUSED"]);
    assert!(issues[0].suppressed);
}

#[test]
fn suppression_reaches_adapter_issues() {
    let profile = suppress_profile("  - SUPPRESS: {code: PARSE_ERROR}\n");
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [],
        "issues": [{"severity": "error", "code": "PARSE_ERROR", "message": "m",
                    "provenance": {"file": "needs.md", "line": 3}, "node_id": null}],
    }));
    let issues = validate(&graph, &profile, false);
    let parse = find(&issues, "PARSE_ERROR");
    assert!(parse.suppressed);
    assert!(!issues.iter().any(Issue::gates));
}
