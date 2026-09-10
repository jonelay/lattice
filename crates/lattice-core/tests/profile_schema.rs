//! The `profile-schema` capability's scenarios, against the Rust loader.
//!
//! Written from `openspec/specs/profile-schema/spec.md` rather than from
//! `profile.rs`: the spike's only gate was the phase-sweep baselines, which reach
//! one profile down one path. Everything a profile can get wrong is unexercised
//! there, and a test derived from the implementation would agree with it by
//! construction.

mod common;

use common::{MINIMAL_PROFILE, profile_from, profile_from_files};

/// The message of a load that was expected to fail.
fn load_error(yaml: &str) -> String {
    match profile_from(yaml) {
        Ok(_) => panic!("profile loaded but should have been rejected:\n{yaml}"),
        Err(e) => e.0,
    }
}

// Requirement: Profile YAML structure

#[test]
fn valid_profile_loads() {
    let profile = profile_from(MINIMAL_PROFILE).expect("minimal profile is valid");
    assert_eq!(profile.name(), "t");
    assert_eq!(profile.profile_version(), "1.0.0");
    assert!(profile.node_kinds().contains_key("req"));
    assert!(profile.edge_kinds().contains_key("derives"));
}

#[test]
fn missing_required_key_names_it() {
    let yaml = "name: t\nprofile_version: \"1.0.0\"\nedge_kinds: {}\n";
    assert!(load_error(yaml).contains("missing required key 'node_kinds'"));
}

#[test]
fn unrecognised_top_level_key_does_not_reject() {
    // The Rust core drops `extra` rather than carrying it: an adapter is a
    // separate program now and opens the profile itself. What the spec requires
    // here — that core not reject the key — still holds.
    let profile = profile_from(&format!("{MINIMAL_PROFILE}adapter: openspec\n"))
        .expect("an uninterpreted top-level key is not an error");
    assert_eq!(profile.name(), "t");
}

// Requirement: Node kind declaration

#[test]
fn node_kind_records_pattern_and_attrs() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d{4}$"
    attrs:
      text: {type: string, required: true}
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    let req = &profile.node_kinds()["req"];
    assert_eq!(req.id_pattern_source, r"^REQ-\d{4}$");
    assert!(req.id_matches("REQ-0001"));
    assert!(!req.id_matches("REQ-1"));
    let text = &req.attrs["text"];
    assert_eq!(text.kind, "string");
    assert!(text.required);
}

#[test]
fn node_kind_without_attrs_loads_empty() {
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    assert!(profile.node_kinds()["req"].attrs.is_empty());
}

#[test]
fn invalid_regex_names_the_kind_and_the_pattern() {
    let yaml = "name: t\nprofile_version: \"1.0.0\"\n\
                node_kinds:\n  req:\n    id_pattern: \"[invalid\"\nedge_kinds: {}\n";
    let error = load_error(yaml);
    assert!(error.contains("node kind 'req'"), "{error}");
    assert!(error.contains("invalid id_pattern"), "{error}");
}

// Requirement: Node kind declaration

#[test]
fn summary_attr_is_recorded_when_it_names_a_declared_attr() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: text
    attrs:
      text: {type: string, required: true}
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    assert_eq!(
        profile.node_kinds()["req"].summary_attr.as_deref(),
        Some("text")
    );
}

#[test]
fn summary_attr_absent_means_none() {
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    assert_eq!(profile.node_kinds()["req"].summary_attr, None);
}

#[test]
fn summary_attr_naming_an_undeclared_attr_is_rejected() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: title
    attrs:
      text: {type: string, required: true}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("summary_attr"), "{error}");
    assert!(error.contains("title"), "{error}");
}

#[test]
fn text_attrs_records_the_declared_names_in_order() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: title
    text_attrs: [title, body]
    attrs:
      title: {type: string, required: true}
      body: {type: string}
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    assert_eq!(
        profile.node_kinds()["req"].text_attrs.as_deref(),
        Some(["title".to_string(), "body".to_string()].as_slice())
    );
}

#[test]
fn text_attrs_naming_an_undeclared_attr_is_rejected() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [title, body]
    attrs:
      title: {type: string, required: true}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_attrs"), "{error}");
    assert!(error.contains("body"), "{error}");
}

#[test]
fn text_attrs_naming_a_list_attr_is_rejected() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [refs]
    attrs:
      refs: {type: list, items: string}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_attrs"), "{error}");
    assert!(error.contains("list"), "{error}");
}

#[test]
fn text_attrs_naming_an_int_attr_is_rejected() {
    // An int attr's runtime value is never text: a ranking consumer would
    // skip it silently, leaving the key in the profile doing nothing.
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [count]
    attrs:
      count: {type: int}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_attrs"), "{error}");
    assert!(error.contains("count"), "{error}");
    assert!(error.contains("int"), "{error}");
}

#[test]
fn text_attrs_naming_a_bool_attr_is_rejected() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [done]
    attrs:
      done: {type: bool}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_attrs"), "{error}");
    assert!(error.contains("done"), "{error}");
}

#[test]
fn text_attrs_naming_an_enum_attr_is_accepted() {
    // An enum value arrives as a string at runtime, so it ranks like one.
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [stage]
    attrs:
      stage: {type: enum, values: [draft, final]}
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    assert_eq!(
        profile.node_kinds()["req"].text_attrs.as_deref(),
        Some(["stage".to_string()].as_slice())
    );
}

#[test]
fn text_attrs_repeating_a_name_is_rejected() {
    // A repeat would embed one attr's text twice, double-weighting it in a
    // ranking with no visible symptom.
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [title, title]
    attrs:
      title: {type: string, required: true}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_attrs"), "{error}");
    assert!(error.contains("title"), "{error}");
}

#[test]
fn text_attrs_absent_is_distinct_from_declared_empty() {
    // The two mean different things to a ranker: absent falls back to
    // summary_attr, empty declares there is nothing to rank.
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: title
    attrs:
      title: {type: string, required: true}
  note:
    id_pattern: "^N-\\d+$"
    summary_attr: title
    text_attrs: []
    attrs:
      title: {type: string, required: true}
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    assert_eq!(profile.node_kinds()["req"].text_attrs, None);
    assert_eq!(
        profile.node_kinds()["note"].text_attrs.as_deref(),
        Some([].as_slice())
    );
}

#[test]
fn chunk_line_prefix_is_recorded() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [title, body]
    text_chunk_line_prefix: '#### Scenario:'
    attrs:
      title: {type: string, required: true}
      body: {type: string}
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    assert_eq!(
        profile.node_kinds()["req"]
            .text_chunk_line_prefix
            .as_deref(),
        Some("#### Scenario:")
    );
}

#[test]
fn chunk_line_prefix_absent_leaves_the_kind_undivided() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [title]
    attrs:
      title: {type: string, required: true}
edge_kinds: {}
"#;
    let profile = profile_from(yaml).unwrap();
    assert_eq!(profile.node_kinds()["req"].text_chunk_line_prefix, None);
}

#[test]
fn chunk_line_prefix_that_is_not_a_string_is_rejected() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [title]
    text_chunk_line_prefix: 4
    attrs:
      title: {type: string, required: true}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_chunk_line_prefix"), "{error}");
}

#[test]
fn chunk_line_prefix_that_is_blank_is_rejected() {
    // A prefix of spaces is a prefix of almost every line, so it names no cut
    // point — it would chunk on indentation.
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_attrs: [title]
    text_chunk_line_prefix: "   "
    attrs:
      title: {type: string, required: true}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_chunk_line_prefix"), "{error}");
}

#[test]
fn chunk_line_prefix_spanning_a_line_boundary_is_rejected() {
    // The consumer compares the prefix against one line at a time, so a prefix
    // containing a newline could never match and would silently chunk nothing.
    let yaml = "name: t\nprofile_version: \"1.0.0\"\nnode_kinds:\n  req:\n    \
                id_pattern: \"^REQ-\\\\d+$\"\n    text_attrs: [title]\n    \
                text_chunk_line_prefix: \"a\\nb\"\n    attrs:\n      \
                title: {type: string, required: true}\nedge_kinds: {}\n";
    let error = load_error(yaml);
    assert!(error.contains("text_chunk_line_prefix"), "{error}");
}

#[test]
fn chunk_line_prefix_on_a_kind_declaring_no_text_is_rejected() {
    // Configuration that silently does nothing is indistinguishable from
    // configuration that works, which is why a partial pathway binding is rejected
    // on the same grounds.
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: title
    text_attrs: []
    text_chunk_line_prefix: '#### Scenario:'
    attrs:
      title: {type: string, required: true}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_chunk_line_prefix"), "{error}");
}

#[test]
fn chunk_line_prefix_without_any_text_source_is_rejected() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    text_chunk_line_prefix: '#### Scenario:'
    attrs:
      title: {type: string, required: true}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("text_chunk_line_prefix"), "{error}");
}

#[test]
fn summary_attr_naming_a_list_attr_is_rejected() {
    let yaml = r#"
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    summary_attr: refs
    attrs:
      refs: {type: list, items: string}
edge_kinds: {}
"#;
    let error = load_error(yaml);
    assert!(error.contains("summary_attr"), "{error}");
    assert!(error.contains("list"), "{error}");
}

// Requirement: Typed attribute declarations

#[test]
fn enum_attr_records_its_values() {
    let profile = profile_from(&attr_profile("{type: enum, values: [done, todo]}")).unwrap();
    let attr = &profile.node_kinds()["req"].attrs["status"];
    assert_eq!(attr.kind, "enum");
    assert_eq!(
        attr.values.as_deref(),
        Some(["done".to_string(), "todo".to_string()].as_slice())
    );
}

#[test]
fn list_attr_records_its_item_type() {
    let profile = profile_from(&attr_profile("{type: list, items: date}")).unwrap();
    let attr = &profile.node_kinds()["req"].attrs["status"];
    assert_eq!(attr.kind, "list");
    assert_eq!(attr.items.as_deref(), Some("date"));
}

#[test]
fn date_attr_is_accepted_without_subfields() {
    let profile = profile_from(&attr_profile("{type: date}")).unwrap();
    let attr = &profile.node_kinds()["req"].attrs["status"];
    assert_eq!(attr.kind, "date");
    assert_eq!(attr.values, None);
    assert_eq!(attr.items, None);
}

#[test]
fn enum_without_values_is_rejected() {
    let error = load_error(&attr_profile("{type: enum}"));
    assert!(error.contains("enum type requires 'values'"), "{error}");
}

#[test]
fn list_without_items_is_rejected() {
    let error = load_error(&attr_profile("{type: list}"));
    assert!(error.contains("list type requires 'items'"), "{error}");
}

#[test]
fn list_of_a_non_scalar_type_names_the_valid_items() {
    let error = load_error(&attr_profile("{type: list, items: enum}"));
    assert!(error.contains("bool, date, int, string"), "{error}");
}

/// A profile whose `req` kind carries one attr named `status`, declared as given.
fn attr_profile(declaration: &str) -> String {
    format!(
        "name: t\nprofile_version: \"1.0.0\"\n\
         node_kinds:\n  req:\n    id_pattern: \"^REQ-\\\\d+$\"\n\
         \x20   attrs:\n      status: {declaration}\nedge_kinds: {{}}\n"
    )
}

// Requirement: Edge kind declaration

#[test]
fn edge_kind_records_its_allowed_pairs() {
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    let derives = &profile.edge_kinds()["derives"];
    assert!(derives.admits("req", "req"));
    assert!(!derives.admits("req", "test"));
}

#[test]
fn edge_kind_without_allowed_admits_nothing() {
    let yaml = MINIMAL_PROFILE.replace("    allowed: [[req, req]]\n", "");
    let profile = profile_from(&yaml).unwrap();
    assert!(!profile.edge_kinds()["derives"].admits("req", "req"));
}

#[test]
fn edge_kind_naming_an_undefined_node_kind_is_rejected() {
    let yaml = MINIMAL_PROFILE.replace("[[req, req]]", "[[req, nosuch]]");
    let error = load_error(&yaml);
    assert!(
        error.contains("references undefined node kind 'nosuch'"),
        "{error}"
    );
}

// Requirement: Profile version

#[test]
fn supported_major_version_loads() {
    let profile = profile_from(&MINIMAL_PROFILE.replace("1.0.0", "1.9.3")).unwrap();
    assert_eq!(profile.profile_version(), "1.9.3");
}

#[test]
fn unsupported_major_version_is_rejected() {
    let error = load_error(&MINIMAL_PROFILE.replace("1.0.0", "2.0.0"));
    assert!(
        error.contains("unsupported profile version 2.0.0"),
        "{error}"
    );
}

// Requirement: Validation configuration schema

#[test]
fn unknown_config_key_names_the_offender() {
    let yaml = format!("{MINIMAL_PROFILE}validations:\n  - COVERAGE:\n      target_kinds: req\n");
    let error = load_error(&yaml);
    assert!(error.contains("target_kinds"), "{error}");
    assert!(error.contains("target_kind"), "{error}");
}

#[test]
fn unknown_validator_code_takes_severity() {
    let yaml =
        format!("{MINIMAL_PROFILE}validations:\n  - OBLIGATION_UNBACKED:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    assert_eq!(
        profile.validation_overrides()["OBLIGATION_UNBACKED"],
        lattice_core::types::Severity::Info
    );
}

#[test]
fn unknown_validator_code_takes_no_other_key() {
    let yaml =
        format!("{MINIMAL_PROFILE}validations:\n  - OBLIGATION_UNBACKED:\n      node_kind: req\n");
    assert!(load_error(&yaml).contains("node_kind"));
}

#[test]
fn adapter_code_takes_a_pathway_binding() {
    let yaml = format!(
        "{MINIMAL_PROFILE}pathways: [phase]\nvalidations:\n  - OBLIGATION_UNBACKED:\n\
         \x20     pathway: phase\n      position_attr: trigger\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let binding = &profile.pathway_bindings()["OBLIGATION_UNBACKED"];
    assert_eq!(binding.pathway, "phase");
    assert_eq!(binding.position_attr, "trigger");
}

// Requirement: Ordering pathway declaration

#[test]
fn pathway_list_loads() {
    let profile = profile_from(&format!("{MINIMAL_PROFILE}pathways: [phase]\n")).unwrap();
    assert_eq!(profile.pathways(), ["phase"]);
}

#[test]
fn axes_carrying_values_is_rejected() {
    let yaml = format!("{MINIMAL_PROFILE}pathways:\n  phase:\n    order: [a, b]\n    current: a\n");
    let error = load_error(&yaml);
    assert!(
        error.contains("'pathways' must be a list of pathway names"),
        "{error}"
    );
    assert!(error.contains("read from the register"), "{error}");
}

#[test]
fn no_axes_list_loads_with_no_axes() {
    let profile = profile_from(MINIMAL_PROFILE).unwrap();
    assert!(profile.pathways().is_empty());
    assert!(profile.pathway_bindings().is_empty());
}

// Requirement: Validation entry pathway binding

#[test]
fn partial_binding_names_the_missing_key() {
    let yaml = format!(
        "{MINIMAL_PROFILE}pathways: [phase]\nvalidations:\n  - COVERAGE:\n      pathway: phase\n"
    );
    let error = load_error(&yaml);
    assert!(error.contains("position_attr"), "{error}");
}

#[test]
fn binding_to_an_undeclared_pathway_names_it() {
    let yaml = format!(
        "{MINIMAL_PROFILE}validations:\n  - COVERAGE:\n      pathway: phase\n\
         \x20     position_attr: trigger\n"
    );
    let error = load_error(&yaml);
    assert!(error.contains("undeclared pathway 'phase'"), "{error}");
}

// Requirement: One pathway binding per finding code

#[test]
fn second_binding_for_one_code_is_rejected() {
    let yaml = format!(
        "{MINIMAL_PROFILE}pathways: [phase]\nvalidations:\n\
         \x20 - OBLIGATION_UNBACKED:\n      pathway: phase\n      position_attr: trigger\n\
         \x20 - OBLIGATION_UNBACKED:\n      pathway: phase\n      position_attr: trigger\n"
    );
    let error = load_error(&yaml);
    assert!(error.contains("OBLIGATION_UNBACKED"), "{error}");
    assert!(error.contains("second pathway binding"), "{error}");
}

#[test]
fn repeated_plain_configuration_is_honoured_twice() {
    let yaml = format!(
        "{MINIMAL_PROFILE}validations:\n\
         \x20 - COVERAGE:\n      target_kind: req\n      edge_kind: derives\n\
         \x20 - COVERAGE:\n      target_kind: req\n      edge_kind: verifies\n"
    );
    let profile = profile_from(&yaml).unwrap();
    assert_eq!(profile.validation_configs()["COVERAGE"].len(), 2);
}

#[test]
fn one_binding_alongside_a_repeated_configuration() {
    let yaml = format!(
        "{MINIMAL_PROFILE}pathways: [phase]\nvalidations:\n\
         \x20 - COVERAGE:\n      target_kind: req\n      edge_kind: derives\n\
         \x20 - COVERAGE:\n      target_kind: req\n      edge_kind: verifies\n\
         \x20     pathway: phase\n      position_attr: trigger\n"
    );
    let profile = profile_from(&yaml).unwrap();
    assert_eq!(profile.validation_configs()["COVERAGE"].len(), 2);
    assert_eq!(profile.pathway_bindings()["COVERAGE"].pathway, "phase");
}

// Shapes the loader must reject rather than read past. Each is a `test_profile.py`
// case whose Rust counterpart the scenario tests above do not reach.

// Requirement: Profile version

#[test]
fn a_non_string_version_is_rejected() {
    let yaml = MINIMAL_PROFILE.replace("\"1.0.0\"", "1.0");
    assert!(load_error(&yaml).contains("profile_version must be a string"));
}

#[test]
fn a_non_semver_version_is_rejected() {
    let error = load_error(&MINIMAL_PROFILE.replace("1.0.0", "1.0"));
    assert!(error.contains("is not valid semver"), "{error}");
}

// Requirement: Profile YAML structure

#[test]
fn node_kinds_that_are_not_a_mapping_are_rejected() {
    let yaml = "name: t\nprofile_version: \"1.0.0\"\nnode_kinds: [req]\nedge_kinds: {}\n";
    assert!(load_error(yaml).contains("'node_kinds' must be a mapping"));
}

// Requirement: Edge kind declaration

#[test]
fn an_edge_kind_that_is_not_a_mapping_is_rejected() {
    let yaml = MINIMAL_PROFILE.replace("    allowed: [[req, req]]", "  - req");
    assert!(
        load_error(&yaml).contains("expected a mapping"),
        "{}",
        load_error(&yaml)
    );
}

// Requirement: Profile YAML structure

#[test]
fn validations_declared_as_a_mapping_is_rejected() {
    let yaml = format!("{MINIMAL_PROFILE}validations:\n  COVERAGE:\n    severity: info\n");
    assert!(load_error(&yaml).contains("'validations' must be a list"));
}

// Requirement: Validation configuration schema

#[test]
fn an_invalid_severity_override_names_the_valid_ones() {
    let yaml = format!("{MINIMAL_PROFILE}validations:\n  - ORPHAN_NODE:\n      severity: loud\n");
    let error = load_error(&yaml);
    assert!(error.contains("invalid severity 'loud'"), "{error}");
    assert!(error.contains("error, info, warning"), "{error}");
}

#[test]
fn a_severity_override_on_a_built_in_code_is_recorded() {
    let yaml = format!("{MINIMAL_PROFILE}validations:\n  - ORPHAN_NODE:\n      severity: info\n");
    let profile = profile_from(&yaml).unwrap();
    assert_eq!(
        profile.validation_overrides()["ORPHAN_NODE"],
        lattice_core::types::Severity::Info
    );
}

// Requirement: Typed attribute declarations

#[test]
fn a_non_boolean_required_is_rejected() {
    let error = load_error(&attr_profile("{type: string, required: yes-please}"));
    assert!(error.contains("'required' must be a boolean"), "{error}");
}

#[test]
fn a_non_string_enum_value_is_rejected() {
    let error = load_error(&attr_profile("{type: enum, values: [done, 3]}"));
    assert!(error.contains("enum value must be a string"), "{error}");
}

#[test]
fn a_non_string_attr_type_is_rejected() {
    let error = load_error(&attr_profile("{type: [string]}"));
    assert!(error.contains("'type' must be a string"), "{error}");
}

#[test]
fn an_unknown_attr_type_names_the_valid_ones() {
    let error = load_error(&attr_profile("{type: float}"));
    assert!(error.contains("unknown type 'float'"), "{error}");
    assert!(
        error.contains("bool, date, enum, int, list, string"),
        "{error}"
    );
}

#[test]
fn a_bare_type_name_is_shorthand_for_a_typed_attr() {
    let profile = profile_from(&attr_profile("string")).unwrap();
    let attr = &profile.node_kinds()["req"].attrs["status"];
    assert_eq!(attr.kind, "string");
    assert!(
        !attr.required,
        "shorthand cannot say required, so it is not"
    );
}

// Requirement: Validation configuration schema

#[test]
fn a_complete_coverage_config_is_accepted_and_kept() {
    let yaml = format!(
        "{MINIMAL_PROFILE}validations:\n  - COVERAGE:\n      target_kind: req\n\
         \x20     edge_kind: derives\n      severity: error\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let config = &profile.validation_configs()["COVERAGE"][0];
    assert_eq!(config["target_kind"].as_str(), Some("req"));
    assert_eq!(config["edge_kind"].as_str(), Some("derives"));
}

#[test]
fn coverage_where_is_parsed_and_kept() {
    let yaml = format!(
        "{MINIMAL_PROFILE}validations:\n  - COVERAGE:\n      target_kind: req\n\
         \x20     edge_kind: derives\n      where:\n        status: {{not: deferred}}\n"
    );
    let profile = profile_from(&yaml).unwrap();
    let where_ = &profile.validation_configs()["COVERAGE"][0]["where"];
    assert!(where_.is_mapping());
}

#[test]
fn coverage_where_may_be_absent() {
    let yaml = format!(
        "{MINIMAL_PROFILE}validations:\n  - COVERAGE_DEEP:\n      target_kind: req\n\
         \x20     via: derives\n      evidence: derives\n"
    );
    let profile = profile_from(&yaml).unwrap();
    assert!(!profile.validation_configs()["COVERAGE_DEEP"][0].contains_key("where"));
}

#[test]
fn malformed_coverage_where_is_rejected() {
    let yaml = format!(
        "{MINIMAL_PROFILE}validations:\n  - COVERAGE:\n      target_kind: req\n\
         \x20     edge_kind: derives\n      where:\n        status: deferred\n"
    );
    let error = load_error(&yaml);
    assert!(error.contains("COVERAGE config: 'where.status'"), "{error}");
}

// Requirement: Validation entry pathway binding

#[test]
fn a_binding_carrying_position_attr_alone_is_rejected() {
    let yaml = format!(
        "{MINIMAL_PROFILE}pathways: [phase]\nvalidations:\n  - COVERAGE:\n\
         \x20     position_attr: trigger\n"
    );
    assert!(
        load_error(&yaml).contains("pathway"),
        "the missing key is named"
    );
}

// Requirement: Trace entry ordering

#[test]
fn node_kinds_declaration_order_is_carried_for_the_trace_sort() {
    let yaml = "name: t\nprofile_version: \"1.0.0\"\n\
                node_kinds:\n  zeta:\n    id_pattern: \"^Z-\\\\d+$\"\n\
                \x20 alpha:\n    id_pattern: \"^A-\\\\d+$\"\nedge_kinds: {}\n";
    let profile = profile_from(yaml).unwrap();
    // Keyed for lookup, so the map is sorted; the declared order is on the kind.
    assert_eq!(profile.node_kinds()["zeta"].declared_index, 0);
    assert_eq!(profile.node_kinds()["alpha"].declared_index, 1);
}

// Requirement: Profile inheritance via extends

const PARENT_PROFILE: &str = r#"
name: parent
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\d+$"
    attrs:
      text: {type: string, required: true}
edge_kinds:
  derives:
    allowed: [[req, req]]
"#;

#[test]
fn child_inherits_parent_node_and_edge_kinds() {
    let child = "extends: parent.yaml\nname: child\nprofile_version: \"1.0.0\"\n\
                 node_kinds: {}\nedge_kinds: {}\n";
    let profile =
        profile_from_files(&[("child.yaml", child), ("parent.yaml", PARENT_PROFILE)]).unwrap();
    assert!(profile.node_kinds().contains_key("req"));
    assert!(profile.edge_kinds().contains_key("derives"));
    assert_eq!(profile.name(), "child");
}

#[test]
fn scalar_child_wins() {
    let child = "extends: parent.yaml\nname: child\nprofile_version: \"1.0.0\"\n\
                 node_kinds: {}\nedge_kinds: {}\n";
    let profile =
        profile_from_files(&[("child.yaml", child), ("parent.yaml", PARENT_PROFILE)]).unwrap();
    assert_eq!(profile.name(), "child");
}

#[test]
fn list_replaces_whole() {
    let parent = format!(
        "{PARENT_PROFILE}validations:\n  - ORPHAN_NODE:\n      severity: info\n\
         \x20 - VACANCY:\n      severity: warning\n"
    );
    let child = "extends: parent.yaml\nname: child\nprofile_version: \"1.0.0\"\n\
                 node_kinds: {}\nedge_kinds: {}\n\
                 validations:\n  - ORPHAN_NODE:\n      severity: error\n";
    let profile = profile_from_files(&[("child.yaml", child), ("parent.yaml", &parent)]).unwrap();
    assert_eq!(
        profile.validation_overrides().len(),
        1,
        "child's single validation replaces parent's two"
    );
    assert_eq!(
        profile.validation_overrides()["ORPHAN_NODE"],
        lattice_core::types::Severity::Error
    );
}

#[test]
fn mapping_deep_merge_inherits_parent_attrs() {
    let child = "extends: parent.yaml\nname: child\nprofile_version: \"1.0.0\"\n\
                 node_kinds:\n  req:\n    id_pattern: \"^R-\\\\d+$\"\n\
                 edge_kinds: {}\n";
    let profile =
        profile_from_files(&[("child.yaml", child), ("parent.yaml", PARENT_PROFILE)]).unwrap();
    let req = &profile.node_kinds()["req"];
    assert!(req.id_matches("R-1"), "child's pattern should be used");
    assert!(!req.id_matches("REQ-1"), "parent's pattern should be gone");
    assert!(
        req.attrs.contains_key("text"),
        "parent's attrs should be inherited"
    );
    assert!(req.attrs["text"].required);
}

#[test]
fn child_chunk_line_prefix_overrides_the_parents() {
    // The key is a scalar, so it merges as one: a child that declares its own
    // marker replaces the parent's rather than adding a second cut point.
    let parent = "name: parent\nprofile_version: \"1.0.0\"\n\
                  node_kinds:\n  req:\n    id_pattern: \"^REQ-\\\\d+$\"\n    \
                  text_attrs: [text]\n    text_chunk_line_prefix: \"## Case:\"\n    \
                  attrs:\n      text: {type: string, required: true}\n\
                  edge_kinds:\n  derives:\n    allowed: [[req, req]]\n";
    let child = "extends: parent.yaml\nname: child\nprofile_version: \"1.0.0\"\n\
                 node_kinds:\n  req:\n    \
                 text_chunk_line_prefix: \"@@ Case:\"\n\
                 edge_kinds: {}\n";
    let profile = profile_from_files(&[("child.yaml", child), ("parent.yaml", parent)]).unwrap();
    assert_eq!(
        profile.node_kinds()["req"]
            .text_chunk_line_prefix
            .as_deref(),
        Some("@@ Case:")
    );
}

#[test]
fn child_missing_profile_version_is_rejected() {
    let child = "extends: parent.yaml\nname: child\nnode_kinds: {}\nedge_kinds: {}\n";
    let error = match profile_from_files(&[("child.yaml", child), ("parent.yaml", PARENT_PROFILE)])
    {
        Ok(_) => panic!("should have been rejected"),
        Err(e) => e.0,
    };
    assert!(
        error.contains("profile_version"),
        "error should name the missing key: {error}"
    );
}

#[test]
fn resolved_document_contains_merged_content_and_no_extends() {
    let child = "extends: parent.yaml\nname: child\nprofile_version: \"1.0.0\"\n\
                 node_kinds: {}\nedge_kinds: {}\n";
    let profile =
        profile_from_files(&[("child.yaml", child), ("parent.yaml", PARENT_PROFILE)]).unwrap();
    let doc = lattice_core::profile::resolved_document(&profile).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&doc).unwrap();
    assert!(
        parsed.get("node_kinds").unwrap().get("req").is_some(),
        "merged node kinds should be in the resolved document"
    );
    assert!(
        parsed.get("extends").is_none(),
        "extends should be stripped from the resolved document"
    );
}

// Requirement: Inheritance chain support

#[test]
fn chain_three_levels() {
    let grandparent = r#"
name: gp
profile_version: "1.0.0"
node_kinds:
  alpha:
    id_pattern: "^A-\\d+$"
edge_kinds: {}
"#;
    let parent = "extends: gp.yaml\nname: p\nprofile_version: \"1.0.0\"\n\
                  node_kinds:\n  beta:\n    id_pattern: \"^B-\\\\d+$\"\n\
                  edge_kinds: {}\n";
    let child = "extends: parent.yaml\nname: c\nprofile_version: \"1.0.0\"\n\
                 node_kinds:\n  gamma:\n    id_pattern: \"^G-\\\\d+$\"\n\
                 edge_kinds: {}\n";
    let profile = profile_from_files(&[
        ("child.yaml", child),
        ("parent.yaml", parent),
        ("gp.yaml", grandparent),
    ])
    .unwrap();
    assert!(profile.node_kinds().contains_key("alpha"));
    assert!(profile.node_kinds().contains_key("beta"));
    assert!(profile.node_kinds().contains_key("gamma"));
    assert_eq!(profile.name(), "c");
}

// Requirement: Inheritance cycle detection

#[test]
fn direct_cycle_is_rejected() {
    let a = "extends: b.yaml\nname: a\nprofile_version: \"1.0.0\"\n\
             node_kinds: {}\nedge_kinds: {}\n";
    let b = "extends: a.yaml\nname: b\nprofile_version: \"1.0.0\"\n\
             node_kinds: {}\nedge_kinds: {}\n";
    let error = match profile_from_files(&[("a.yaml", a), ("b.yaml", b)]) {
        Ok(_) => panic!("cycle should have been rejected"),
        Err(e) => e.0,
    };
    assert!(
        error.contains("cycle"),
        "error should mention cycle: {error}"
    );
}

#[test]
fn self_extends_is_rejected() {
    let a = "extends: a.yaml\nname: a\nprofile_version: \"1.0.0\"\n\
             node_kinds: {}\nedge_kinds: {}\n";
    let error = match profile_from_files(&[("a.yaml", a)]) {
        Ok(_) => panic!("self-extends should have been rejected"),
        Err(e) => e.0,
    };
    assert!(
        error.contains("cycle"),
        "error should mention cycle: {error}"
    );
}

// Requirement: Extends key validation

#[test]
fn non_string_extends_is_rejected() {
    let child = "extends: [a.yaml, b.yaml]\nname: c\nprofile_version: \"1.0.0\"\n\
                 node_kinds: {}\nedge_kinds: {}\n";
    let error = match profile_from_files(&[("child.yaml", child)]) {
        Ok(_) => panic!("list extends should have been rejected"),
        Err(e) => e.0,
    };
    assert!(
        error.contains("'extends' must be a string"),
        "error should say extends must be string: {error}"
    );
}

#[test]
fn missing_parent_file_is_rejected() {
    let child = "extends: nonexistent.yaml\nname: c\nprofile_version: \"1.0.0\"\n\
                 node_kinds: {}\nedge_kinds: {}\n";
    let error = match profile_from_files(&[("child.yaml", child)]) {
        Ok(_) => panic!("missing parent should have been rejected"),
        Err(e) => e.0,
    };
    assert!(
        error.contains("nonexistent.yaml"),
        "error should name the missing file: {error}"
    );
}

// Requirement: Node-kind declaration order across merge

#[test]
fn parent_first_declared_index_ordering() {
    let parent = r#"
name: p
profile_version: "1.0.0"
node_kinds:
  alpha:
    id_pattern: "^A-\\d+$"
  beta:
    id_pattern: "^B-\\d+$"
  delta:
    id_pattern: "^D-\\d+$"
edge_kinds: {}
"#;
    let child = "extends: parent.yaml\nname: c\nprofile_version: \"1.0.0\"\n\
                 node_kinds:\n  beta:\n    id_pattern: \"^BB-\\\\d+$\"\n\
                 \x20 gamma:\n    id_pattern: \"^G-\\\\d+$\"\n\
                 edge_kinds: {}\n";
    let profile = profile_from_files(&[("child.yaml", child), ("parent.yaml", parent)]).unwrap();
    assert_eq!(profile.node_kinds()["alpha"].declared_index, 0);
    assert_eq!(
        profile.node_kinds()["beta"].declared_index,
        1,
        "override keeps parent position"
    );
    assert_eq!(profile.node_kinds()["delta"].declared_index, 2);
    assert_eq!(profile.node_kinds()["gamma"].declared_index, 3);
}

// Requirement: Profile inheritance via extends

#[test]
fn non_mapping_parent_is_rejected() {
    let parent = "- just\n- a\n- list\n";
    let child = "extends: parent.yaml\nname: c\nprofile_version: \"1.0.0\"\n\
                 node_kinds: {}\nedge_kinds: {}\n";
    let error = match profile_from_files(&[("child.yaml", child), ("parent.yaml", parent)]) {
        Ok(_) => panic!("non-mapping parent should have been rejected"),
        Err(e) => e.0,
    };
    assert!(
        error.contains("not a YAML mapping"),
        "error should say parent is not a mapping: {error}"
    );
}

// Requirement: Node kind orphan policy flag

#[test]
fn orphan_ok_flag_loads() {
    let profile = profile_from(
        r#"
name: t
profile_version: "1.0.0"
node_kinds:
  test:
    id_pattern: "^T-\\d+$"
    orphan_ok: true
  req:
    id_pattern: "^REQ-\\d+$"
edge_kinds: {}
"#,
    )
    .unwrap();
    assert!(profile.node_kinds()["test"].orphan_ok);
    assert!(
        !profile.node_kinds()["req"].orphan_ok,
        "absence means false"
    );
}

#[test]
fn orphan_ok_of_the_wrong_type_is_rejected_naming_the_kind() {
    let error = profile_from(
        r#"
name: t
profile_version: "1.0.0"
node_kinds:
  test:
    id_pattern: "^T-\\d+$"
    orphan_ok: "yes"
edge_kinds: {}
"#,
    )
    .unwrap_err();
    assert!(error.0.contains("node kind 'test'"), "{}", error.0);
    assert!(error.0.contains("got string"), "{}", error.0);
}

#[test]
fn child_can_unset_an_inherited_chunk_line_prefix_with_null() {
    // The only way to disable an inherited marker: an empty string is rejected,
    // so if explicit null did not reach the loader as null, a child could not
    // opt out of its parent's chunking at all.
    let parent = "name: parent\nprofile_version: \"1.0.0\"\n\
                  node_kinds:\n  req:\n    id_pattern: \"^REQ-\\\\d+$\"\n    \
                  text_attrs: [text]\n    text_chunk_line_prefix: \"## Case:\"\n    \
                  attrs:\n      text: {type: string, required: true}\n\
                  edge_kinds:\n  derives:\n    allowed: [[req, req]]\n";
    let child = "extends: parent.yaml\nname: child\nprofile_version: \"1.0.0\"\n\
                 node_kinds:\n  req:\n    text_chunk_line_prefix: null\n\
                 edge_kinds: {}\n";
    let profile = profile_from_files(&[("child.yaml", child), ("parent.yaml", parent)]).unwrap();
    assert_eq!(profile.node_kinds()["req"].text_chunk_line_prefix, None);
}

#[test]
fn chunk_line_prefix_containing_a_carriage_return_is_rejected() {
    // Rejected alongside the line feed: a lone CR is a line break to some
    // producers, so a prefix carrying one could never match a line the way the
    // consumer compares them.
    let yaml = "name: t\nprofile_version: \"1.0.0\"\nnode_kinds:\n  req:\n    \
                id_pattern: \"^REQ-\\\\d+$\"\n    text_attrs: [title]\n    \
                text_chunk_line_prefix: \"a\\rb\"\n    attrs:\n      \
                title: {type: string, required: true}\nedge_kinds: {}\n";
    let error = load_error(yaml);
    assert!(error.contains("text_chunk_line_prefix"), "{error}");
}
