//! Lattice's validators against their Trustfall translations, over the shipped
//! fixtures and generated graphs.

mod common;

use std::collections::BTreeSet;

use latt_trustfall::crosscheck::crosscheck;
use latt_trustfall::deliberate::RULES;
use latt_trustfall::generate::generate;

const SEEDS: std::ops::Range<u64> = 0..40;

// Requirement: development-verification/Validators are cross-checked against an independent engine
#[test]
fn crosscheck_shipped_fixtures_and_generated_graphs_agree() {
    let mut failures = Vec::new();
    let mut compared: BTreeSet<String> = BTreeSet::new();
    let mut skipped: BTreeSet<String> = BTreeSet::new();
    let mut fired: BTreeSet<&str> = BTreeSet::new();

    for (name, profile, graph) in common::fixture_pairs() {
        let report = crosscheck(&profile, &graph);
        compared.extend(report.compared.iter().map(|c| format!("{name}: {c}")));
        skipped.extend(
            report
                .skipped
                .iter()
                .map(|(c, why)| format!("{name}: {c}: {why}")),
        );
        fired.extend(report.explained.iter().map(|e| e.rule));
        for mismatch in &report.mismatches {
            failures.push(format!("fixture {name}: {mismatch}"));
        }
    }
    let mut generated = common::shipped_profiles();
    generated.push((
        "shapes".into(),
        latt_core::profile::load_profile(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shapes.yaml"),
        )
        .expect("the shapes profile loads"),
    ));
    for (name, profile) in generated {
        for seed in SEEDS {
            let graph = generate(&profile, seed);
            let report = crosscheck(&profile, &graph);
            compared.extend(report.compared.iter().map(|c| format!("{name}: {c}")));
            skipped.extend(
                report
                    .skipped
                    .iter()
                    .map(|(c, why)| format!("{name}: {c}: {why}")),
            );
            fired.extend(report.explained.iter().map(|e| e.rule));
            for mismatch in &report.mismatches {
                failures.push(format!("generated {name} seed {seed}: {mismatch}"));
            }
        }
    }

    eprintln!(
        "compared:\n  {}",
        compared.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );
    eprintln!(
        "skipped:\n  {}",
        skipped.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );
    assert!(
        failures.is_empty(),
        "{} disagreement(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
    // The per-code union can hide a deliberate difference behind another
    // entry's finding on the same node; `crosscheck_each_operator_alone`
    // asserts each listed difference still fires.
    let _ = fired;
    for code in [
        "VACANCY",
        "UNREFERENCED",
        "UNTRACED",
        "COVERAGE",
        "COVERAGE_DEEP",
        "CONSTRAINT",
        "EDGE_WHERE",
        "EDGE_COUNT",
    ] {
        assert!(
            compared.iter().any(|c| c == &format!("shapes: {code}")),
            "the shapes profile did not exercise {code}: {compared:?} {skipped:?}"
        );
    }
}

/// One `CONSTRAINT` per operator, as `expect` and as `reject`, so each
/// operator's translation decides the result alone and no other condition on
/// the same node can mask a disagreement.
const OPERATORS: &[(&str, &str)] = &[
    ("status", "{eq: approved}"),
    ("status", "{ne: approved}"),
    ("status", "{in: [draft, done]}"),
    ("priority", "{eq: 3}"),
    ("priority", "{ne: 3}"),
    ("priority", "{lt: 2}"),
    ("priority", "{gt: 2}"),
    ("priority", "{lte: 0}"),
    ("priority", "{gte: 4}"),
    ("priority", "{eq: \"3\"}"),
    ("owner", "{matches: \"^a\"}"),
    ("owner", "{present: true}"),
    ("owner", "{present: false}"),
    ("critical", "{eq: true}"),
    ("due", "{lt: \"2026-05-01\"}"),
    ("due", "{gte: \"2026-05-01\"}"),
    ("tags", "{present: true}"),
];

const OPERATOR_PROFILE: &str = r#"
name: operators
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    attrs:
      status: {type: enum, values: [draft, approved, done]}
      priority: {type: int}
      owner: {type: string}
      due: {type: date}
      critical: {type: bool}
      tags: {type: list, items: string}
edge_kinds:
  derives:
    allowed: [[req, req]]
validations:
"#;

// Requirement: development-verification/Validators are cross-checked against an independent engine
#[test]
fn crosscheck_each_operator_alone() {
    let dir = std::env::temp_dir().join(format!("latt-trustfall-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    let mut fired: BTreeSet<&str> = BTreeSet::new();
    for (n, (attr, op)) in OPERATORS.iter().enumerate() {
        for block in ["expect", "reject"] {
            let yaml = format!(
                "{OPERATOR_PROFILE}  - CONSTRAINT:\n      kind: req\n      {block}: {{{attr}: {op}}}\n"
            );
            let path = dir.join(format!("op-{n}-{block}.yaml"));
            std::fs::write(&path, yaml).unwrap();
            let profile = latt_core::profile::load_profile(&path)
                .unwrap_or_else(|e| panic!("{attr} {op} {block}: {e}"));
            for seed in 0..25 {
                let graph = generate(&profile, seed);
                let report = crosscheck(&profile, &graph);
                assert!(
                    report.compared.iter().any(|c| c == "CONSTRAINT"),
                    "{attr} {op}: {report:?}"
                );
                fired.extend(report.explained.iter().map(|e| e.rule));
                for mismatch in &report.mismatches {
                    failures.push(format!("{block} {attr} {op} seed {seed}: {mismatch}"));
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "{} disagreement(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
    for rule in RULES {
        assert!(
            fired.contains(rule.rule),
            "deliberate difference '{}' never occurred here",
            rule.rule
        );
    }
}
