//! The native message vocabulary, blessed by consolidation phase 2a.
//!
//! Type names in core-emitted messages come from the profile's own attr-type
//! vocabulary — `string`, `int`, `float`, `bool`, `list` — extended with `null`
//! and `object`; allowed-value lists render as compact JSON. Several of these
//! started life in the parity suite (now `pinned_behaviours.rs`) pinned to the
//! Python core's `repr` and `type(x).__name__`; they migrated here with native
//! expectations when that dialect was retired.

mod common;

use common::{MINIMAL_PROFILE, ingest, profile_from};
use lattice_core::document::ingest_document;
use lattice_core::output::{output_result, strip_ansi};
use lattice_core::summary::build_summary;
use lattice_core::trace::build_trace_report;
use lattice_core::validate::validate;
use serde_json::{Value, json};

// Requirement: Schema failure messages use the native type vocabulary

#[test]
fn explicit_null_document_arrays_are_reported_with_native_type() {
    for key in ["nodes", "edges", "pathways", "issues"] {
        let mut document = json!({"interface_version": "1.0"});
        document[key] = Value::Null;
        let error = ingest_document(document).expect_err(&format!(
            "'{key}': null must be reported, not read as empty"
        ));
        assert_eq!(
            error.0,
            format!("document: '{key}' must be a list, got null")
        );
    }
}

// Requirement: Profile errors use the native type vocabulary

#[test]
fn profile_null_pathways_error_names_the_native_type() {
    let error = profile_from(&format!("{MINIMAL_PROFILE}pathways: null\n"))
        .expect_err("'pathways: null' must be reported");
    assert!(
        error.0.ends_with(
            "'pathways' must be a list of pathway names, got null. Pathway values are target \
             state and are read from the register, never declared here"
        ),
        "message was {:?}",
        error.0
    );
}

// Requirement: Native message vocabulary

#[test]
fn attr_enum_findings_render_the_allowed_list_as_json() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    attrs:
      status: { type: enum, values: ["can't", "done"] }
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {"status": "nope"},
                   "provenance": {"file": "r.md", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);

    let enum_issue = issues
        .iter()
        .find(|i| i.code == "ATTR_ENUM")
        .expect("enum finding");
    assert_eq!(
        enum_issue.message,
        r#"node 'REQ-1': attr 'status' value 'nope' not in ["can't","done"]"#
    );
}

#[test]
fn attr_type_findings_name_the_native_type() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    attrs:
      count: { type: int }
      tags: { type: list, items: string }
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req",
                   "attrs": {"count": "many", "tags": ["a", {}]},
                   "provenance": {"file": "r.md", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);

    let type_issue = issues
        .iter()
        .find(|i| i.code == "ATTR_TYPE")
        .expect("type finding");
    assert_eq!(
        type_issue.message,
        "node 'REQ-1': attr 'count' expected type 'int', got string"
    );

    let item_issue = issues
        .iter()
        .find(|i| i.code == "ATTR_LIST_ITEMS")
        .expect("list item finding");
    assert_eq!(
        item_issue.message,
        "node 'REQ-1': attr 'tags' element 1 expected type 'string', got object"
    );
}

/// `MINIMAL_PROFILE` plus one `COVERAGE` validation with the given config lines.
fn coverage_profile(config_lines: &str) -> String {
    format!("{MINIMAL_PROFILE}validations:\n  - COVERAGE:\n{config_lines}")
}

fn config_errors(yaml: &str) -> Vec<String> {
    let profile = profile_from(yaml).unwrap();
    let graph = ingest(json!({"interface_version": "1.0"}));
    validate(&graph, &profile, false)
        .into_iter()
        .filter(|i| i.code == "CONFIG_ERROR")
        .map(|i| i.message)
        .collect()
}

// Requirement: Validation config values are typed

#[test]
fn config_typing_wrong_type_is_one_error_not_a_stringified_kind() {
    let errors = config_errors(&coverage_profile(
        "      target_kind: true\n      edge_kind: derives\n",
    ));
    assert_eq!(
        errors,
        ["COVERAGE config: 'target_kind' must be a string, got bool"],
        "one wrong-type error, no missing-keys arm, no stringified check"
    );
}

#[test]
fn config_typing_null_is_wrong_type_not_missing() {
    let errors = config_errors(&coverage_profile(
        "      target_kind: req\n      edge_kind: null\n",
    ));
    assert_eq!(
        errors,
        ["COVERAGE config: 'edge_kind' must be a string, got null"]
    );
}

#[test]
fn config_typing_empty_string_is_a_value_not_an_absence() {
    let errors = config_errors(&coverage_profile(
        "      target_kind: \"\"\n      edge_kind: derives\n",
    ));
    assert_eq!(
        errors,
        ["COVERAGE config: target_kind '' not in profile node kinds"]
    );
}

#[test]
fn config_typing_wrong_type_and_missing_key_are_reported_separately() {
    let errors = config_errors(&coverage_profile("      target_kind: true\n"));
    // Output ordering sorts findings; the behaviour under test is that both
    // faults are reported, each exactly once.
    assert_eq!(
        errors,
        [
            "COVERAGE config missing required keys: edge_kind",
            "COVERAGE config: 'target_kind' must be a string, got bool",
        ]
    );
}

/// `MINIMAL_PROFILE` plus one `COVERAGE_DEEP` validation with the given config lines.
fn coverage_deep_profile(config_lines: &str) -> String {
    format!("{MINIMAL_PROFILE}validations:\n  - COVERAGE_DEEP:\n{config_lines}")
}

#[test]
fn config_typing_deep_keys_are_typed_like_coverage_keys() {
    let errors = config_errors(&coverage_deep_profile(
        "      target_kind: req\n      via: true\n      evidence: derives\n",
    ));
    assert_eq!(
        errors,
        ["COVERAGE_DEEP config: 'via' must be a string, got bool"]
    );
}

#[test]
fn config_typing_deep_mixed_faults_are_all_reported() {
    // Independence: the mistyped key and the missing key each report, and the
    // well-formed key's referenced-kind check still runs — one fault never
    // masks another.
    let errors = config_errors(&coverage_deep_profile(
        "      target_kind: 3\n      evidence: nosuch\n",
    ));
    assert_eq!(
        errors,
        [
            "COVERAGE_DEEP config missing required keys: via",
            "COVERAGE_DEEP config: 'target_kind' must be a string, got int",
            "COVERAGE_DEEP config: evidence 'nosuch' not in profile edge kinds",
        ]
    );
}

#[test]
fn config_typing_coverage_kind_check_survives_a_missing_sibling() {
    // The same independence applied back to flat COVERAGE: an undeclared kind
    // is reported even when the other key is absent.
    let errors = config_errors(&coverage_profile("      edge_kind: nosuch\n"));
    assert_eq!(
        errors,
        [
            "COVERAGE config missing required keys: target_kind",
            "COVERAGE config: edge_kind 'nosuch' not in profile edge kinds",
        ]
    );
}

// Requirement: Summary rollup keys are JSON scalars

#[test]
fn summary_rollup_keys_are_json_scalars() {
    let yaml = format!(
        "{MINIMAL_PROFILE}validations:\n  - SUMMARY:\n      node_kind: req\n\
         \x20     status_attr: status\n      group_by_attr: file\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req",
                   "attrs": {"status": true, "file": "a.md"},
                   "provenance": {"file": "a.md", "line": 1}}],
    }));
    let report = build_summary(&profile, &graph).expect("summary builds");
    assert!(
        report.status_keys.iter().any(|k| k == "true"),
        "keys were {:?}",
        report.status_keys
    );
    assert!(
        !report.status_keys.iter().any(|k| k == "True"),
        "keys were {:?}",
        report.status_keys
    );
}

// Requirement: Key attr scalars render as JSON scalars

#[test]
fn trace_key_attr_renders_non_string_scalars_as_json() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: flag
    attrs:
      flag: { type: bool }
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {"flag": true},
                   "provenance": {"file": "r.md", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);
    let report = build_trace_report(&profile, &graph, issues, "0.0.0");
    let rendered = strip_ansi(&output_result(&report, "plain").unwrap());

    let row = rendered
        .lines()
        .find(|l| l.contains("REQ-1"))
        .expect("trace row for REQ-1");
    assert!(row.contains("true"), "row was {row:?}");
    assert!(!row.contains("True"), "row was {row:?}");
}
