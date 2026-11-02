//! Validation scenarios: declared message placeholders and `OVERRIDE_UNUSED`.
//!
//! Each test cites the `openspec/specs/` requirement it holds the core to. Some
//! overlap `pinned_behaviours.rs`, which asks a different question: it pins
//! observed behaviour on paths the real-data gate never exercises. A case can
//! pass one and fail the other. Shared fixtures are in `common/validation.rs`.

mod common;

use common::validation::*;
use common::{ingest, profile_from};
use latt_core::graph::LatticeGraph;
use latt_core::profile::Profile;
use latt_core::types::{Issue, Severity};
use latt_core::validate::validate;
use serde_json::{Value, json};

// Requirement: validation/Declared messages render placeholders

const MESSAGE_PROFILE: &str = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    attrs:
      owner: {type: string}
  test:
    id_pattern: "^T-\\d+$"
edge_kinds:
  verifies:
    allowed: [[test, req]]
  derives:
    allowed: [[req, req]]
validations:
"#;

fn message_graph(edges: Vec<Value>) -> LatticeGraph {
    let mut owned = node("REQ-1", "req", "REQS.md", 1);
    owned["attrs"] = json!({"owner": "ops"});
    graph(vec![owned, node("REQ-2", "req", "REQS.md", 2)], edges)
}

#[test]
fn message_placeholders_wrap_the_generated_text() {
    let profile = profile_from(&format!(
        "{MESSAGE_PROFILE}  - VACANCY: {{message: \"see ADR-12: {{message}}\"}}\n"
    ))
    .unwrap();
    let graph = message_graph(vec![edge("REQ-1", "REQ-9", "derives", "REQS.md", 1)]);
    let issues = validate(&graph, &profile, false);
    let message = &find(&issues, "VACANCY").message;
    let generated = message
        .strip_prefix("see ADR-12: ")
        .unwrap_or_else(|| panic!("{message}"));
    assert!(generated.contains("'REQ-9' does not exist"), "{message}");
}

#[test]
fn message_placeholders_render_the_node_and_an_absent_attr() {
    let profile = profile_from(&format!(
        "{MESSAGE_PROFILE}  - COVERAGE: {{target_kind: req, edge_kind: verifies, \
         message: \"{{id}} ({{attrs.owner}}) has no test\"}}\n"
    ))
    .unwrap();
    let issues = validate(&message_graph(vec![]), &profile, false);
    let mut messages: Vec<&str> = with_code(&issues, "COVERAGE")
        .into_iter()
        .map(|i| i.message.as_str())
        .collect();
    messages.sort_unstable();
    assert_eq!(
        messages,
        ["REQ-1 (ops) has no test", "REQ-2 (<absent>) has no test"]
    );
}

#[test]
fn message_placeholders_literal_braces_must_be_escaped() {
    let constraint = |message: &str| {
        format!(
            "{MESSAGE_PROFILE}  - CONSTRAINT: {{kind: req, expect: {{owner: {{present: true}}}}, \
             message: \"{message}\"}}\n"
        )
    };
    let error = profile_from(&constraint("use {braces}"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("'{braces}'"), "{error}");

    let profile = profile_from(&constraint("use {{braces}}")).unwrap();
    let issues = validate(&message_graph(vec![]), &profile, false);
    let finding = find(&issues, "CONSTRAINT");
    assert_eq!(finding.node_id.as_deref(), Some("REQ-2"));
    assert_eq!(finding.message, "use {braces}");
}

#[test]
fn message_placeholders_a_node_placeholder_needs_a_node() {
    let error = profile_from(&format!(
        "{MESSAGE_PROFILE}  - SUPPRESS_UNUSED: {{message: \"{{id}} is stale\"}}\n"
    ))
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("findings of 'SUPPRESS_UNUSED' carry none"),
        "{error}"
    );
}

#[test]
fn message_placeholders_never_name_a_location() {
    for placeholder in ["{provenance.file}", "{provenance.line}"] {
        let error = profile_from(&format!(
            "{MESSAGE_PROFILE}  - VACANCY: {{message: \"{{id}} at {}\"}}\n",
            placeholder
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains(placeholder), "{error}");
        assert!(error.contains("names a location"), "{error}");
        assert!(!error.contains("valid: {message}, {code}, {severity}, {id}, {kind}, {provenance"));
    }
}

// Requirement: validation/OVERRIDE_UNUSED built-in code

fn override_unused_profile(validations: &str) -> Profile {
    profile_from(&format!("{MESSAGE_PROFILE}{validations}")).unwrap()
}

#[test]
fn override_unused_reports_a_mistyped_override() {
    let profile = override_unused_profile("  - UNTRACD: {severity: error}\n");
    let issues = validate(&message_graph(vec![]), &profile, false);
    let unused = with_code(&issues, "OVERRIDE_UNUSED");
    assert_eq!(unused.len(), 1, "{issues:?}");
    assert_eq!(unused[0].severity, Severity::Info);
    assert!(
        unused[0].message.contains("'UNTRACD'"),
        "{}",
        unused[0].message
    );
    assert!(unused[0].node_id.is_none());
    assert!(!unused[0].message.to_lowercase().contains("typo"));
    assert!(!issues.iter().any(Issue::gates));
}

#[test]
fn override_unused_is_silent_for_an_external_code_that_fires() {
    let profile = override_unused_profile("  - OBLIGATION_UNBACKED: {severity: error}\n");
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [],
        "edges": [],
        "findings": [{"severity": "warning", "code": "OBLIGATION_UNBACKED", "message": "x",
                      "provenance": {"file": "a", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Error
    );
    assert_absent(&issues, &["OVERRIDE_UNUSED"]);
}

#[test]
fn override_unused_is_silent_for_a_dormant_shipped_code() {
    let profile = override_unused_profile("  - VACANCY: {severity: warning}\n");
    let issues = validate(&message_graph(vec![]), &profile, false);
    assert_absent(&issues, &["OVERRIDE_UNUSED"]);
}

#[test]
fn override_unused_is_strict_promotable_and_suppressible() {
    let strict = override_unused_profile(
        "  - UNTRACD: {severity: error}\n  - OVERRIDE_UNUSED: {severity: warning}\n",
    );
    let issues = validate(&message_graph(vec![]), &strict, true);
    assert_eq!(find(&issues, "OVERRIDE_UNUSED").severity, Severity::Error);
    assert!(issues.iter().any(Issue::gates));

    let suppressed = override_unused_profile(
        "  - UNTRACD: {severity: error}\n  - SUPPRESS: {code: OVERRIDE_UNUSED}\n",
    );
    let issues = validate(&message_graph(vec![]), &suppressed, false);
    assert!(find(&issues, "OVERRIDE_UNUSED").suppressed);
    assert_absent(&issues, &["SUPPRESS_UNUSED"]);
    assert!(!issues.iter().any(Issue::gates));
}
