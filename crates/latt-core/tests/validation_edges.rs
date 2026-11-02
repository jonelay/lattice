//! Validation scenarios: `EDGE_WHERE` and `EDGE_COUNT` entries, and what their entries share.
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
use latt_core::types::{Issue, Origin, Provenance, Severity};
use latt_core::validate::validate;
use proptest::prelude::*;
use serde_json::{Value, json};

// ── EDGE_WHERE validation ──────────────────────────────────────────
// Requirement: validation/EDGE_WHERE evaluation
// Requirement: profile-schema/EDGE_WHERE configuration

fn edge_where_profile(entries: &[&str]) -> String {
    let mut yaml = format!("{KINDS_PROFILE}validations:\n");
    for entry in entries {
        yaml.push_str("  - EDGE_WHERE:\n");
        yaml.push_str(entry);
    }
    yaml
}

/// `REQ-0001` derives from `REQ-0002`, with the given attrs on the target and the edge.
fn edge_where_graph(src: Value, tgt: Value, edge_attrs: Value) -> LatticeGraph {
    let mut derives = edge("REQ-0001", "REQ-0002", "derives", "REQS.md", 7);
    derives["attrs"] = edge_attrs;
    graph(
        vec![req("REQ-0001", src), req("REQ-0002", tgt)],
        vec![derives],
    )
}

const REJECT_BLOCKED_TARGET: &str =
    "      edge_kind: derives\n      reject:\n        target:\n          status: {eq: blocked}\n";

#[test]
fn edge_where_reject_on_target_fires() {
    let profile = profile_from(&edge_where_profile(&[REJECT_BLOCKED_TARGET])).unwrap();
    let graph = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    let issues = validate(&graph, &profile, false);
    let found = with_code(&issues, "EDGE_WHERE");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    assert_eq!(found[0].node_id.as_deref(), Some("REQ-0001"));
    assert_eq!(found[0].origin, Origin::from(Provenance::new("REQS.md", 7)));
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test]
fn edge_where_passing_edge_has_no_finding() {
    let profile = profile_from(&edge_where_profile(&[REJECT_BLOCKED_TARGET])).unwrap();
    let graph = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b", "status": "done"}),
        json!({}),
    );
    let issues = validate(&graph, &profile, false);
    assert_absent(&issues, &["EDGE_WHERE"]);
}

#[test]
fn edge_where_source_expect_fails() {
    let entry =
        "      edge_kind: derives\n      expect:\n        source:\n          status: {eq: done}\n";
    let profile = profile_from(&edge_where_profile(&[entry])).unwrap();
    let graph = edge_where_graph(
        json!({"text": "a", "status": "todo"}),
        json!({"text": "b"}),
        json!({}),
    );
    let issues = validate(&graph, &profile, false);
    assert_eq!(
        with_code(&issues, "EDGE_WHERE").len(),
        1,
        "{:?}",
        codes(&issues)
    );
}

#[test]
fn edge_where_edge_selector_reads_edge_attrs() {
    let entry = "      edge_kind: derives\n      expect:\n        edge:\n          weight: {present: true}\n";
    let profile = profile_from(&edge_where_profile(&[entry])).unwrap();
    let bare = edge_where_graph(json!({"text": "a"}), json!({"text": "b"}), json!({}));
    assert_eq!(
        with_code(&validate(&bare, &profile, false), "EDGE_WHERE").len(),
        1
    );
    let weighted = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b"}),
        json!({"weight": 3}),
    );
    assert_absent(&validate(&weighted, &profile, false), &["EDGE_WHERE"]);
}

#[test]
fn edge_where_when_guard_on_source_skips() {
    let entry = "      edge_kind: derives\n      when:\n        source:\n          status: {eq: done}\n      \
                 reject:\n        target:\n          status: {eq: blocked}\n";
    let profile = profile_from(&edge_where_profile(&[entry])).unwrap();
    let skipped = edge_where_graph(
        json!({"text": "a", "status": "todo"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    assert_absent(&validate(&skipped, &profile, false), &["EDGE_WHERE"]);
    let checked = edge_where_graph(
        json!({"text": "a", "status": "done"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    assert_eq!(
        with_code(&validate(&checked, &profile, false), "EDGE_WHERE").len(),
        1
    );
}

#[test]
fn edge_where_other_edge_kinds_are_not_evaluated() {
    let entry = "      edge_kind: fulfills\n      reject:\n        target:\n          status: {eq: blocked}\n";
    let profile = profile_from(&edge_where_profile(&[entry])).unwrap();
    let graph = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    assert_absent(&validate(&graph, &profile, false), &["EDGE_WHERE"]);
}

#[test]
fn edge_where_dangling_edge_is_only_a_vacancy() {
    let entry = "      edge_kind: derives\n      expect:\n        target:\n          status: {present: true}\n";
    let profile = profile_from(&edge_where_profile(&[entry])).unwrap();
    let graph = graph(
        vec![req("REQ-0001", json!({"text": "a"}))],
        vec![edge("REQ-0001", "REQ-0009", "derives", "REQS.md", 7)],
    );
    let issues = validate(&graph, &profile, false);
    assert!(codes(&issues).contains(&"VACANCY"));
    assert_absent(&issues, &["EDGE_WHERE"]);
}

#[test]
fn edge_where_still_evaluates_an_illegal_kind_pair() {
    let entry = "      edge_kind: derives\n      expect:\n        target:\n          status: {present: true}\n";
    let profile = profile_from(&edge_where_profile(&[entry])).unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "a"})),
            node("NEED-1", "need", "N.md", 1),
        ],
        vec![edge("REQ-0001", "NEED-1", "derives", "REQS.md", 7)],
    );
    let issues = validate(&graph, &profile, false);
    assert!(codes(&issues).contains(&"EDGE_CONSTRAINT"));
    assert_eq!(
        with_code(&issues, "EDGE_WHERE").len(),
        1,
        "{:?}",
        codes(&issues)
    );
}

#[test]
fn edge_where_generated_message_names_selector_and_attr() {
    let profile = profile_from(&edge_where_profile(&[REJECT_BLOCKED_TARGET])).unwrap();
    let graph = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    let issues = validate(&graph, &profile, false);
    let message = &find(&issues, "EDGE_WHERE").message;
    assert!(message.contains("'REQ-0001'->'REQ-0002'"), "{message}");
    assert!(message.contains("target.status"), "{message}");
}

#[test]
fn edge_where_custom_message_and_severity() {
    let entry = format!(
        "{REJECT_BLOCKED_TARGET}      message: \"No pick of a blocked option\"\n      severity: error\n"
    );
    let profile = profile_from(&edge_where_profile(&[&entry])).unwrap();
    let graph = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    let issues = validate(&graph, &profile, false);
    let finding = find(&issues, "EDGE_WHERE");
    assert_eq!(finding.message, "No pick of a blocked option");
    assert_eq!(finding.severity, Severity::Error);
}

#[test]
fn edge_where_entries_are_honoured_independently() {
    let second = "      edge_kind: derives\n      expect:\n        edge:\n          weight: {present: true}\n";
    let profile = profile_from(&edge_where_profile(&[REJECT_BLOCKED_TARGET, second])).unwrap();
    let graph = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    let issues = validate(&graph, &profile, false);
    assert_eq!(
        with_code(&issues, "EDGE_WHERE").len(),
        2,
        "{:?}",
        codes(&issues)
    );
}

#[test]
fn edge_where_config_with_all_keys_loads() {
    let entry = "      edge_kind: derives\n      when:\n        edge:\n          weight: {present: true}\n      \
                 expect:\n        source:\n          text: {present: true}\n      \
                 reject:\n        target:\n          status: {eq: blocked}\n      \
                 message: m\n      severity: info\n";
    profile_from(&edge_where_profile(&[entry])).unwrap();
}

#[test]
fn edge_where_config_missing_edge_kind_rejected() {
    let entry = "      reject:\n        target:\n          status: {eq: blocked}\n";
    let e = profile_from(&edge_where_profile(&[entry]))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("EDGE_WHERE config missing required key: edge_kind"),
        "{e}"
    );
}

#[test]
fn edge_where_config_undeclared_edge_kind_rejected() {
    let entry =
        "      edge_kind: picks\n      reject:\n        target:\n          status: {eq: blocked}\n";
    let e = profile_from(&edge_where_profile(&[entry]))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("edge_kind 'picks' not in profile edge kinds"),
        "{e}"
    );
}

#[test]
fn edge_where_config_missing_expect_and_reject_rejected() {
    let e = profile_from(&edge_where_profile(&["      edge_kind: derives\n"]))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("EDGE_WHERE config: requires at least one of 'expect' or 'reject'"),
        "{e}"
    );
}

#[test]
fn edge_where_config_attr_in_selector_position_rejected() {
    let entry = "      edge_kind: derives\n      reject:\n        status: {eq: blocked}\n";
    let e = profile_from(&edge_where_profile(&[entry]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("'reject.status'"), "{e}");
    assert!(e.contains("source, target, edge"), "{e}");
}

#[test]
fn edge_where_config_block_not_a_mapping_rejected() {
    let entry = "      edge_kind: derives\n      reject: [target]\n";
    let e = profile_from(&edge_where_profile(&[entry]))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("EDGE_WHERE config: 'reject' must be a mapping"),
        "{e}"
    );
}

#[test]
fn edge_where_config_unknown_operator_rejected() {
    let entry = "      edge_kind: derives\n      expect:\n        target:\n          count: {between: [1, 5]}\n";
    let e = profile_from(&edge_where_profile(&[entry]))
        .unwrap_err()
        .to_string();
    assert!(e.contains("unknown operator 'between'"), "{e}");
    assert!(e.contains("'expect.target.count'"), "{e}");
}

// ── EDGE_COUNT validation ──────────────────────────────────────────
// Requirement: validation/EDGE_COUNT evaluation
// Requirement: profile-schema/EDGE_COUNT configuration

fn edge_count_profile(entry: &str) -> Result<Profile, String> {
    profile_from(&format!(
        "{KINDS_PROFILE}validations:\n  - EDGE_COUNT:\n      edge_kind: derives\n      node_kind: req\n{entry}"
    ))
    .map_err(|e| e.to_string())
}

/// `req` nodes with the given statuses (`None` for no status), and `derives` edges
/// between them by index; edge `i` is declared at line `100 + i`.
fn edge_count_graph(statuses: &[Option<&str>], edges: &[(usize, usize)]) -> LatticeGraph {
    let id = |i: usize| format!("REQ-{:04}", i + 1);
    let nodes: Vec<Value> = statuses
        .iter()
        .enumerate()
        .map(|(i, status)| match status {
            Some(s) => req(&id(i), json!({"text": "t", "status": s})),
            None => req(&id(i), json!({"text": "t"})),
        })
        .collect();
    let edges: Vec<Value> = edges
        .iter()
        .zip(100..)
        .map(|((s, t), line)| edge(&id(*s), &id(*t), "derives", "E.md", line))
        .collect();
    graph(nodes, edges)
}

#[test]
fn edge_count_above_max_fires_once_per_node() {
    let profile = edge_count_profile("      max: 1\n").unwrap();
    let graph = edge_count_graph(&[None, None, None], &[(0, 1), (0, 2)]);
    let issues = validate(&graph, &profile, false);
    let found = with_code(&issues, "EDGE_COUNT");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    let finding = found[0];
    assert_eq!(finding.node_id.as_deref(), Some("REQ-0001"));
    assert_eq!(finding.origin, Origin::from(Provenance::new("REQS.md", 42)));
    assert_eq!(finding.severity, Severity::Warning);
    for part in ["2", "'REQ-0002' (E.md:100)", "'REQ-0003' (E.md:101)"] {
        assert!(
            finding.message.contains(part),
            "{part}: {}",
            finding.message
        );
    }
}

#[test]
fn edge_count_within_bounds_has_no_finding() {
    let profile = edge_count_profile("      min: 1\n      max: 2\n").unwrap();
    let graph = edge_count_graph(&[None, None, None], &[(0, 1), (1, 2), (2, 0)]);
    let issues = validate(&graph, &profile, false);
    assert_absent(&issues, &["EDGE_COUNT"]);
}

#[test]
fn edge_count_zero_edges_is_a_count() {
    let profile = edge_count_profile("      min: 1\n").unwrap();
    let graph = edge_count_graph(&[None, None], &[(0, 1)]);
    let issues = validate(&graph, &profile, false);
    let found = with_code(&issues, "EDGE_COUNT");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    assert_eq!(found[0].node_id.as_deref(), Some("REQ-0002"));
}

#[test]
fn edge_count_incoming_direction_counts_per_target() {
    let profile = edge_count_profile("      direction: in\n      max: 1\n").unwrap();
    let graph = edge_count_graph(&[None, None, None], &[(0, 2), (1, 2)]);
    let issues = validate(&graph, &profile, false);
    let found = with_code(&issues, "EDGE_COUNT");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    assert_eq!(found[0].node_id.as_deref(), Some("REQ-0003"));
}

/// A node gets one `EDGE_COUNT` finding exactly when the edges at its end
/// of the configured direction number outside `[min, max]`, counted from the
/// generated edge list; a self-loop is one edge at each end.
#[test]
fn edge_count_fires_exactly_outside_the_bounds() {
    let strategy = (
        (1usize..=5).prop_flat_map(|n| (Just(n), prop::collection::vec((0..n, 0..n), 0..=10))),
        any::<bool>(),
        (0usize..=3).prop_flat_map(|min| (Just(min), min..=4)),
    );
    property_runner(file!())
        .run(&strategy, |((n, edges), incoming, (min, max))| {
            let direction = if incoming { "in" } else { "out" };
            let profile = edge_count_profile(&format!(
                "      direction: {direction}\n      min: {min}\n      max: {max}\n"
            ))
            .unwrap();
            let issues = validate(&edge_count_graph(&vec![None; n], &edges), &profile, false);
            let mut fired: Vec<String> = with_code(&issues, "EDGE_COUNT")
                .into_iter()
                .filter_map(|i| i.node_id.clone())
                .collect();
            fired.sort();
            let expected: Vec<String> = (0..n)
                .filter(|&i| {
                    let count = edges
                        .iter()
                        .filter(|&&(s, t)| if incoming { t == i } else { s == i })
                        .count();
                    count < min || count > max
                })
                .map(|i| format!("REQ-{:04}", i + 1))
                .collect();
            prop_assert_eq!(fired, expected);
            Ok(())
        })
        .unwrap();
}

#[test]
fn edge_count_grouped_by_far_end_attr() {
    let profile = edge_count_profile("      group_by: status\n      max: 1\n").unwrap();
    let graph = edge_count_graph(
        &[None, Some("done"), Some("done"), Some("todo")],
        &[(0, 1), (0, 2), (0, 3)],
    );
    let issues = validate(&graph, &profile, false);
    let found = with_code(&issues, "EDGE_COUNT");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    assert!(
        found[0].message.contains("\"done\""),
        "{}",
        found[0].message
    );
    assert!(
        !found[0].message.contains("REQ-0004"),
        "{}",
        found[0].message
    );
}

#[test]
fn edge_count_far_end_missing_group_attr_is_its_own_finding() {
    let profile = edge_count_profile("      group_by: status\n      max: 1\n").unwrap();
    let graph = edge_count_graph(&[None, None, Some("done")], &[(0, 1), (0, 2)]);
    let issues = validate(&graph, &profile, false);
    let found = with_code(&issues, "EDGE_COUNT");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    assert_eq!(found[0].origin, Origin::from(Provenance::new("E.md", 100)));
    assert!(found[0].message.contains("status"), "{}", found[0].message);
}

#[test]
fn edge_count_where_limits_counted_nodes() {
    let profile =
        edge_count_profile("      min: 1\n      where:\n        status: {eq: done}\n").unwrap();
    let graph = edge_count_graph(&[Some("todo"), Some("done")], &[]);
    let issues = validate(&graph, &profile, false);
    let found = with_code(&issues, "EDGE_COUNT");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    assert_eq!(found[0].node_id.as_deref(), Some("REQ-0002"));
}

#[test]
fn edge_count_dangling_edges_are_not_counted() {
    let profile = edge_count_profile("      min: 1\n").unwrap();
    let graph = graph(
        vec![req("REQ-0001", json!({"text": "t"}))],
        vec![edge("REQ-0001", "REQ-0009", "derives", "E.md", 1)],
    );
    let issues = validate(&graph, &profile, false);
    assert!(codes(&issues).contains(&"VACANCY"));
    assert_eq!(
        with_code(&issues, "EDGE_COUNT").len(),
        1,
        "{:?}",
        codes(&issues)
    );
}

#[test]
fn edge_count_custom_message_and_severity() {
    let profile = edge_count_profile(
        "      min: 1\n      message: \"Each req derives\"\n      severity: error\n",
    )
    .unwrap();
    let graph = edge_count_graph(&[None], &[]);
    let issues = validate(&graph, &profile, false);
    let finding = find(&issues, "EDGE_COUNT");
    assert_eq!(finding.message, "Each req derives");
    assert_eq!(finding.severity, Severity::Error);
}

#[test]
fn edge_count_config_with_all_keys_loads() {
    edge_count_profile(
        "      direction: in\n      group_by: status\n      min: 0\n      max: 3\n      \
         where:\n        status: {present: true}\n      message: m\n      severity: info\n",
    )
    .unwrap();
}

#[test]
fn edge_count_config_node_kind_never_at_counted_end_rejected() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n  - EDGE_COUNT:\n      edge_kind: fulfills\n      \
         node_kind: need\n      max: 1\n"
    );
    let e = profile_from(&yaml).unwrap_err().to_string();
    for part in ["'need'", "'fulfills'", "out"] {
        assert!(e.contains(part), "{part}: {e}");
    }
    let incoming = yaml.replace("max: 1", "max: 1\n      direction: in");
    profile_from(&incoming).unwrap();
}

#[test]
fn edge_count_config_without_bounds_rejected() {
    let e = edge_count_profile("").unwrap_err();
    assert!(e.contains("requires at least one of 'min' or 'max'"), "{e}");
}

#[test]
fn edge_count_config_min_above_max_rejected() {
    let e = edge_count_profile("      min: 2\n      max: 1\n").unwrap_err();
    assert!(e.contains("'min' 2 exceeds 'max' 1"), "{e}");
}

#[test]
fn edge_count_config_negative_bound_rejected() {
    let e = edge_count_profile("      max: -1\n").unwrap_err();
    assert!(e.contains("'max' must be a non-negative integer"), "{e}");
}

#[test]
fn edge_count_config_invalid_direction_rejected() {
    let e = edge_count_profile("      direction: both\n      max: 1\n").unwrap_err();
    assert!(e.contains("'both'") && e.contains("out, in"), "{e}");
}

#[test]
fn edge_count_config_missing_node_kind_rejected() {
    let yaml = format!(
        "{KINDS_PROFILE}validations:\n  - EDGE_COUNT:\n      edge_kind: derives\n      max: 1\n"
    );
    let e = profile_from(&yaml).unwrap_err().to_string();
    assert!(
        e.contains("EDGE_COUNT config missing required key: node_kind"),
        "{e}"
    );
}

#[test]
fn edge_count_still_counts_an_illegal_kind_pair() {
    let profile = edge_count_profile("      max: 0\n").unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            node("NEED-1", "need", "N.md", 1),
        ],
        vec![edge("REQ-0001", "NEED-1", "derives", "E.md", 1)],
    );
    let issues = validate(&graph, &profile, false);
    assert!(codes(&issues).contains(&"EDGE_CONSTRAINT"));
    assert_eq!(
        with_code(&issues, "EDGE_COUNT").len(),
        1,
        "{:?}",
        codes(&issues)
    );
}

// Requirement: profile-schema/Validation configuration schema

#[test]
fn a_non_string_message_is_a_load_error_for_every_entry_code() {
    let cases = [
        constraint_profile(
            "      kind: req\n      expect:\n        text: {present: true}\n      message: 5\n",
        ),
        edge_where_profile(&[&format!("{REJECT_BLOCKED_TARGET}      message: [a]\n")]),
        format!(
            "{KINDS_PROFILE}validations:\n  - EDGE_COUNT:\n      edge_kind: derives\n      \
             node_kind: req\n      max: 1\n      message: {{a: 1}}\n"
        ),
    ];
    for (yaml, code) in cases.iter().zip(["CONSTRAINT", "EDGE_WHERE", "EDGE_COUNT"]) {
        let e = profile_from(yaml).unwrap_err().to_string();
        assert!(
            e.contains(&format!("{code} config: 'message' must be a string")),
            "{code}: {e}"
        );
    }
}

#[test]
fn an_entry_with_no_conditions_to_check_is_a_load_error() {
    let constraint = constraint_profile("      kind: req\n      expect: {}\n");
    let e = profile_from(&constraint).unwrap_err().to_string();
    assert!(
        e.contains("CONSTRAINT config: 'expect' and 'reject' hold no conditions"),
        "{e}"
    );
    let edge_where =
        edge_where_profile(&["      edge_kind: derives\n      reject:\n        source: {}\n"]);
    let e = profile_from(&edge_where).unwrap_err().to_string();
    assert!(
        e.contains("EDGE_WHERE config: 'expect' and 'reject' hold no conditions"),
        "{e}"
    );
}

// Requirement: validation/EDGE_WHERE evaluation
// Requirement: validation/EDGE_COUNT evaluation
// Requirement: validation/CONSTRAINT condition operators
// Requirement: profile-schema/EDGE_WHERE configuration
// Requirement: profile-schema/EDGE_COUNT configuration
// Requirement: validation/Built-in validators

#[test]
fn matches_coerces_a_scalar_attr_to_text() {
    let entry = "      edge_kind: derives\n      expect:\n        edge:\n          weight: {matches: \"^1$\"}\n";
    let profile = profile_from(&edge_where_profile(&[entry])).unwrap();
    let int = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b"}),
        json!({"weight": 1}),
    );
    assert_absent(&validate(&int, &profile, false), &["EDGE_WHERE"]);
    let flag = "      edge_kind: derives\n      expect:\n        edge:\n          flag: {matches: \"^true$\"}\n";
    let profile = profile_from(&edge_where_profile(&[flag])).unwrap();
    let boolean = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b"}),
        json!({"flag": true}),
    );
    assert_absent(&validate(&boolean, &profile, false), &["EDGE_WHERE"]);
    let list = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b"}),
        json!({"flag": [true]}),
    );
    assert_eq!(
        with_code(&validate(&list, &profile, false), "EDGE_WHERE").len(),
        1,
        "a list is not text"
    );
}

#[test]
fn repeated_entries_keep_their_own_severity() {
    let first = format!("{REJECT_BLOCKED_TARGET}      severity: error\n");
    let second = format!("{REJECT_BLOCKED_TARGET}      severity: info\n");
    let profile = profile_from(&edge_where_profile(&[&first, &second])).unwrap();
    let graph = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    let mut severities: Vec<Severity> = with_code(&validate(&graph, &profile, false), "EDGE_WHERE")
        .iter()
        .map(|i| i.severity)
        .collect();
    severities.sort_by_key(|s| format!("{s:?}"));
    assert_eq!(severities, [Severity::Error, Severity::Info]);
}

// Requirement: validation/Strict mode
// Requirement: validation/Finding suppression

#[test]
fn edge_findings_are_promoted_by_strict_and_selected_by_suppress() {
    let suppressed = format!(
        "{}  - SUPPRESS: {{code: EDGE_WHERE}}\n",
        edge_where_profile(&[REJECT_BLOCKED_TARGET])
    );
    let graph = edge_where_graph(
        json!({"text": "a"}),
        json!({"text": "b", "status": "blocked"}),
        json!({}),
    );
    let strict = validate(
        &graph,
        &profile_from(&edge_where_profile(&[REJECT_BLOCKED_TARGET])).unwrap(),
        true,
    );
    assert_eq!(find(&strict, "EDGE_WHERE").severity, Severity::Error);
    let issues = validate(&graph, &profile_from(&suppressed).unwrap(), false);
    assert!(
        find(&issues, "EDGE_WHERE").suppressed,
        "{:?}",
        codes(&issues)
    );
}

// Requirement: validation/EDGE_WHERE evaluation
// Requirement: validation/EDGE_COUNT evaluation
// Requirement: validation/CONSTRAINT condition operators
// Requirement: profile-schema/EDGE_WHERE configuration
// Requirement: profile-schema/EDGE_COUNT configuration
// Requirement: validation/Built-in validators

#[test]
fn edge_count_all_far_ends_missing_the_group_attr_report_each_edge_only() {
    let profile = edge_count_profile("      group_by: status\n      min: 1\n").unwrap();
    let graph = edge_count_graph(&[None, None, None], &[(0, 1), (0, 2)]);
    let issues = validate(&graph, &profile, false);
    let mine: Vec<&Issue> = with_code(&issues, "EDGE_COUNT")
        .into_iter()
        .filter(|i| i.node_id.as_deref() == Some("REQ-0001"))
        .collect();
    assert_eq!(mine.len(), 2, "{mine:?}");
    assert!(
        mine.iter()
            .all(|i| i.origin.location().map(Provenance::file) == Some("E.md"))
    );
}

#[test]
fn edge_count_null_group_value_is_a_missing_attr() {
    let profile = edge_count_profile("      group_by: status\n      max: 5\n").unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            req("REQ-0002", json!({"text": "t", "status": null})),
        ],
        vec![edge("REQ-0001", "REQ-0002", "derives", "E.md", 1)],
    );
    let issues = validate(&graph, &profile, false);
    let found = with_code(&issues, "EDGE_COUNT");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    assert!(
        found[0].message.contains("has no 'status'"),
        "{}",
        found[0].message
    );
}

#[test]
fn edge_count_min_zero_never_fires_and_self_loops_count_once_per_end() {
    let profile = edge_count_profile("      min: 0\n      max: 1\n").unwrap();
    let graph = edge_count_graph(&[None, None], &[(0, 0)]);
    assert_absent(&validate(&graph, &profile, false), &["EDGE_COUNT"]);
    let incoming = edge_count_profile("      direction: in\n      max: 0\n").unwrap();
    let issues = validate(&graph, &incoming, false);
    let found = with_code(&issues, "EDGE_COUNT");
    assert_eq!(found.len(), 1, "{:?}", codes(&issues));
    assert_eq!(found[0].node_id.as_deref(), Some("REQ-0001"));
}

#[test]
fn edge_count_skips_edges_to_an_unknown_kind() {
    let profile = edge_count_profile("      min: 1\n").unwrap();
    let graph = graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            node("W-1", "widget", "W.md", 1),
        ],
        vec![edge("REQ-0001", "W-1", "derives", "E.md", 1)],
    );
    let issues = validate(&graph, &profile, false);
    assert!(codes(&issues).contains(&"UNKNOWN_KIND"));
    assert_eq!(
        with_code(&issues, "EDGE_COUNT").len(),
        1,
        "the widget edge is not counted"
    );
}

#[test]
fn edge_count_config_rejects_bad_group_by_where_and_kinds() {
    let cases = [
        (
            "      max: 1\n      group_by: [a]\n",
            "'group_by' must be a string",
        ),
        (
            "      max: 1\n      where: [a]\n",
            "'where' must be a mapping",
        ),
    ];
    for (entry, expected) in cases {
        let e = edge_count_profile(entry).unwrap_err();
        assert!(e.contains(expected), "{expected}: {e}");
    }
    for (kinds, expected) in [
        (
            "      edge_kind: picks\n      node_kind: req\n",
            "edge_kind 'picks' not in profile edge kinds",
        ),
        (
            "      edge_kind: derives\n      node_kind: widget\n",
            "node_kind 'widget' not in profile node kinds",
        ),
    ] {
        let yaml = format!("{KINDS_PROFILE}validations:\n  - EDGE_COUNT:\n{kinds}      max: 1\n");
        let e = profile_from(&yaml).unwrap_err().to_string();
        assert!(e.contains(expected), "{expected}: {e}");
    }
}
