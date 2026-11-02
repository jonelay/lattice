//! Validation scenarios: severity overrides, strict mode, hints, the adapter issue channel and pathways.
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
use latt_core::types::{Origin, Provenance, Severity};
use latt_core::validate::{resolve_adapter_issues, validate};
use serde_json::{Value, json};

// Requirement: validation/Strict mode

#[test]
fn strict_promotes_warnings_but_not_infos() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "issues": [adapter_issue("NOTE", "info", None)],
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
        "{issues:?}"
    );
}

// Requirement: validation/Severity override in profile

#[test]
fn a_profile_override_applies_to_a_built_in_code() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - UNREFERENCED:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = graph(vec![req("REQ-0001", json!({"text": "t"}))], vec![]);
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
        "issues": [adapter_issue("PARSE_ERROR", "warning", None)],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "PARSE_ERROR").severity,
        Severity::Info
    );
}

/// An issue arriving at hint from outside core cannot be promoted. Core has no
/// shipped default for the code, so the pre-pass cannot see it coming. Only the
/// severity it actually arrives at says it is advice.
#[test]
fn an_override_promoting_an_externally_emitted_hint_is_a_config_error() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - ADAPTER_ADVICE:\n      severity: error\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({
        "interface_version": "1.1",
        "issues": [adapter_issue("ADAPTER_ADVICE", "hint", None)],
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
        "issues": [adapter_issue("ADAPTER_ADVICE", "hint", None)],
    }));
    let issues = validate(&graph, &profile, false);

    assert_eq!(find(&issues, "ADAPTER_ADVICE").severity, Severity::Hint);
    assert_absent(&issues, &["CONFIG_ERROR"]);
}

/// One `CONFIG_ERROR` per offending code, however many findings arrive under it:
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

    assert_eq!(with_code(&issues, "CONFIG_ERROR").len(), 1, "{issues:?}");
}

// Requirement: validation/Adapter issue channel

#[test]
fn an_adapter_issue_appears_beside_the_graph_findings() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [req("REQ-0001", json!({"text": "t"}))],
        "issues": [adapter_issue("PARSE_ERROR", "error", None)],
    }));
    let issues = validate(&graph, &kinds_profile(), false);

    assert!(codes(&issues).contains(&"PARSE_ERROR"));
    assert!(codes(&issues).contains(&"UNREFERENCED"));
    assert_eq!(
        find(&issues, "PARSE_ERROR").origin,
        Origin::from(Provenance::new("REQS.md", 12))
    );
}

// Requirement: validation/Pathway severity resolution

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
/// graph's only one; otherwise an incidental `UNREFERENCED`/`UNTRACED` would
/// answer the strict case instead of the demotion under test.
fn pathway_graph(trigger: Value) -> LatticeGraph {
    pathway_graph_at(trigger, "warning")
}

/// `pathway_graph`, with the bound finding arriving at `severity`.
fn pathway_graph_at(trigger: Value, severity: &str) -> LatticeGraph {
    let mut attrs = json!({"text": "t"});
    if !trigger.is_null() {
        attrs["trigger"] = trigger;
    }
    ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "nodes": [req("REQ-0001", attrs), req("REQ-0002", json!({"text": "t"}))],
        "edges": [edge("REQ-0001", "REQ-0002", "derives", "REQS.md", 42),
                  edge("REQ-0002", "REQ-0001", "derives", "REQS.md", 43)],
        "issues": [adapter_issue("OBLIGATION_UNBACKED", severity, Some("REQ-0001"))],
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
fn pathway_demotion_never_raises_a_hint() {
    let issues = validate(
        &pathway_graph_at("M4".into(), "hint"),
        &pathway_profile(),
        false,
    );
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Hint
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

// Requirement: validation/Findings the pathway pass cannot resolve

#[test]
fn a_bound_finding_without_a_node_id_keeps_its_severity() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "issues": [adapter_issue("OBLIGATION_UNBACKED", "warning", None)],
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

    let unresolved = with_code(&issues, "PATHWAY_UNRESOLVED");
    assert_eq!(unresolved.len(), 1, "one per bound code, not per finding");
    assert_eq!(unresolved[0].origin, Origin::Profile);
    assert!(unresolved[0].message.contains("OBLIGATION_UNBACKED"));
    // Nothing was demoted: the pairing is the finding, not the register.
    assert!(
        with_code(&issues, "OBLIGATION_UNBACKED")
            .iter()
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
        "issues": [adapter_issue("OBLIGATION_UNBACKED", "warning", Some("REQ-0001"))],
    }));
    let issues = validate(&graph, &profile, false);

    let unresolved = with_code(&issues, "PATHWAY_UNRESOLVED");
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

// Requirement: validation/Severity resolution is shared by every command

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

// Requirement: validation/Findings the pathway pass cannot resolve

#[test]
fn a_bound_finding_naming_a_node_the_graph_lacks_keeps_its_severity() {
    let graph = ingest(json!({
        "interface_version": "1.0",
        "pathways": [{"name": "phase", "order": ["CB", "M0", "M4"], "current": "M0"}],
        "issues": [adapter_issue("OBLIGATION_UNBACKED", "warning", Some("REQ-9999"))],
    }));
    let issues = validate(&graph, &pathway_profile(), false);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Warning
    );
}

// Requirement: validation/Pathway severity resolution

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
        let mut by_message: Vec<(String, Severity)> = with_code(&resolved, "OBLIGATION_UNBACKED")
            .into_iter()
            .map(|i| (i.message.clone(), i.severity))
            .collect();
        by_message.sort();
        by_message
    };
    assert_eq!(issues(1, 2), issues(2, 1));
    assert_eq!(issues(1, 2)[0].1, Severity::Info, "REQ-0001 is not yet due");
    assert_eq!(issues(1, 2)[1].1, Severity::Warning, "REQ-0002 is");
}

// Requirement: validation/Strict mode

#[test]
fn strict_still_promotes_a_finding_that_is_due() {
    let issues = validate(&pathway_graph("M0".into()), &pathway_profile(), true);
    assert_eq!(
        find(&issues, "OBLIGATION_UNBACKED").severity,
        Severity::Error
    );
}

// Requirement: validation/Hint severity tier

#[test]
fn a_demotion_to_hint_is_honoured() {
    let yaml = format!("{KINDS_PROFILE}validations:\n  - UNREFERENCED:\n      severity: hint\n");
    let profile = profile_from(&yaml).unwrap();
    let graph = graph(vec![req("REQ-0001", json!({"text": "t"}))], vec![]);
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
    let graph = graph(vec![req("REQ-0001", json!({"text": "t"}))], vec![]);
    let issues = validate(&graph, &profile, true);

    assert_eq!(find(&issues, "UNREFERENCED").severity, Severity::Hint);
    assert_eq!(find(&issues, "UNTRACED").severity, Severity::Hint);
    assert!(
        !issues.iter().any(|i| i.severity == Severity::Error),
        "{issues:?}"
    );
}

// Requirement: validation/Severity override in profile

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
