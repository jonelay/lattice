//! Validation scenarios: finding suppression and `SUPPRESS_UNUSED`.
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
use latt_core::types::{Issue, Origin, Severity};
use latt_core::validate::validate;
use proptest::prelude::*;
use serde_json::json;

// Requirement: validation/Finding suppression

/// `need` nodes with `orphan_ok`, so the only findings are the VACANCYs the
/// dangling `derives` edges produce, one per source node (`node_id` = source).
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
    let nodes = ids.iter().zip(1..);
    let edges = nodes.clone();
    graph(
        nodes
            .map(|(id, line)| node(id, "need", "needs.md", line))
            .collect(),
        edges
            .map(|(id, line)| edge(id, "UN-999", "derives", "needs.md", line))
            .collect(),
    )
}

fn suppressed_ids(issues: &[Issue], code: &str) -> Vec<String> {
    with_code(issues, code)
        .into_iter()
        .filter(|i| i.suppressed)
        .map(|i| i.node_id.clone().unwrap_or_default())
        .collect()
}

#[test]
fn suppress_all_findings_of_a_code() {
    let profile = suppress_profile("  - SUPPRESS: {code: VACANCY}\n");
    let issues = validate(&vacancies_for(&["UN-1", "UN-2", "UN-3"]), &profile, false);
    let vacancies = with_code(&issues, "VACANCY");
    assert_eq!(vacancies.len(), 3, "suppression reports, never drops");
    assert!(vacancies.iter().all(|i| i.suppressed));
    assert_absent(&issues, &["SUPPRESS_UNUSED"]);
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
    assert_absent(&issues, &["SUPPRESS_UNUSED"]);
}

#[test]
fn suppress_all_supersedes_an_id_specific_entry() {
    let profile = suppress_profile(
        "  - SUPPRESS: {code: VACANCY, node_ids: [UN-1]}\n  - SUPPRESS: {code: VACANCY}\n",
    );
    let issues = validate(&vacancies_for(&["UN-1", "UN-2"]), &profile, false);
    assert_eq!(suppressed_ids(&issues, "VACANCY"), ["UN-1", "UN-2"]);
    assert_absent(&issues, &["SUPPRESS_UNUSED"]);
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
    assert_eq!(after.origin, before.origin);
    assert_eq!(after.node_id, before.node_id);
    assert!(after.suppressed && !before.suppressed);
}

#[test]
fn a_suppressed_finding_does_not_gate() {
    let profile = suppress_profile("  - SUPPRESS: {code: VACANCY}\n");
    let issues = validate(&vacancies_for(&["UN-1", "UN-2"]), &profile, false);
    assert_eq!(
        suppressed_ids(&issues, "VACANCY").len(),
        2,
        "{:?}",
        codes(&issues)
    );
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

// Requirement: profile-schema/Suppress configuration in profiles

#[test]
fn config_error_suppression_is_rejected_at_load() {
    let error = profile_from(&format!(
        "{SUPPRESS_PROFILE}validations:\n  - SUPPRESS: {{code: CONFIG_ERROR}}\n"
    ))
    .expect_err("silencing CONFIG_ERROR is a load error");
    assert!(
        error.to_string().contains("CONFIG_ERROR"),
        "{}",
        error.to_string()
    );
}

// Requirement: validation/SUPPRESS_UNUSED built-in code

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
    let graph = graph(vec![node("UN-1", "need", "needs.md", 1)], vec![]);
    let issues = without_empty_kind(validate(&graph, &profile, false));
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

/// For vacancies on the set V and a suppression naming the set S, the
/// suppressed findings are exactly V ∩ S, none is dropped, and
/// `SUPPRESS_UNUSED` fires iff S ∖ V is non-empty, naming each of its IDs
/// and none from V ∩ S.
#[test]
fn suppression_by_node_id_is_set_intersection() {
    let ids = vec!["UN-1", "UN-2", "UN-3", "UN-4", "UN-5"];
    let strategy = (
        prop::sample::subsequence(ids.clone(), 0..=5),
        prop::sample::subsequence(ids, 1..=5),
    );
    property_runner(file!())
        .run(&strategy, |(vacant, named)| {
            let profile = suppress_profile(&format!(
                "  - SUPPRESS: {{code: VACANCY, node_ids: [{}]}}\n",
                named.join(", ")
            ));
            let issues = validate(&vacancies_for(&vacant), &profile, false);
            let both: Vec<&str> = vacant
                .iter()
                .copied()
                .filter(|v| named.contains(v))
                .collect();
            let stale: Vec<&str> = named
                .iter()
                .copied()
                .filter(|s| !vacant.contains(s))
                .collect();

            prop_assert_eq!(with_code(&issues, "VACANCY").len(), vacant.len());
            prop_assert_eq!(suppressed_ids(&issues, "VACANCY"), both.clone());
            let unused = with_code(&issues, "SUPPRESS_UNUSED");
            prop_assert_eq!(unused.len(), usize::from(!stale.is_empty()), "{:?}", unused);
            if let Some(unused) = unused.first() {
                for id in &stale {
                    prop_assert!(
                        unused.message.contains(&format!("\"{id}\"")),
                        "{}",
                        unused.message
                    );
                }
                for id in &both {
                    prop_assert!(
                        !unused.message.contains(&format!("\"{id}\"")),
                        "{}",
                        unused.message
                    );
                }
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn suppress_unused_names_the_profile_as_provenance() {
    let profile = suppress_profile("  - SUPPRESS: {code: TYPO_CODE}\n");
    let issues = validate(&vacancies_for(&["UN-1"]), &profile, false);
    let unused = find(&issues, "SUPPRESS_UNUSED");
    assert_eq!(unused.node_id, None);
    assert_eq!(unused.origin, Origin::Profile);
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
    let unused = with_code(&issues, "SUPPRESS_UNUSED");
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
    let issues = without_empty_kind(validate(&vacancies_for(&[]), &profile, false));
    assert_eq!(codes(&issues), ["SUPPRESS_UNUSED"]);
    assert!(issues[0].suppressed);
}

#[test]
fn suppression_reaches_adapter_issues() {
    let profile = suppress_profile("  - SUPPRESS: {code: PARSE_ERROR}\n");
    let graph = ingest(json!({
        "interface_version": "1.0",
        "nodes": [],
        "issues": [adapter_issue("PARSE_ERROR", "error", None)],
    }));
    let issues = validate(&graph, &profile, false);
    let parse = find(&issues, "PARSE_ERROR");
    assert!(parse.suppressed);
    assert!(!issues.iter().any(Issue::gates));
}

/// The scenarios here build sparse graphs over a profile declaring more kinds
/// than they use; the `EMPTY_KIND` lines that earns are another test's subject.
fn without_empty_kind(issues: Vec<Issue>) -> Vec<Issue> {
    issues
        .into_iter()
        .filter(|i| i.code != "EMPTY_KIND")
        .collect()
}
