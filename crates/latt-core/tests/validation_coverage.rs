//! Validation scenarios: coverage gaps, coverage evidence state and per-entry severity.
//!
//! Each test cites the `openspec/specs/` requirement it holds the core to. Some
//! overlap `pinned_behaviours.rs`, which asks a different question: it pins
//! observed behaviour on paths the real-data gate never exercises. A case can
//! pass one and fail the other. Shared fixtures are in `common/validation.rs`.

mod common;

use common::profile_from;
use common::validation::*;
use latt_core::graph::LatticeGraph;
use latt_core::profile::Profile;
use latt_core::types::{Issue, Severity};
use latt_core::validate::validate;
use serde_json::{Value, json};

// Requirement: validation/Built-in validators

#[test]
fn a_coverage_gap_names_the_uncovered_node() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            node("T-1", "test", "t.py", 1),
        ],
        vec![edge("T-1", "REQ-0002", "verifies", "t.py", 1)],
    );
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
         \x20     edge_kind: verifies\n      where:\n        status: {{ne: deferred}}\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t", "status": "todo"})),
            req("REQ-0002", json!({"text": "t", "status": "deferred"})),
        ],
        vec![],
    );
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

/// The validator's side of this is the composed profile's alone: one bad entry
/// among several, through fuse (`crates/latt/tests/fuse.rs`).
#[test]
fn a_coverage_config_naming_an_undeclared_kind_does_not_load() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n  - COVERAGE:\n      target_kind: nosuch\n\
         \x20     edge_kind: verifies\n"
    );
    let error = profile_from(&yaml).unwrap_err().to_string();
    assert!(error.contains("target_kind 'nosuch'"), "{error}");
}

/// The `COVERAGE` findings raised against one node. `covered_by` declares a
/// second `req` node for the edge to come from, and it draws its own findings.
fn coverage_for<'a>(issues: &'a [Issue], node_id: &str) -> Vec<&'a Issue> {
    let mut found = with_code(issues, "COVERAGE");
    found.retain(|i| i.node_id.as_deref() == Some(node_id));
    found
}

/// `REQ-0001` with one incoming edge of the named kind, so exactly one coverage
/// rule over `verifies`/`fulfills` is satisfied.
fn covered_by(edge_kind: &str) -> LatticeGraph {
    let src = if edge_kind == "verifies" {
        "T-1"
    } else {
        "REQ-0002"
    };
    graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
            node("T-1", "test", "t.py", 1),
        ],
        vec![edge(src, "REQ-0001", edge_kind, "t.py", 1)],
    )
}

// Requirement: coverage-query/Coverage check

#[test]
fn a_dangling_edge_source_is_not_counted_as_coverage() {
    // The edge exists, but nothing declared its source. Counting it would report
    // a requirement as verified by a test the register never declares.
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = graph(
        vec![req("REQ-0001", json!({"text": "t"}))],
        vec![edge("T-9", "REQ-0001", "verifies", "t.py", 1)],
    );
    let issues = validate(&graph, &profile, false);
    assert_eq!(coverage_for(&issues, "REQ-0001").len(), 1);
}

// Requirement: validation/Coverage evidence state

/// `REQ-0001` unverified; `T-1` attributed to `REQ-0002`, so no candidate is left.
fn fully_attributed() -> LatticeGraph {
    graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
            node("T-1", "test", "t.py", 1),
        ],
        vec![edge("T-1", "REQ-0002", "verifies", "t.py", 1)],
    )
}

#[test]
fn coverage_state_is_unverified_when_every_candidate_is_attributed() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let issues = validate(&fully_attributed(), &profile, false);

    let coverage = coverage_for(&issues, "REQ-0001");
    assert_eq!(coverage.len(), 1);
    assert_eq!(coverage[0].state.as_deref(), Some("unverified"));
    assert_absent(&issues, &["COVERAGE_UNKNOWN"]);
}

#[test]
fn coverage_state_is_unknown_when_an_unattributed_candidate_exists() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let issues = validate(&with_unattributed_test(), &profile, false);

    let coverage = coverage_for(&issues, "REQ-0001");
    assert_eq!(coverage.len(), 1);
    assert_eq!(coverage[0].state.as_deref(), Some("unknown"));

    let unknown = find(&issues, "COVERAGE_UNKNOWN");
    assert_eq!(unknown.severity, Severity::Hint);
    assert!(unknown.message.contains("1 of 1"), "{}", unknown.message);
    assert!(unknown.message.contains("test"), "{}", unknown.message);
    assert!(unknown.message.contains("verifies"), "{}", unknown.message);
}

#[test]
fn coverage_state_never_invents_a_finding_for_a_verified_node() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            node("T-1", "test", "t.py", 1),
            node("T-2", "test", "t.py", 9),
        ],
        vec![edge("T-1", "REQ-0001", "verifies", "t.py", 1)],
    );
    let issues = validate(&graph, &profile, false);

    assert!(coverage_for(&issues, "REQ-0001").is_empty());
    // The unattributed population is still one fact worth one hint.
    assert_eq!(find(&issues, "COVERAGE_UNKNOWN").severity, Severity::Hint);
}

#[test]
fn coverage_state_reports_the_population_once_per_config() {
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
            req("REQ-0003", json!({"text": "t"})),
            node("T-1", "test", "t.py", 1),
            node("T-2", "test", "t.py", 2),
            node("T-3", "test", "t.py", 3),
        ],
        vec![],
    );
    let issues = validate(&graph, &profile, false);

    let unknowns = with_code(&issues, "COVERAGE_UNKNOWN");
    assert_eq!(unknowns.len(), 1);
    assert!(
        unknowns[0].message.contains("3 of 3"),
        "{}",
        unknowns[0].message
    );
}

#[test]
fn coverage_state_hint_names_the_base_population() {
    // 1 of 3 tests unattributed. Without the base, the hint reads as "the
    // test layer is not wired".
    let profile = profile_from(&coverage_profile(&["verifies"])).unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
            node("T-1", "test", "t.py", 1),
            node("T-2", "test", "t.py", 2),
            node("T-3", "test", "t.py", 3),
        ],
        vec![
            edge("T-1", "REQ-0002", "verifies", "t.py", 1),
            edge("T-2", "REQ-0002", "verifies", "t.py", 2),
        ],
    );
    let issues = validate(&graph, &profile, false);

    let unknown = find(&issues, "COVERAGE_UNKNOWN");
    assert!(unknown.message.contains("1 of 3"), "{}", unknown.message);
    assert!(unknown.message.contains("33.3%"), "{}", unknown.message);
}

/// T-1 verifies REQ-0002; T-2 carries `t2_attrs`; T-3 is retired. Both T-2 and
/// T-3 are unattributed unless `extra_edges` gives them an edge.
fn with_retired_test(t2_attrs: Value, extra_edges: Vec<Value>) -> LatticeGraph {
    let mut edges = vec![edge("T-1", "REQ-0002", "verifies", "t.py", 1)];
    edges.extend(extra_edges);
    let mut t2 = node("T-2", "test", "t.py", 2);
    t2["attrs"] = t2_attrs;
    let mut t3 = node("T-3", "test", "t.py", 3);
    t3["attrs"] = json!({"status": "retired"});
    graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
            node("T-1", "test", "t.py", 1),
            t2,
            t3,
        ],
        edges,
    )
}

fn source_where_profile() -> String {
    format!(
        "{}      source_where:\n        status: {{ne: retired}}\n",
        coverage_profile(&["verifies"])
    )
}

#[test]
fn coverage_state_source_where_narrows_the_population() {
    let profile = profile_from(&source_where_profile()).unwrap();
    let issues = validate(&with_retired_test(json!({}), vec![]), &profile, false);

    let unknown = find(&issues, "COVERAGE_UNKNOWN");
    assert!(unknown.message.contains("1 of 2"), "{}", unknown.message);
    assert!(
        unknown.message.contains("source_where"),
        "{}",
        unknown.message
    );
}

#[test]
fn coverage_state_is_unverified_when_source_where_excludes_every_candidate() {
    let profile = profile_from(&source_where_profile()).unwrap();
    let graph = with_retired_test(json!({"status": "retired"}), vec![]);
    let issues = validate(&graph, &profile, false);

    assert_eq!(
        coverage_for(&issues, "REQ-0001")[0].state.as_deref(),
        Some("unverified")
    );
    assert_absent(&issues, &["COVERAGE_UNKNOWN"]);
}

#[test]
fn coverage_state_an_edge_from_an_excluded_source_is_not_coverage() {
    let profile = profile_from(&source_where_profile()).unwrap();
    let retired_edge = edge("T-3", "REQ-0001", "verifies", "t.py", 3);
    let issues = validate(
        &with_retired_test(json!({}), vec![retired_edge]),
        &profile,
        false,
    );

    assert_eq!(coverage_for(&issues, "REQ-0001").len(), 1);
}

/// Two tiers of one coverage code over kind `item`: core items at `error`, the
/// rest at `info`; `core_first` picks the order of the two entries.
fn tiered_coverage_profile(code: &str, extra_keys: &str, core_first: bool) -> Profile {
    let entry = |tier: &str, severity: &str| {
        format!(
            "  - {code}:\n      target_kind: item\n{extra_keys}      where: {{tier: {{eq: {tier}}}}}\n      severity: {severity}\n"
        )
    };
    let (a, b) = (entry("core", "error"), entry("extra", "info"));
    let entries = if core_first { a + &b } else { b + &a };
    profile_from(&format!(
        r#"
name: t
profile_version: "1.0.0"
node_kinds:
  item:
    id_pattern: "^I\\d+$"
    orphan_ok: true
  test:
    id_pattern: "^T\\d+$"
    orphan_ok: true
edge_kinds:
  covers:
    allowed: [[test, item]]
  derives:
    allowed: [[item, item]]
validations:
{entries}"#
    ))
    .unwrap()
}

fn tiered_items() -> LatticeGraph {
    let item = |id: &str, tier: &str| {
        let mut item = node(id, "item", "reg.md", 1);
        item["attrs"] = json!({"tier": tier});
        item
    };
    graph(vec![item("I1", "core"), item("I2", "extra")], vec![])
}

fn severity_by_node(issues: &[Issue], code: &str) -> Vec<(String, Severity)> {
    let mut found: Vec<(String, Severity)> = with_code(issues, code)
        .into_iter()
        .map(|i| (i.node_id.clone().unwrap_or_default(), i.severity))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

// Requirement: validation/Built-in validators
// Scenario: Each coverage entry keeps its own severity
#[test]
fn coverage_entry_severity_is_per_entry_in_either_order() {
    for core_first in [true, false] {
        let profile = tiered_coverage_profile("COVERAGE", "      edge_kind: covers\n", core_first);
        let issues = validate(&tiered_items(), &profile, false);
        assert_eq!(
            severity_by_node(&issues, "COVERAGE"),
            [
                ("I1".to_owned(), Severity::Error),
                ("I2".to_owned(), Severity::Info)
            ],
            "core_first={core_first}"
        );
    }
}

// Requirement: validation/Built-in validators
// Scenario: Each deep-coverage entry keeps its own severity
#[test]
fn coverage_deep_entry_severity_is_per_entry_in_either_order() {
    for core_first in [true, false] {
        let profile = tiered_coverage_profile(
            "COVERAGE_DEEP",
            "      via: derives\n      evidence: covers\n",
            core_first,
        );
        let issues = validate(&tiered_items(), &profile, false);
        assert_eq!(
            severity_by_node(&issues, "COVERAGE_DEEP"),
            [
                ("I1".to_owned(), Severity::Error),
                ("I2".to_owned(), Severity::Info)
            ],
            "core_first={core_first}"
        );
    }
}
