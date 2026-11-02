//! Validation scenarios: the issue model, built-in validators, orphans and attr placement.
//!
//! Each test cites the `openspec/specs/` requirement it holds the core to. Some
//! overlap `pinned_behaviours.rs`, which asks a different question: it pins
//! observed behaviour on paths the real-data gate never exercises. A case can
//! pass one and fail the other. Shared fixtures are in `common/validation.rs`.

mod common;

use common::validation::*;
use common::{ingest, profile_from};
use latt_core::output::Format;
use latt_core::profile::Profile;
use latt_core::types::{Issue, Origin, Provenance, Severity};
use latt_core::validate::validate;
use proptest::prelude::*;
use serde_json::json;

// Requirement: validation/Issue model

#[test]
fn an_issue_carries_severity_code_provenance_and_message() {
    let graph = graph(vec![req("REQ-0001", json!({"text": "t"}))], vec![]);
    let issues = validate(&graph, &kinds_profile(), false);

    let unreferenced = find(&issues, "UNREFERENCED");
    assert_eq!(unreferenced.severity, Severity::Warning);
    assert_eq!(
        unreferenced.origin,
        Origin::from(Provenance::new("REQS.md", 42))
    );
    assert!(unreferenced.message.contains("REQ-0001"));
}

#[test]
fn a_node_scoped_finding_carries_its_node_id() {
    let graph = graph(vec![req("REQ-1", json!({"text": "t"}))], vec![]);
    let issues = validate(&graph, &kinds_profile(), false);
    assert_eq!(find(&issues, "ID_FORMAT").node_id.as_deref(), Some("REQ-1"));
}

// Requirement: validation/Built-in validators

#[test]
fn an_id_not_matching_its_pattern_is_reported_with_the_node_provenance() {
    let graph = graph(vec![req("REQ-1", json!({"text": "t"}))], vec![]);
    let issues = validate(&graph, &kinds_profile(), false);

    let id_format = find(&issues, "ID_FORMAT");
    assert_eq!(
        id_format.origin,
        Origin::from(Provenance::new("REQS.md", 42))
    );
    assert_eq!(id_format.severity, Severity::Error);
}

#[test]
fn an_edge_between_disallowed_kinds_is_attributed_to_its_source() {
    let graph = graph(
        vec![
            node("NEED-1", "need", "n.md", 1),
            req("REQ-0001", json!({"text": "t"})),
        ],
        vec![edge("NEED-1", "REQ-0001", "verifies", "n.md", 1)],
    );
    let issues = validate(&graph, &kinds_profile(), false);

    let constraint = find(&issues, "EDGE_CONSTRAINT");
    assert_eq!(constraint.node_id.as_deref(), Some("NEED-1"));
    assert_eq!(constraint.severity, Severity::Error);
}

#[test]
fn a_vacancy_is_attributed_to_its_source() {
    let graph = graph(
        vec![req("REQ-0604", json!({"text": "t"}))],
        vec![edge("REQ-0604", "RISK-001", "derives", "REQS.md", 7)],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert_eq!(
        find(&issues, "VACANCY").node_id.as_deref(),
        Some("REQ-0604")
    );
}

#[test]
fn a_missing_required_attr_is_reported() {
    let graph = graph(vec![req("REQ-0001", json!({}))], vec![]);
    let issues = validate(&graph, &kinds_profile(), false);

    let required = find(&issues, "ATTR_REQUIRED");
    assert!(required.message.contains("text"), "{}", required.message);
}

#[test]
fn an_attr_of_the_wrong_type_is_reported() {
    let graph = graph(
        vec![req("REQ-0001", json!({"text": "t", "count": "seven"}))],
        vec![],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "ATTR_TYPE").message.contains("count"));
}

#[test]
fn an_enum_value_outside_the_declared_list_is_reported() {
    let graph = graph(
        vec![req("REQ-0001", json!({"text": "t", "status": "unknown"}))],
        vec![],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "ATTR_ENUM").message.contains("unknown"));
}

#[test]
fn a_list_element_of_the_wrong_type_is_reported() {
    let graph = graph(
        vec![req("REQ-0001", json!({"text": "t", "tags": ["a", 42]}))],
        vec![],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    let items = find(&issues, "ATTR_LIST_ITEMS");
    assert!(items.message.contains("element 1"), "{}", items.message);
    assert!(items.message.contains("got int 42"), "{}", items.message);
    assert!(
        items.message.contains("expected type 'string'"),
        "{}",
        items.message
    );
}

#[test]
fn valid_date_attrs_and_list_items_pass() {
    let graph = graph(
        vec![req(
            "REQ-0001",
            json!({
                "text": "t",
                "expires": "2028-02-29",
                "milestones": ["2026-09-09", "2030-01-15"]
            }),
        )],
        vec![],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert_absent(&issues, &["ATTR_TYPE", "ATTR_LIST_ITEMS"]);
}

#[test]
fn invalid_date_strings_report_the_required_format() {
    for invalid in [
        "2030-02-29",
        "2030-13-01",
        "2030-1-15",
        "2030-01-15T00:00:00Z",
    ] {
        let graph = graph(
            vec![req("REQ-0001", json!({"text": "t", "expires": invalid}))],
            vec![],
        );
        let issues = validate(&graph, &kinds_profile(), false);
        let attr = find(&issues, "ATTR_TYPE");
        assert!(
            attr.message.contains("date (YYYY-MM-DD)"),
            "{}",
            attr.message
        );
    }

    let graph = graph(
        vec![req(
            "REQ-0001",
            json!({"text": "t", "milestones": ["2030-01-15", "2030-13-01"]}),
        )],
        vec![],
    );
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
        "issues": [adapter_issue("PATHWAY_INVALID", "warning", None)],
    }));
    assert_eq!(
        find(&validate(&graph, &profile, false), "PATHWAY_INVALID").severity,
        Severity::Info
    );
}

// Requirement: validation/Suppress findings cascading from an unknown kind

#[test]
fn an_unknown_kind_does_not_cascade_into_edge_constraints() {
    let graph = graph(
        vec![
            node("M-1", "mystery", "m.md", 1),
            req("REQ-0001", json!({"text": "t"})),
        ],
        vec![edge("M-1", "REQ-0001", "verifies", "m.md", 1)],
    );
    let issues = validate(&graph, &kinds_profile(), false);

    assert_eq!(
        find(&issues, "UNKNOWN_KIND").node_id.as_deref(),
        Some("M-1")
    );
    assert_absent(&issues, &["EDGE_CONSTRAINT"]);
}

// Checks that must stay silent, and configuration faults the scenarios above do
// not reach. A validator that fires on valid input is as wrong as one that
// misses, and neither shows up in a test that only counts findings.

// Requirement: validation/Built-in validators
// Requirement: coverage-query/Orphan detection

#[test]
fn a_well_formed_node_produces_no_finding_but_the_directional_orphan_ones() {
    let graph = graph(
        vec![req(
            "REQ-0001",
            json!({"text": "t", "status": "done", "tags": ["a"], "count": 3}),
        )],
        vec![],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    let mut found = codes(&issues);
    found.retain(|code| *code != "EMPTY_KIND");
    found.sort_unstable();
    assert_eq!(found, ["UNREFERENCED", "UNTRACED"], "{issues:?}");
}

#[test]
fn a_fully_connected_node_gets_no_directional_orphan_finding() {
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
        ],
        vec![
            edge("REQ-0001", "REQ-0002", "derives", "REQS.md", 1),
            edge("REQ-0002", "REQ-0001", "derives", "REQS.md", 2),
        ],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert_absent(&issues, &["UNREFERENCED", "UNTRACED"]);
}

/// The source of a one-way edge has only outgoing edges, so it is unreferenced
/// and traced; the target has only incoming ones, so the reverse.
#[test]
fn a_node_with_edges_one_way_is_orphaned_only_the_other_way() {
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
        ],
        vec![edge("REQ-0001", "REQ-0002", "derives", "REQS.md", 1)],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    let node_ids = |code: &str| {
        with_code(&issues, code)
            .into_iter()
            .map(|i| i.node_id.as_deref())
            .collect::<Vec<_>>()
    };
    assert_eq!(node_ids("UNREFERENCED"), [Some("REQ-0001")], "{issues:?}");
    assert_eq!(node_ids("UNTRACED"), [Some("REQ-0002")], "{issues:?}");
}

/// Over any `derives` edges among up to six `req` nodes, `UNREFERENCED`
/// names exactly the nodes no edge enters and `UNTRACED` exactly the nodes
/// no edge leaves, counted from the generated edge list.
#[test]
fn orphan_findings_follow_the_edge_degrees() {
    let strategy =
        (1usize..=6).prop_flat_map(|n| (Just(n), prop::collection::vec((0..n, 0..n), 0..=10)));
    property_runner(file!())
        .run(&strategy, |(n, edges)| {
            let id = |i: usize| format!("REQ-{:04}", i + 1);
            let nodes = (0..n).map(|i| req(&id(i), json!({"text": "t"}))).collect();
            let edge_values = edges
                .iter()
                .map(|&(s, t)| edge(&id(s), &id(t), "derives", "REQS.md", 1))
                .collect();
            let issues = validate(&graph(nodes, edge_values), &kinds_profile(), false);
            let named = |code: &str| {
                let mut ids: Vec<String> = with_code(&issues, code)
                    .into_iter()
                    .filter_map(|i| i.node_id.clone())
                    .collect();
                ids.sort();
                ids
            };
            let without = |end: fn(&(usize, usize)) -> usize| -> Vec<String> {
                (0..n)
                    .filter(|i| !edges.iter().any(|e| end(e) == *i))
                    .map(id)
                    .collect()
            };
            prop_assert_eq!(named("UNREFERENCED"), without(|e| e.1));
            prop_assert_eq!(named("UNTRACED"), without(|e| e.0));
            prop_assert_eq!(
                issues.iter().filter(|i| i.code != "EMPTY_KIND").count(),
                named("UNREFERENCED").len() + named("UNTRACED").len(),
                "{:?}",
                codes(&issues)
            );
            Ok(())
        })
        .unwrap();
}

// Requirement: validation/Built-in validators

#[test]
fn an_allowed_endpoint_pair_produces_no_constraint_finding() {
    let graph = graph(
        vec![
            node("T-1", "test", "t.py", 1),
            req("REQ-0001", json!({"text": "t"})),
        ],
        vec![edge("T-1", "REQ-0001", "verifies", "t.py", 1)],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert_absent(&issues, &["EDGE_CONSTRAINT"]);
}

#[test]
fn an_edge_of_an_undeclared_kind_is_reported() {
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
        ],
        vec![edge("REQ-0001", "REQ-0002", "invents", "REQS.md", 1)],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "UNKNOWN_KIND").message.contains("invents"));
}

#[test]
fn a_boolean_is_not_accepted_where_an_int_is_declared() {
    // A JSON bool is never an int; accepting one would let `count: true` pass.
    let graph = graph(
        vec![req("REQ-0001", json!({"text": "t", "count": true}))],
        vec![],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert!(find(&issues, "ATTR_TYPE").message.contains("count"));
}

// Requirement: output/Deterministic output ordering

/// Findings as JSON, which is the form the ordering contract is stated over.
fn render(issues: &[Issue]) -> String {
    latt_core::output::output_result(issues, Format::Json)
}

#[test]
fn findings_do_not_depend_on_the_order_the_register_was_written_in() {
    let one = graph(
        vec![
            req("REQ-1", json!({})),
            req("REQ-0002", json!({"text": "t"})),
        ],
        vec![],
    );
    let two = graph(
        vec![
            req("REQ-0002", json!({"text": "t"})),
            req("REQ-1", json!({})),
        ],
        vec![],
    );
    let profile = kinds_profile();
    assert_eq!(
        render(&validate(&one, &profile, false)),
        render(&validate(&two, &profile, false))
    );
}

// Requirement: validation/Per-kind orphan exemption

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
    let graph = graph(vec![node("T-1", "test", "t.py", 1)], vec![]);
    let issues = validate(&graph, &orphan_ok_profile(), false);
    assert_absent(&issues, &["UNREFERENCED", "UNTRACED"]);
}

#[test]
fn orphan_ok_on_one_kind_leaves_the_others_checked() {
    let graph = graph(
        vec![
            node("T-1", "test", "t.py", 1),
            req("REQ-0001", json!({"text": "t"})),
        ],
        vec![],
    );
    let issues = validate(&graph, &orphan_ok_profile(), false);
    let unreferenced = find(&issues, "UNREFERENCED");
    assert_eq!(unreferenced.node_id.as_deref(), Some("REQ-0001"));
    let untraced = find(&issues, "UNTRACED");
    assert_eq!(untraced.node_id.as_deref(), Some("REQ-0001"));
}

// Requirement: validation/Orphan checks follow the allowed pairs

#[test]
fn a_kind_no_allowed_pair_can_reach_is_not_unreferenced() {
    // `test` is a source in `verifies` and a target nowhere; `need` the reverse.
    let graph = graph(
        vec![
            node("T-1", "test", "t.py", 1),
            node("NEED-1", "need", "NEEDS.md", 1),
        ],
        vec![],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    let orphans: Vec<_> = issues
        .iter()
        .filter(|i| i.code == "UNREFERENCED" || i.code == "UNTRACED")
        .map(|i| (i.code.as_str(), i.node_id.as_deref().unwrap()))
        .collect();
    assert_eq!(orphans, [("UNREFERENCED", "NEED-1"), ("UNTRACED", "T-1")]);
}

#[test]
fn a_kind_no_allowed_pair_names_gets_no_orphan_finding() {
    let yaml = KINDS_PROFILE.replace(
        "edge_kinds:\n",
        "  memo:\n    id_pattern: \"^M-\\\\d+$\"\nedge_kinds:\n",
    );
    assert_ne!(yaml, KINDS_PROFILE, "the replacement must have applied");
    let graph = graph(vec![node("M-1", "memo", "m.md", 1)], vec![]);
    let issues = validate(&graph, &profile_from(&yaml).unwrap(), false);
    assert_absent(&issues, &["UNREFERENCED", "UNTRACED"]);
}

// Requirement: validation/Attr findings are placed where the attr was written

/// A type, enum or list-item finding sits where the adapter says the attr was
/// written; a missing attr, written nowhere, stays at the node.
#[test]
fn attr_findings_land_at_the_attr_provenance() {
    let graph = ingest(json!({
        "interface_version": "1.4",
        "nodes": [{"id": "REQ-0001", "kind": "req",
                   "attrs": {"status": "paused", "tags": ["a", 7]},
                   "provenance": {"file": "r.toml", "line": 10},
                   "attr_provenance": {
                       "status": {"file": "r.toml", "line": 12},
                       "tags": {"file": "r.toml", "line": 13}}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    for (code, line) in [
        ("ATTR_ENUM", 12),
        ("ATTR_LIST_ITEMS", 13),
        ("ATTR_REQUIRED", 10),
    ] {
        assert_eq!(
            find(&issues, code).origin,
            Origin::from(Provenance::new("r.toml", line)),
            "{code}"
        );
    }
}

/// Without `attr_provenance` every attr finding stays at the node's line.
#[test]
fn attr_findings_fall_back_to_the_node_provenance() {
    let graph = ingest(json!({
        "interface_version": "1.3",
        "nodes": [{"id": "REQ-0001", "kind": "req",
                   "attrs": {"text": "t", "status": "paused"},
                   "provenance": {"file": "r.md", "line": 4}}],
    }));
    let issues = validate(&graph, &kinds_profile(), false);
    assert_eq!(
        find(&issues, "ATTR_ENUM").origin,
        Origin::from(Provenance::new("r.md", 4))
    );
}

// Requirement: validation/EMPTY_KIND built-in code

#[test]
fn empty_kind_names_each_declared_kind_without_an_instance() {
    let graph = graph(vec![req("REQ-0001", json!({"text": "t"}))], vec![]);
    let issues = validate(&graph, &kinds_profile(), false);

    let empty = with_code(&issues, "EMPTY_KIND");
    let messages: Vec<&str> = empty.iter().map(|i| i.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "edge kind 'derives' has no edge",
            "edge kind 'fulfills' has no edge",
            "edge kind 'verifies' has no edge",
            "node kind 'need' has no node",
            "node kind 'test' has no node",
        ]
    );
    for issue in &empty {
        assert_eq!(issue.severity, Severity::Info, "{}", issue.message);
        assert_eq!(issue.origin, Origin::Profile, "{}", issue.message);
        assert_eq!(issue.node_id, None, "{}", issue.message);
    }
}

#[test]
fn empty_kind_is_absent_when_every_kind_is_populated() {
    let graph = graph(
        vec![
            node("NEED-1", "need", "n.md", 1),
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t"})),
            node("T-1", "test", "t.md", 1),
        ],
        vec![
            edge("T-1", "REQ-0001", "verifies", "t.md", 2),
            edge("REQ-0001", "NEED-1", "fulfills", "REQS.md", 43),
            edge("REQ-0002", "REQ-0001", "derives", "REQS.md", 44),
        ],
    );
    let issues = validate(&graph, &kinds_profile(), false);
    assert_absent(&issues, &["EMPTY_KIND"]);
}
