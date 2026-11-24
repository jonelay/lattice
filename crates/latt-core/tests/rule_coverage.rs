mod common;
use latt_core::document::ingest_document;
use latt_core::service::{
    GapState, InputContext, InputStatus, Report, Request, View, execute, prepare_profile_str,
};
use proptest::prelude::*;
use serde_json::json;

const PROFILE: &str = r#"
name: coverage
profile_version: '1.0.0'
node_kinds:
  item: {id_pattern: '.*'}
  source: {id_pattern: '.*'}
edge_kinds:
  evidence: {allowed: [[source, item]]}
validations:
  - COVERAGE:
      target_kind: item
      edge_kind: evidence
      where: {active: {eq: true}}
      source_where: {active: {eq: true}}
"#;

fn answer(
    mut doc: serde_json::Value,
    profile: &str,
    context: InputContext,
) -> latt_core::service::RuleCoverageReport {
    for key in ["nodes", "edges"] {
        for value in doc[key].as_array_mut().unwrap() {
            value["provenance"] = json!({"file":"register","line":1});
        }
    }
    let graph = ingest_document(doc).unwrap();
    let profile = prepare_profile_str(profile, "test").unwrap();
    let response = execute(
        View {
            graph: &graph,
            profile: &profile,
            context: &context,
        },
        Request::RuleCoverage { rule: 1 },
    )
    .unwrap();
    let Report::RuleCoverage(report) = response.payload else {
        panic!("wrong payload")
    };
    report
}

// Requirement: coverage-query/Rule-specific coverage report
#[test]
fn filters_define_both_populations_and_only_explicit_sources_cover_targets() {
    let doc = json!({"interface_version":"1.2", "nodes":[
        {"id":"I1","kind":"item","attrs":{"active":true}},
        {"id":"I2","kind":"item","attrs":{"active":true}},
        {"id":"I3","kind":"item","attrs":{"active":true}},
        {"id":"I4","kind":"item","attrs":{"active":false}},
        {"id":"S1","kind":"source","attrs":{"active":true}},
        {"id":"S2","kind":"source","attrs":{"active":true}},
        {"id":"S3","kind":"source","attrs":{"active":false}}
    ], "edges":[
        {"src":"S1","tgt":"I1","kind":"evidence"},
        {"src":"S1","tgt":"I1","kind":"evidence"},
        {"src":"S3","tgt":"I2","kind":"evidence"},
        {"src":"absent","tgt":"I3","kind":"evidence"},
        {"src":"S2","tgt":"absent","kind":"evidence"}
    ]});
    let report = answer(
        doc,
        PROFILE,
        InputContext::default().with_status(InputStatus::Partial),
    );
    assert_eq!((report.total, report.covered), (3, 1));
    assert_eq!(report.covered_pct, Some(33.3));
    assert_eq!((report.source_total, report.unattributed), (2, 0));
    assert_eq!(
        report
            .missing
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        ["I2", "I3"]
    );
    assert!(
        report
            .missing
            .iter()
            .all(|n| n.state == GapState::Unverified)
    );
    assert_eq!(report.where_, json!({"active":{"eq":true}}));
    assert_eq!(report.source_where, report.where_);
    assert!(report.input_partial);
}

// Requirement: coverage-query/Rule-specific coverage report
#[test]
fn a_gap_state_spells_as_the_json_output_does() {
    for (state, wire) in [
        (GapState::Unverified, "unverified"),
        (GapState::Unknown, "unknown"),
    ] {
        assert_eq!(state.as_str(), wire);
        assert_eq!(state.to_string(), wire);
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            format!("\"{wire}\"")
        );
    }
}

#[test]
fn empty_denominator_has_no_percentage_and_no_claim_of_complete_input() {
    let report = answer(
        json!({"interface_version":"1.2", "nodes":[],"edges":[]}),
        PROFILE,
        InputContext::default(),
    );
    assert_eq!((report.total, report.covered), (0, 0));
    assert_eq!(report.covered_pct, None);
    assert!(report.missing.is_empty());
    assert!(!report.input_partial);
}

// Requirement: coverage-query/Rule-specific coverage report
#[test]
fn missing_and_severity_only_rules_are_refused() {
    let graph = ingest_document(json!({"interface_version":"1.2","nodes":[],"edges":[]})).unwrap();
    for validations in ["", "validations:\n  - COVERAGE: {severity: hint}\n"] {
        let base = PROFILE.split("validations:").next().unwrap();
        let loaded = prepare_profile_str(&format!("{base}{validations}"), "test");
        if !validations.is_empty() {
            assert!(loaded.is_err(), "severity-only COVERAGE is refused at load");
            continue;
        }
        let profile = loaded.unwrap();
        let context = InputContext::default();
        assert!(
            execute(
                View {
                    graph: &graph,
                    profile: &profile,
                    context: &context
                },
                Request::RuleCoverage { rule: 1 }
            )
            .is_err()
        );
    }
}

// Requirement: coverage-query/Rule-specific coverage report
#[test]
fn covered_and_missing_partition_the_selected_population() {
    common::validation::property_runner(file!())
        .run(&prop::collection::vec(any::<bool>(), 0..80), |flags| {
            let mut nodes = vec![json!({"id":"S","kind":"source","attrs":{"active":true}})];
            let mut edges = Vec::new();
            let mut expected_missing = Vec::new();
            for (i, covered) in flags.iter().enumerate() {
                let id = format!("I{i:03}");
                nodes.push(json!({"id":id,"kind":"item","attrs":{"active":true}}));
                if *covered {
                    edges.push(json!({"src":"S","tgt":id,"kind":"evidence"}));
                } else {
                    expected_missing.push(id);
                }
            }
            nodes.reverse();
            let report = answer(
                json!({"interface_version":"1.2","nodes":nodes,"edges":edges}),
                PROFILE,
                InputContext::default(),
            );
            prop_assert_eq!(report.total, flags.len());
            prop_assert_eq!(report.covered, flags.iter().filter(|x| **x).count());
            prop_assert_eq!(report.covered + report.missing.len(), report.total);
            prop_assert_eq!(
                report.missing.into_iter().map(|n| n.id).collect::<Vec<_>>(),
                expected_missing
            );
            Ok(())
        })
        .unwrap();
}
