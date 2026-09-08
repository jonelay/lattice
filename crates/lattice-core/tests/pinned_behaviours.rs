//! Paths the synthetic fixture never exercises, pinned as the tool's own contract.
//!
//! The baselines cover one register through one profile. Everything they do not
//! contain — a malformed document, a duplicate ID, a dangling edge, a config
//! error, non-ASCII in a finding — is unverified by that gate, so this suite
//! pins it directly. It began as `python_parity.rs`, each expectation captured
//! from the Python core before it was deleted; the consolidation track retired
//! that coupling (2a: message dialect, into `native_messages.rs`; 2b: config
//! semantics), and what remains is lattice's own behaviour, held on its own
//! authority. One expectation keeps its Python provenance: the `ensure_ascii`
//! JSON escaping pin, whose byte-level form is still the ancestor's and is
//! parked deliberately.

mod common;

use common::{MINIMAL_PROFILE, ingest, profile_from};
use lattice_core::document::{ingest_document, parse_document};
use lattice_core::output::output_result;
use lattice_core::profile::load_profile;
use lattice_core::types::{Issue, Provenance, Severity};
use lattice_core::validate::validate;
use serde_json::{Value, json};

// An explicit null is malformed input, not an empty collection: lattice exits 2
// rather than reading it as "nothing to see", which is exactly the silence the
// invariants forbid. The message text is pinned in `native_messages.rs`; the
// rejection itself is the behaviour pinned here.

#[test]
fn explicit_null_document_arrays_are_reported() {
    for key in ["nodes", "edges", "axes", "issues"] {
        let mut document = json!({"contract_version": "1.0"});
        document[key] = Value::Null;
        ingest_document(document).expect_err(&format!(
            "'{key}': null must be reported, not read as empty"
        ));
    }
}

#[test]
fn absent_document_arrays_default_to_empty() {
    let graph = ingest(json!({"contract_version": "1.0"}));
    assert_eq!(graph.iter_nodes().count(), 0);
    assert_eq!(graph.iter_edges().count(), 0);
}

#[test]
fn explicit_null_axes_in_a_profile_is_reported() {
    // Message text pinned in `native_messages.rs`; the rejection is the pin.
    profile_from(&format!("{MINIMAL_PROFILE}axes: null\n"))
        .expect_err("'axes: null' must be reported");
}

#[test]
fn explicit_null_validations_in_a_profile_is_accepted() {
    // Unlike `axes`, this one defaults. The asymmetry is deliberate and pinned
    // rather than tidied away.
    let profile = profile_from(&format!("{MINIMAL_PROFILE}validations: null\n"))
        .expect("'validations: null' is accepted");
    assert!(profile.validation_configs().is_empty());
}

// Output behaviour on input the fixture does not contain.

#[test]
fn json_output_escapes_non_ascii_the_way_python_does() {
    let issue = Issue::new(
        Severity::Warning,
        "PARSE_ERROR",
        "−27% B₁ ≈ 0.96 — café 😀",
        Provenance::new("r.md", 1),
        None,
    );
    let rendered = output_result(std::slice::from_ref(&issue), "json").unwrap();

    // Byte-for-byte what json.dumps(ensure_ascii=True) produces, including the
    // surrogate pair for the astral character.
    assert!(
        rendered.contains(
            r#""message": "\u221227% B\u2081 \u2248 0.96 \u2014 caf\u00e9 \ud83d\ude00""#
        ),
        "rendered was:\n{rendered}"
    );
    assert!(
        rendered.is_ascii(),
        "ensure_ascii output must be pure ASCII"
    );
}

// Every shipped profile must load, since Rust's regex engine is not Python's.

#[test]
fn the_shipped_profiles_load_under_the_rust_regex_engine() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate sits two levels below the repo root");
    for name in ["requirements-rm.yaml", "tomlreg.yaml"] {
        let path = root.join("profiles").join(name);
        let profile = load_profile(&path).unwrap_or_else(|e| panic!("{name} must load: {e}"));
        assert!(
            !profile.node_kinds().is_empty(),
            "{name} declares node kinds"
        );
    }
}

#[test]
fn id_patterns_match_whole_ids_not_substrings() {
    // Python uses `fullmatch`; Rust's `is_match` searches, so an unanchored
    // pattern must still reject a superstring.
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "REQ-\\d+"
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "xxREQ-1yy", "kind": "req", "attrs": {},
                   "provenance": {"file": "r.md", "line": 1}}],
    }));
    let issues = validate(&graph, &profile, false);
    assert!(
        issues.iter().any(|i| i.code == "ID_FORMAT"),
        "an unanchored pattern must not match a superstring"
    );
}

// Findings the fixture never produces.

// Requirement: Built-in validators

#[test]
fn a_dangling_edge_target_is_reported_and_the_node_is_not_invented() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                   "provenance": {"file": "r.md", "line": 1}}],
        "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives",
                   "provenance": {"file": "r.md", "line": 1}}],
    }));
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    let issues = validate(&graph, &profile, false);

    let dangling: Vec<&Issue> = issues.iter().filter(|i| i.code == "DANGLING_REF").collect();
    assert_eq!(dangling.len(), 1);
    assert_eq!(
        dangling[0].message,
        "edge 'REQ-1'->'REQ-9' (kind 'derives'): target 'REQ-9' does not exist"
    );
    assert_eq!(dangling[0].severity, Severity::Error);
    // The edge referred to REQ-1, so it is not an orphan even though its
    // counterpart never existed.
    assert!(!issues.iter().any(|i| i.code == "ORPHAN_NODE"));
}

// Requirement: Provenance on findings

#[test]
fn an_orphan_finding_carries_the_node_provenance() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                   "provenance": {"file": "REQUIREMENTS.md", "line": 102}}],
    }));
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    let issues = validate(&graph, &profile, false);

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "ORPHAN_NODE");
    assert_eq!(
        issues[0].provenance,
        Provenance::new("REQUIREMENTS.md", 102)
    );
}

#[test]
fn a_config_error_names_the_profile_rather_than_a_source_line() {
    let yaml = format!(
        "{MINIMAL_PROFILE}validations:\n  - COVERAGE:\n      target_kind: nosuch\n\
         \x20     edge_kind: derives\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let graph = ingest(json!({"contract_version": "1.0"}));
    let issues = validate(&graph, &profile, false);

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "CONFIG_ERROR");
    assert_eq!(issues[0].severity, Severity::Error);
    assert_eq!(
        issues[0].message,
        "COVERAGE config: target_kind 'nosuch' not in profile node kinds"
    );
    // No register line produced this, so it is attributed to the profile.
    assert_eq!(issues[0].provenance, Provenance::new("<profile>", 0));
}

// Requirement: Strict mode

#[test]
fn strict_promotes_warnings_and_leaves_errors_alone() {
    let graph = ingest(json!({
        "contract_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
                   "provenance": {"file": "r.md", "line": 1}}],
        "issues": [{"severity": "info", "code": "NOTE", "message": "m",
                    "provenance": {"file": "r.md", "line": 1}, "node_id": null}],
    }));
    let profile = profile_from(MINIMAL_PROFILE).unwrap();

    let relaxed = validate(&graph, &profile, false);
    assert_eq!(
        relaxed
            .iter()
            .find(|i| i.code == "ORPHAN_NODE")
            .unwrap()
            .severity,
        Severity::Warning
    );

    let strict = validate(&graph, &profile, true);
    assert_eq!(
        strict
            .iter()
            .find(|i| i.code == "ORPHAN_NODE")
            .unwrap()
            .severity,
        Severity::Error
    );
    // Promotion touches warnings only; an info stays info.
    assert_eq!(
        strict.iter().find(|i| i.code == "NOTE").unwrap().severity,
        Severity::Info
    );
}

// Requirement: Adapter failure is a broken setup, not a finding

#[test]
fn parse_document_rejects_empty_and_malformed_input() {
    assert_eq!(
        parse_document("   ")
            .expect_err("empty is a contract error")
            .0,
        "adapter emitted no output"
    );
    assert!(
        parse_document("{oops}")
            .expect_err("malformed JSON is a contract error")
            .0
            .starts_with("could not parse adapter output as JSON:")
    );
}
