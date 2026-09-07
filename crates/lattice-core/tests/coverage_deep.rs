//! The `coverage-query` capability's deep-coverage rollup scenarios.
//!
//! Written from `openspec/specs/coverage-query/spec.md` ("Deep coverage
//! rollup"). The rollup is a least fixed point: seeded with directly
//! evidenced targets, grown by "every child covered", so what these tests
//! pin above all is which of the two solutions a cycle resolves to.

mod common;

use common::{ingest, profile_from};
use lattice_core::graph::LatticeGraph;
use lattice_core::types::{Issue, Severity};
use lattice_core::validate::validate;
use serde_json::{Value, json};

/// Kinds for the scenarios: `req` targets, `test` evidence sources, and an
/// `other` kind the `via` edge admits so the foreign-child rule is testable.
const DEEP_PROFILE: &str = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^R-\\d+$"
  test:
    id_pattern: "^T-\\d+$"
    orphan_ok: true
  other:
    id_pattern: "^O-\\d+$"
    orphan_ok: true
edge_kinds:
  verifies:
    allowed: [[test, req]]
  derives:
    allowed: [[req, req], [other, req]]
validations:
  - COVERAGE_DEEP:
      target_kind: req
      via: derives
      evidence: verifies
"#;

fn node(id: &str, kind: &str) -> Value {
    json!({"id": id, "kind": kind, "attrs": {},
           "provenance": {"file": "r.md", "line": 1}})
}

fn edge(src: &str, tgt: &str, kind: &str) -> Value {
    json!({"src": src, "tgt": tgt, "kind": kind,
           "provenance": {"file": "r.md", "line": 1}})
}

fn graph(nodes: Vec<Value>, edges: Vec<Value>) -> LatticeGraph {
    ingest(json!({"contract_version": "1.0", "nodes": nodes, "edges": edges}))
}

fn deep_findings(issues: &[Issue]) -> Vec<&Issue> {
    issues
        .iter()
        .filter(|i| i.code == "COVERAGE_DEEP")
        .collect()
}

fn deep_for<'a>(issues: &'a [Issue], node_id: &str) -> Vec<&'a Issue> {
    issues
        .iter()
        .filter(|i| i.code == "COVERAGE_DEEP" && i.node_id.as_deref() == Some(node_id))
        .collect()
}

fn run(nodes: Vec<Value>, edges: Vec<Value>) -> Vec<Issue> {
    let profile = profile_from(DEEP_PROFILE).unwrap();
    validate(&graph(nodes, edges), &profile, false)
}

// Requirement: Deep coverage rollup

// Scenario: Parent covered through fully verified children

#[test]
fn parent_covered_through_fully_verified_children() {
    let issues = run(
        vec![
            node("R-1", "req"),
            node("R-2", "req"),
            node("R-3", "req"),
            node("T-1", "test"),
            node("T-2", "test"),
        ],
        vec![
            edge("R-2", "R-1", "derives"),
            edge("R-3", "R-1", "derives"),
            edge("T-1", "R-2", "verifies"),
            edge("T-2", "R-3", "verifies"),
        ],
    );
    assert!(
        deep_findings(&issues).is_empty(),
        "{:?}",
        deep_findings(&issues)
    );
}

// Scenario: One unverified child leaves the parent uncovered

#[test]
fn one_unverified_child_leaves_the_parent_uncovered_and_is_named() {
    let issues = run(
        vec![
            node("R-1", "req"),
            node("R-2", "req"),
            node("R-3", "req"),
            node("T-1", "test"),
        ],
        vec![
            edge("R-2", "R-1", "derives"),
            edge("R-3", "R-1", "derives"),
            edge("T-1", "R-2", "verifies"),
        ],
    );
    assert_eq!(deep_for(&issues, "R-1").len(), 1);
    assert_eq!(deep_for(&issues, "R-3").len(), 1);
    assert!(deep_for(&issues, "R-2").is_empty());
    let parent = deep_for(&issues, "R-1")[0];
    assert!(parent.message.contains("R-3"), "{}", parent.message);
    assert!(!parent.message.contains("R-2"), "{}", parent.message);
    assert_eq!(parent.severity, Severity::Warning);
}

// Scenario: Direct evidence covers a parent regardless of its children

#[test]
fn direct_evidence_covers_a_parent_regardless_of_its_children() {
    let issues = run(
        vec![node("R-1", "req"), node("R-2", "req"), node("T-1", "test")],
        vec![
            edge("R-2", "R-1", "derives"),
            edge("T-1", "R-1", "verifies"),
        ],
    );
    assert!(deep_for(&issues, "R-1").is_empty());
    assert_eq!(deep_for(&issues, "R-2").len(), 1);
}

// Scenario: Childless target without evidence is uncovered

#[test]
fn childless_target_without_evidence_is_uncovered() {
    let issues = run(vec![node("R-9", "req")], vec![]);
    let findings = deep_for(&issues, "R-9");
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0].message.contains("no evidence and no children"),
        "{}",
        findings[0].message
    );
}

// Scenario: Dangling evidence is not coverage

#[test]
fn dangling_evidence_is_not_coverage() {
    let issues = run(
        vec![node("R-1", "req")],
        vec![edge("T-9", "R-1", "verifies")],
    );
    assert_eq!(deep_for(&issues, "R-1").len(), 1);
}

// Scenario: Dangling via source contributes no child

#[test]
fn dangling_via_source_contributes_no_child() {
    let issues = run(
        vec![node("R-1", "req")],
        vec![edge("R-8", "R-1", "derives")],
    );
    let findings = deep_for(&issues, "R-1");
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0].message.contains("no evidence and no children"),
        "{}",
        findings[0].message
    );
}

// Scenario: Child of another kind is not part of the rollup

#[test]
fn a_child_of_another_kind_is_not_part_of_the_rollup() {
    let issues = run(
        vec![node("R-1", "req"), node("O-1", "other")],
        vec![edge("O-1", "R-1", "derives")],
    );
    let findings = deep_for(&issues, "R-1");
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0].message.contains("no evidence and no children"),
        "{}",
        findings[0].message
    );
}

// Scenario: A shared child is one finding, evaluated once

#[test]
fn a_shared_child_is_one_finding_and_both_parents_name_it() {
    let issues = run(
        vec![node("R-4", "req"), node("R-5", "req"), node("R-7", "req")],
        vec![edge("R-7", "R-4", "derives"), edge("R-7", "R-5", "derives")],
    );
    assert_eq!(deep_for(&issues, "R-7").len(), 1);
    for parent in ["R-4", "R-5"] {
        let findings = deep_for(&issues, parent);
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].message.contains("R-7"),
            "{}",
            findings[0].message
        );
    }
}

// Scenario: Evidence-free cycle stays uncovered and is reported

#[test]
fn an_evidence_free_cycle_stays_uncovered_with_identical_member_messages() {
    let issues = run(
        vec![node("R-5", "req"), node("R-6", "req")],
        vec![edge("R-5", "R-6", "derives"), edge("R-6", "R-5", "derives")],
    );
    let a = deep_for(&issues, "R-5");
    let b = deep_for(&issues, "R-6");
    assert_eq!((a.len(), b.len()), (1, 1));
    for finding in [a[0], b[0]] {
        assert!(finding.message.contains("R-5, R-6"), "{}", finding.message);
        assert!(finding.message.contains("cycle"), "{}", finding.message);
    }
}

// Scenario: Anchored cycle propagates coverage out

#[test]
fn an_anchored_cycle_propagates_coverage_out() {
    let issues = run(
        vec![node("R-5", "req"), node("R-6", "req"), node("T-1", "test")],
        vec![
            edge("R-5", "R-6", "derives"),
            edge("R-6", "R-5", "derives"),
            edge("T-1", "R-5", "verifies"),
        ],
    );
    assert!(
        deep_findings(&issues).is_empty(),
        "{:?}",
        deep_findings(&issues)
    );
}

// Scenario: State split propagates

#[test]
fn state_is_unknown_when_an_unattributed_evidence_source_exists() {
    let issues = run(vec![node("R-1", "req"), node("T-1", "test")], vec![]);
    let findings = deep_for(&issues, "R-1");
    assert_eq!(findings[0].state.as_deref(), Some("unknown"));
    let unknown: Vec<_> = issues
        .iter()
        .filter(|i| i.code == "COVERAGE_UNKNOWN")
        .collect();
    assert_eq!(unknown.len(), 1, "one COVERAGE_UNKNOWN per config");
}

#[test]
fn the_unknown_hint_names_the_base_population() {
    let issues = run(
        vec![
            node("R-1", "req"),
            node("R-2", "req"),
            node("T-1", "test"),
            node("T-2", "test"),
        ],
        vec![edge("T-1", "R-2", "verifies")],
    );
    let unknown: Vec<_> = issues
        .iter()
        .filter(|i| i.code == "COVERAGE_UNKNOWN")
        .collect();
    assert_eq!(unknown.len(), 1);
    assert!(
        unknown[0].message.contains("1 of 2"),
        "{}",
        unknown[0].message
    );
    assert!(
        unknown[0].message.contains("50.0%"),
        "{}",
        unknown[0].message
    );
}

#[test]
fn state_is_unverified_when_every_evidence_source_is_attributed() {
    let issues = run(
        vec![node("R-1", "req"), node("R-2", "req"), node("T-1", "test")],
        vec![edge("T-1", "R-2", "verifies")],
    );
    assert_eq!(
        deep_for(&issues, "R-1")[0].state.as_deref(),
        Some("unverified")
    );
    assert!(!issues.iter().any(|i| i.code == "COVERAGE_UNKNOWN"));
}

// Scenario: A faulty entry does not suppress a valid one

#[test]
fn a_faulty_entry_does_not_suppress_a_valid_one() {
    let yaml = format!(
        "{}  - COVERAGE_DEEP:\n      target_kind: req\n      via: nosuch\n\
         \x20     evidence: verifies\n",
        DEEP_PROFILE
    );
    let profile = profile_from(&yaml).unwrap();
    let issues = validate(&graph(vec![node("R-9", "req")], vec![]), &profile, false);
    assert!(
        issues
            .iter()
            .any(|i| i.code == "CONFIG_ERROR" && i.message.contains("nosuch"))
    );
    assert_eq!(deep_for(&issues, "R-9").len(), 1, "valid entry still ran");
}
