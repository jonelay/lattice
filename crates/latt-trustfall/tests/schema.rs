//! Every shipped profile's schema parses, and names round-trip.

mod common;

use latt_trustfall::mangle::decode;
use latt_trustfall::schema::{attr_field, out_field, schema_text, type_name};
use trustfall_core::schema::Schema;

// Requirement: development-verification/Validators are cross-checked against an independent engine
#[test]
fn schema_every_shipped_profile_parses() {
    for (name, profile) in common::shipped_profiles() {
        let text = schema_text(&profile).unwrap_or_else(|e| panic!("{name}: {e}"));
        let schema = Schema::parse(&text).unwrap_or_else(|e| panic!("{name}:\n{text}\n{e}"));
        let implementors: std::collections::BTreeSet<&str> =
            schema.subtypes("Node").expect("Node is a type").collect();
        for kind in profile.node_kinds().keys() {
            assert!(
                implementors.contains(type_name(kind).as_str()),
                "{name}: {kind} does not implement Node"
            );
        }
    }
}

#[test]
fn schema_field_names_decode_to_their_lattice_names() {
    assert_eq!(
        decode(type_name("spec-goal").strip_prefix("K_").unwrap()).unwrap(),
        "spec-goal"
    );
    assert_eq!(
        decode(attr_field("3d_model").strip_prefix("a_").unwrap()).unwrap(),
        "3d_model"
    );
    assert_eq!(
        decode(out_field("reqs:traces").strip_prefix("e_").unwrap()).unwrap(),
        "reqs:traces"
    );
}
