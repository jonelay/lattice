//! Validation scenarios: `CONSTRAINT` entries and their condition operators.
//!
//! Each test cites the `openspec/specs/` requirement it holds the core to. Some
//! overlap `pinned_behaviours.rs`, which asks a different question: it pins
//! observed behaviour on paths the real-data gate never exercises. A case can
//! pass one and fail the other. Shared fixtures are in `common/validation.rs`.

mod common;

use common::validation::*;
use common::{ingest, profile_from};
use latt_core::types::{Issue, Severity};
use latt_core::validate::validate;
use proptest::prelude::*;
use serde_json::{Value, json};

// Requirement: validation/CONSTRAINT evaluation
// Requirement: validation/CONSTRAINT condition operators

/// A `CONSTRAINT` entry on `req` expecting one condition, e.g. `count: {lt: 5}`.
fn expect(condition: &str) -> String {
    format!("      kind: req\n      expect:\n        {condition}\n")
}

/// The `CONSTRAINT` findings `entry` raises on `REQ-0001` carrying `attrs`.
fn constraint_findings(entry: &str, attrs: Value, strict: bool) -> Vec<Issue> {
    let profile = profile_from(&constraint_profile(entry)).unwrap();
    let graph = ingest(json!({"interface_version": "1.0",
                              "nodes": [req("REQ-0001", attrs)]}));
    let issues = validate(&graph, &profile, strict);
    issues
        .into_iter()
        .filter(|i| i.code == "CONSTRAINT")
        .collect()
}

/// The one `CONSTRAINT` finding `entry` raises, which must name `REQ-0001`.
fn fires_once(entry: &str, attrs: Value) -> Issue {
    let mut found = constraint_findings(entry, attrs, false);
    assert_eq!(found.len(), 1, "expected one CONSTRAINT, got {found:?}");
    let c = found.remove(0);
    assert_eq!(c.node_id.as_deref(), Some("REQ-0001"));
    c
}

fn passes(entry: &str, attrs: Value) {
    let found = constraint_findings(entry, attrs, false);
    assert!(found.is_empty(), "expected no CONSTRAINT, got {found:?}");
}

#[test]
fn constraint_expect_passes_no_finding() {
    passes(&expect("text: {present: true}"), json!({"text": "hello"}));
}

#[test]
fn constraint_expect_fails() {
    let c = fires_once(&expect("status: {present: true}"), json!({"text": "hello"}));
    assert_eq!(c.severity, Severity::Warning);
}

#[test]
fn constraint_reject_fires_and_passes() {
    let entry = "      kind: req\n      reject:\n        status: {eq: \"blocked\"}\n";
    fires_once(entry, json!({"text": "hello", "status": "blocked"}));
    passes(entry, json!({"text": "hello", "status": "done"}));
}

#[test]
fn constraint_when_guard_skips_or_applies() {
    let entry = "      kind: req\n      when:\n        status: {eq: \"done\"}\n      \
                 expect:\n        count: {present: true}\n";
    passes(entry, json!({"text": "hello", "status": "draft"}));
    fires_once(entry, json!({"text": "hello", "status": "done"}));
}

#[test]
fn constraint_both_expect_and_reject() {
    let entry = "      kind: req\n      expect:\n        text: {present: true}\n      \
                 reject:\n        status: {eq: \"blocked\"}\n";
    passes(entry, json!({"text": "hello", "status": "done"}));
    fires_once(entry, json!({"text": "hello", "status": "blocked"}));
    fires_once(entry, json!({"status": "done"}));
}

#[test]
fn constraint_custom_message() {
    let entry = format!(
        "{}      message: \"Requirements must have a count\"\n",
        expect("count: {present: true}")
    );
    let c = fires_once(&entry, json!({"text": "hello"}));
    assert_eq!(c.message, "Requirements must have a count");
}

#[test]
fn constraint_generated_message_names_attr() {
    let c = fires_once(&expect("count: {present: true}"), json!({"text": "hello"}));
    assert!(c.message.contains("count"), "{}", c.message);
}

#[test]
fn constraint_kind_mismatch_skips() {
    // The `req` node lacks `status`, so a kind filter that matched it would fire.
    passes(
        "      kind: test\n      expect:\n        status: {present: true}\n",
        json!({"text": "hello"}),
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
    let graph = ingest(json!({"interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "hello", "status": "blocked"}))]}));
    let issues = validate(&graph, &profile, false);
    let constraints = issues.iter().filter(|i| i.code == "CONSTRAINT").count();
    assert_eq!(constraints, 2, "both CONSTRAINT entries should fire");
}

/// Each operator against a value that satisfies it and one that does not.
#[test]
fn each_operator_holds_on_a_passing_and_fails_on_a_failing_value() {
    let cases = [
        (
            "status: {eq: \"done\"}",
            json!({"status": "done"}),
            json!({"status": "draft"}),
        ),
        (
            "status: {ne: \"blocked\"}",
            json!({"status": "done"}),
            json!({"status": "blocked"}),
        ),
        (
            "status: {in: [\"done\", \"partial\"]}",
            json!({"status": "partial"}),
            json!({"status": "blocked"}),
        ),
        (
            "text: {matches: \"^[A-Z]\"}",
            json!({"text": "Hello"}),
            json!({"text": "hello"}),
        ),
        ("count: {present: true}", json!({"count": 0}), json!({})),
        ("count: {present: false}", json!({}), json!({"count": 5})),
        ("count: {lt: 5}", json!({"count": 4}), json!({"count": 5})),
        ("count: {gt: 5}", json!({"count": 6}), json!({"count": 5})),
        ("count: {lte: 5}", json!({"count": 5}), json!({"count": 6})),
        ("count: {gte: 5}", json!({"count": 5}), json!({"count": 4})),
        // Comparison is type-aware: a string never satisfies an int bound, nor
        // an int a string one.
        ("count: {lt: 5}", json!({"count": 4}), json!({"count": "4"})),
        (
            "count: {eq: \"42\"}",
            json!({"count": "42"}),
            json!({"count": 42}),
        ),
        (
            "expires: {lt: \"2030-01-01\"}",
            json!({"expires": "2029-12-31"}),
            json!({"expires": "2030-01-01"}),
        ),
        (
            "expires: {gt: \"2030-01-01\"}",
            json!({"expires": "2030-01-02"}),
            json!({"expires": "2030-01-01"}),
        ),
        (
            "expires: {lte: \"2030-01-01\"}",
            json!({"expires": "2030-01-01"}),
            json!({"expires": "2030-01-02"}),
        ),
        (
            "expires: {gte: \"2030-01-01\"}",
            json!({"expires": "2030-01-01"}),
            json!({"expires": "2029-12-31"}),
        ),
    ];
    for (condition, passing, failing) in cases {
        let entry = expect(condition);
        let found = constraint_findings(&entry, passing, false);
        assert!(found.is_empty(), "{condition} should pass: {found:?}");
        let found = constraint_findings(&entry, failing, false);
        assert_eq!(found.len(), 1, "{condition} should fail once: {found:?}");
    }
}

/// An ordering operator on an int attr holds exactly when Rust's integer
/// order says it does: `{op: bound}` on `count: value` fires iff
/// `value op bound` is false. The bound sits within two of the value, so
/// the equal case that tells `lt` from `lte` comes up.
#[test]
fn an_int_ordering_operator_agrees_with_integer_order() {
    let ops = prop::sample::select(vec!["lt", "gt", "lte", "gte"]);
    let strategy = (ops, any::<i64>(), -2i64..=2);
    property_runner(file!())
        .run(&strategy, |(op, value, offset)| {
            let bound = value.saturating_add(offset);
            let holds = match op {
                "lt" => value < bound,
                "gt" => value > bound,
                "lte" => value <= bound,
                _ => value >= bound,
            };
            let entry = expect(&format!("count: {{{op}: {bound}}}"));
            let found = constraint_findings(&entry, json!({"count": value}), false);
            prop_assert_eq!(
                found.len(),
                usize::from(!holds),
                "{} {} {}: {:?}",
                value,
                op,
                bound,
                found
            );
            Ok(())
        })
        .unwrap();
}

// Requirement: profile-schema/Validation configuration schema
// Requirement: validation/CONSTRAINT condition operators

fn load_error(entry: &str) -> String {
    profile_from(&constraint_profile(entry))
        .unwrap_err()
        .to_string()
}

#[test]
fn constraint_missing_kind_rejected() {
    let e = load_error("      expect:\n        text: {present: true}\n");
    assert!(e.contains("missing required key: kind"), "{e}");
}

#[test]
fn constraint_missing_expect_and_reject_rejected() {
    let e = load_error("      kind: req\n");
    assert!(
        e.contains("requires at least one of 'expect' or 'reject'"),
        "{e}"
    );
}

#[test]
fn constraint_unknown_operator_rejected() {
    let e = load_error(&expect("count: {between: [1, 5]}"));
    assert!(e.contains("unknown operator 'between'"), "{e}");
}

#[test]
fn constraint_undeclared_kind_rejected() {
    let e = load_error("      kind: widget\n      expect:\n        x: {present: true}\n");
    assert!(e.contains("kind 'widget' not in profile node kinds"), "{e}");
}

#[test]
fn constraint_invalid_regex_rejected() {
    let e = load_error(&expect("text: {matches: \"[invalid\"}"));
    assert!(e.contains("'expect.text.matches' invalid regex"), "{e}");
}

#[test]
fn ordering_op_rejects_non_comparable_values() {
    for op in ["lt", "gt", "lte", "gte"] {
        let cases = [
            ("[1, 2]", "list"),
            ("2.5", "float"),
            ("true", "bool"),
            ("null", "null"),
            ("{a: 1}", "object"),
        ];
        for (value, label) in cases {
            let e = load_error(&expect(&format!("count: {{{op}: {value}}}")));
            assert!(
                e.contains(&format!(
                    "'expect.count.{op}' must be an integer or a string, got {label}"
                )),
                "{op} {label}: {e}"
            );
        }
    }
}

// Requirement: validation/CONSTRAINT evaluation
// Requirement: validation/Pathway severity resolution

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
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "hello", "status": "done"}))],
        "pathways": [{"name": "stage", "order": ["todo", "done"], "current": "todo"}]
    }));
    let issues = validate(&graph, &profile, false);
    assert_eq!(find(&issues, "CONSTRAINT").severity, Severity::Info);
}

// Requirement: validation/CONSTRAINT evaluation
// Requirement: validation/Strict mode

#[test]
fn constraint_strict_promotion() {
    let found = constraint_findings(
        &expect("count: {present: true}"),
        json!({"text": "hello"}),
        true,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].severity, Severity::Error);
}
