//! The shipped `requirements-rm` profile, held to the `rm-profile` capability.
//!
//! The profile is data, but it is data the consumer register's IDs are
//! matched against — and Rust's regex engine is not Python's. A pattern that
//! silently stopped matching would show up here rather than as a register that
//! quietly lost its nodes.

use std::path::PathBuf;

use lattice_core::profile::{Profile, load_profile};

fn rm_profile() -> Profile {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../profiles/requirements-rm.yaml");
    load_profile(&path).expect("the shipped profile loads")
}

// Requirement: Node kinds
// Requirement: Edge kinds

#[test]
fn it_declares_five_node_kinds_and_three_edge_kinds() {
    let profile = rm_profile();
    let mut kinds: Vec<&str> = profile.node_kinds().keys().map(String::as_str).collect();
    kinds.sort_unstable();
    assert_eq!(kinds, ["need", "req", "sc", "spec-goal", "test"]);

    let mut edges: Vec<&str> = profile.edge_kinds().keys().map(String::as_str).collect();
    edges.sort_unstable();
    assert_eq!(edges, ["derives", "fulfills", "verifies"]);
}

// Requirement: ID patterns

#[test]
fn each_id_pattern_accepts_its_shape_and_rejects_the_neighbouring_one() {
    let profile = rm_profile();
    let cases: [(&str, &[&str], &[&str]); 5] = [
        ("need", &["BN-1", "UN-42"], &["N-1", "BN-", "REQ-0001"]),
        ("req", &["REQ-0001"], &["REQ-1", "REQ-00001", "req-0001"]),
        // `01-3.6a`: a letter-suffixed insertion between 3.6 and 3.7. One
        // lowercase letter on the last component only — renumbering to make
        // room would change the IDs of goals that already exist, and node IDs
        // are the public join key.
        (
            "spec-goal",
            &["01-1.1", "12-10.3", "01-3.6a"],
            &["1-1.1", "01-1", "01-3.6ab", "01-3.6A", "01-3a.6"],
        ),
        (
            "test",
            &[
                "tests/test_fem.py::test_mesh",
                "tests/test_fem.py::TestMesh::test_mesh",
            ],
            &["tests/test_fem.py::helper", "tests/test_fem.py"],
        ),
        ("sc", &["SC-001"], &["SC-1", "SC-0001"]),
    ];

    for (kind, accepted, rejected) in cases {
        let node_kind = &profile.node_kinds()[kind];
        for id in accepted {
            assert!(node_kind.id_matches(id), "{kind} must accept {id}");
        }
        for id in rejected {
            assert!(!node_kind.id_matches(id), "{kind} must reject {id}");
        }
    }
}

// Requirement: Need node attrs
// Requirement: Req node attrs
// Requirement: Spec-goal node attrs
// Requirement: Test node attrs
// Requirement: SC node attrs

#[test]
fn the_kinds_that_carry_required_attrs_declare_them() {
    let profile = rm_profile();
    let cases: [(&str, &[&str]); 5] = [
        ("need", &["text", "tier"]),
        ("req", &["text", "domain"]),
        ("spec-goal", &["title", "status", "file"]),
        ("test", &["file", "function"]),
        ("sc", &["text", "stakeholder", "category"]),
    ];
    for (kind, required) in cases {
        let attrs = &profile.node_kinds()[kind].attrs;
        for name in required {
            assert!(attrs[*name].required, "{kind}.{name} is required");
        }
    }

    // The two enums downstream reads by name.
    assert_eq!(
        profile.node_kinds()["spec-goal"].attrs["status"]
            .values
            .as_deref(),
        Some(
            ["done", "partial", "todo", "blocked"]
                .map(String::from)
                .as_slice()
        )
    );
    assert_eq!(
        profile.node_kinds()["need"].attrs["tier"].values.as_deref(),
        Some(["business", "user"].map(String::from).as_slice())
    );
}

// Requirement: Edge kinds

#[test]
fn the_edge_kinds_admit_exactly_the_pairs_they_declare() {
    let profile = rm_profile();
    let derives = &profile.edge_kinds()["derives"];
    for (src, tgt) in [
        ("need", "need"),
        ("req", "need"),
        ("req", "req"),
        ("req", "sc"),
    ] {
        assert!(derives.admits(src, tgt), "derives must admit {src}->{tgt}");
    }
    assert!(!derives.admits("test", "req"));

    assert!(profile.edge_kinds()["verifies"].admits("test", "req"));
    assert!(!profile.edge_kinds()["verifies"].admits("req", "test"));
    assert!(profile.edge_kinds()["fulfills"].admits("spec-goal", "req"));
}

// Requirement: Summary configuration
// Requirement: Requirements need both a test and a spec goal

#[test]
fn it_configures_flat_and_deep_coverage_and_one_rollup() {
    let profile = rm_profile();
    let coverage = &profile.validation_configs()["COVERAGE"];
    assert_eq!(coverage.len(), 1, "fulfills stays a flat rule");
    assert_eq!(coverage[0]["edge_kind"].as_str(), Some("fulfills"));

    // The verifies rule is the deep rollup since 1.9.0: direct evidence, or
    // every deriving child covered.
    let deep = &profile.validation_configs()["COVERAGE_DEEP"];
    assert_eq!(deep.len(), 1);
    assert_eq!(deep[0]["target_kind"].as_str(), Some("req"));
    assert_eq!(deep[0]["via"].as_str(), Some("derives"));
    assert_eq!(deep[0]["evidence"].as_str(), Some("verifies"));

    let summary = &profile.validation_configs()["SUMMARY"];
    assert_eq!(summary.len(), 1, "summary renders exactly one rollup");
    assert_eq!(summary[0]["node_kind"].as_str(), Some("spec-goal"));
    assert_eq!(summary[0]["group_by_attr"].as_str(), Some("file"));
}

// Requirement: Node kinds

#[test]
fn every_kind_declares_a_summary_attr_naming_a_declared_attr() {
    let profile = rm_profile();
    let expected: [(&str, &str); 5] = [
        ("need", "text"),
        ("req", "text"),
        ("sc", "text"),
        ("spec-goal", "title"),
        ("test", "function"),
    ];
    for (kind, attr) in expected {
        assert_eq!(
            profile.node_kinds()[kind].summary_attr.as_deref(),
            Some(attr),
            "{kind}'s summary_attr"
        );
    }
}

// Requirement: Spec quotes shipped ID patterns

#[test]
fn the_spec_quotes_every_shipped_id_pattern() {
    // The spec is the normative contract, so a pattern changed here without the
    // spec following is drift the register would inherit silently.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let spec = std::fs::read_to_string(root.join("openspec/specs/rm-profile/spec.md"))
        .expect("the rm-profile spec is committed");

    for (kind, node_kind) in rm_profile().node_kinds() {
        assert!(
            spec.contains(&node_kind.id_pattern_source),
            "the spec does not quote {kind}'s pattern {:?}",
            node_kind.id_pattern_source
        );
    }
}

// Requirement: Test kind is orphan_ok

#[test]
fn the_test_kind_is_orphan_ok_and_the_others_are_not() {
    let profile = rm_profile();
    for (kind_name, kind) in profile.node_kinds() {
        assert_eq!(
            kind.orphan_ok,
            kind_name == "test",
            "only the test kind is exempt, not '{kind_name}'"
        );
    }
}

// Requirement: Test node attrs

#[test]
fn the_test_kind_carries_an_optional_docstring_attr() {
    let profile = rm_profile();
    let docstring = &profile.node_kinds()["test"].attrs["docstring"];
    assert_eq!(docstring.kind, "string");
    assert!(!docstring.required, "absence is absence, not an error");
}
