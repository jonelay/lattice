//! The `validation` suites' shared profile and finding helpers.

use latt_core::profile::Profile;
use latt_core::types::Issue;
use serde_json::{Value, json};

use latt_core::graph::LatticeGraph;
use std::path::Path;

use proptest::test_runner::{Config, FileFailurePersistence, TestRunner};

use super::{ingest, profile_from};

/// A profile with the kinds the scenarios name: `need`, `req`, `test`.
pub const KINDS_PROFILE: &str = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  need:
    id_pattern: "^NEED-\\d+$"
  req:
    id_pattern: "^REQ-\\d{4}$"
    attrs:
      text: {type: string, required: true}
      status: {type: enum, values: [done, partial, todo, blocked]}
      tags: {type: list, items: string}
      milestones: {type: list, items: date}
      count: {type: int}
      expires: {type: date}
  test:
    id_pattern: "^T-\\d+$"
edge_kinds:
  verifies:
    allowed: [[test, req]]
  fulfills:
    allowed: [[req, need]]
  derives:
    allowed: [[req, req]]
"#;

pub fn kinds_profile() -> Profile {
    profile_from(KINDS_PROFILE).expect("the scenarios' profile loads")
}

/// A `req` node carrying the given attrs, complete enough not to trip other checks.
pub fn req(id: &str, attrs: Value) -> Value {
    json!({"id": id, "kind": "req", "attrs": attrs,
           "provenance": {"file": "REQS.md", "line": 42}})
}

/// A node with no attrs, declared at `file:line`.
pub fn node(id: &str, kind: &str, file: &str, line: u32) -> Value {
    json!({"id": id, "kind": kind, "attrs": {},
           "provenance": {"file": file, "line": line}})
}

/// An edge with no attrs, declared at `file:line`.
pub fn edge(src: &str, tgt: &str, kind: &str, file: &str, line: u32) -> Value {
    json!({"src": src, "tgt": tgt, "kind": kind,
           "provenance": {"file": file, "line": line}})
}

/// An issue the adapter emitted itself, with message `m` at `REQS.md:12`.
pub fn adapter_issue(code: &str, severity: &str, node_id: Option<&str>) -> Value {
    json!({"severity": severity, "code": code, "message": "m",
           "provenance": {"file": "REQS.md", "line": 12}, "node_id": node_id})
}

/// The graph a document holding only these nodes and edges ingests to.
pub fn graph(nodes: Vec<Value>, edges: Vec<Value>) -> LatticeGraph {
    ingest(json!({"interface_version": "1.0", "nodes": nodes, "edges": edges}))
}

pub fn codes(issues: &[Issue]) -> Vec<&str> {
    issues.iter().map(|i| i.code.as_str()).collect()
}

pub fn with_code<'a>(issues: &'a [Issue], code: &str) -> Vec<&'a Issue> {
    issues.iter().filter(|i| i.code == code).collect()
}

/// Fail unless no finding carries any of `absent`.
pub fn assert_absent(issues: &[Issue], absent: &[&str]) {
    assert!(
        !issues.iter().any(|i| absent.contains(&i.code.as_str())),
        "expected none of {absent:?}, got {:?}",
        codes(issues)
    );
}

pub fn find<'a>(issues: &'a [Issue], code: &str) -> &'a Issue {
    issues
        .iter()
        .find(|i| i.code == code)
        .unwrap_or_else(|| panic!("expected a {code} finding, got {:?}", codes(issues)))
}

pub fn constraint_profile(constraint_yaml: &str) -> String {
    format!("{KINDS_PROFILE}validations:\n  - CONSTRAINT:\n{constraint_yaml}")
}

/// `KINDS_PROFILE` plus one `COVERAGE` entry per named edge kind, all on `req`.
pub fn coverage_profile(edge_kinds: &[&str]) -> String {
    let mut yaml = format!("{KINDS_PROFILE}validations:\n");
    for edge_kind in edge_kinds {
        yaml.push_str(&format!(
            "  - COVERAGE:\n      target_kind: req\n      edge_kind: {edge_kind}\n"
        ));
    }
    yaml
}

/// `REQ-0001` unverified beside an unattributed `T-1`.
pub fn with_unattributed_test() -> LatticeGraph {
    graph(
        vec![
            req("REQ-0001", json!({"text": "t"})),
            node("T-1", "test", "t.py", 1),
        ],
        vec![],
    )
}

/// A runner with the `proptest!` macro's defaults, failing seeds persisted in
/// the crate's `proptest-regressions/tests/<suite>.txt`.
///
/// Properties here are plain `#[test] fn`s driving it, not `proptest!` blocks:
/// the source-existence scanner does not count a `fn` inside a macro
/// invocation, so a macro-defined test is a libtest name nothing resolves. The
/// macro's own default would also write the seed beside the suite, not under
/// `proptest-regressions/`, because no `lib.rs` sits above `tests/`.
pub fn property_runner(source_file: &'static str) -> TestRunner {
    let suite = Path::new(source_file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("a suite path names a file");
    let seeds = format!(
        "{}/proptest-regressions/tests/{suite}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    TestRunner::new(Config {
        source_file: Some(source_file),
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(seeds.leak()))),
        ..Config::default()
    })
}
