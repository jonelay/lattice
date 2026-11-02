//! A resolved profile as a Trustfall schema.
//!
//! Every node kind is a vertex type implementing the `Node` interface, with
//! its declared attrs as typed properties (`a_<attr>`) so Trustfall's own
//! comparisons do the work: an `int` attr is an `Int`, not a string to match.
//! Every edge kind is an outgoing edge field `e_<edge>` on its allowed source
//! kinds and `in_<edge>` on its allowed target kinds. Every edge is also an
//! `EdgeRecord` vertex with `resolved`, so a dangling edge, which Trustfall's
//! graph model cannot otherwise hold, keeps a spelling.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use latt_core::profile::{AttrSchema, NodeKind, Profile};

use crate::mangle::encode;

/// The vertex type name of a node kind.
#[must_use]
pub fn type_name(kind: &str) -> String {
    format!("K_{}", encode(kind))
}

/// The property name of an attr.
#[must_use]
pub fn attr_field(attr: &str) -> String {
    format!("a_{}", encode(attr))
}

/// The outgoing edge field of an edge kind.
#[must_use]
pub fn out_field(edge_kind: &str) -> String {
    format!("e_{}", encode(edge_kind))
}

/// The incoming edge field of an edge kind.
#[must_use]
pub fn in_field(edge_kind: &str) -> String {
    format!("in_{}", encode(edge_kind))
}

/// The fields every `Node` carries, interface and implementations alike.
const NODE_FIELDS: &str = "    id: String!\n    kind: String!\n    file: String!\n    line: Int!\n    attrs_json: String!\n    out_edges: [EdgeRecord!]!\n    in_edges: [EdgeRecord!]!\n";

/// The GraphQL type of a declared attr: lattice's `int` is `Int`, `bool` is
/// `Boolean`, `list` is a list of its item type, and `string`, `date` and `enum`
/// are `String`.
#[must_use]
pub fn attr_type(schema: &AttrSchema) -> String {
    scalar_type(
        schema.kind.as_str(),
        schema.items.map(|items| items.as_str()),
    )
}

fn scalar_type(kind: &str, items: Option<&str>) -> String {
    match kind {
        "int" => "Int".into(),
        "bool" => "Boolean".into(),
        "list" => format!("[{}]", scalar_type(items.unwrap_or("string"), None)),
        _ => "String".into(),
    }
}

/// Render the schema text for `profile`.
///
/// # Errors
///
/// Two kinds or two attrs of one kind whose encoded names coincide, naming both.
pub fn schema_text(profile: &Profile) -> Result<String, String> {
    let mut text = String::from(trustfall_core::schema::Schema::ALL_DIRECTIVE_DEFINITIONS);
    text.push_str(
        "\nschema {\n    query: RootSchemaQuery\n}\n\n\
         type RootSchemaQuery {\n    Nodes: [Node!]!\n    NodesOfKind(kind: String!): [Node!]!\n    Edges: [EdgeRecord!]!\n}\n\n\
         interface Node {\n",
    );
    text.push_str(NODE_FIELDS);
    text.push_str(
        "}\n\ntype EdgeRecord {\n    kind: String!\n    source_id: String!\n    target_id: String!\n    \
         resolved: Boolean!\n    file: String!\n    line: Int!\n    attrs_json: String!\n    \
         source: Node\n    target: Node\n}\n",
    );

    let mut types: BTreeMap<String, &str> = BTreeMap::new();
    for kind in profile.node_kinds().keys() {
        if let Some(other) = types.insert(type_name(kind), kind) {
            return Err(format!(
                "kinds '{other}' and '{kind}' both encode as '{}'",
                type_name(kind)
            ));
        }
    }
    for (kind, declared) in profile.node_kinds() {
        text.push('\n');
        write_kind(&mut text, profile, kind, declared)?;
    }
    Ok(text)
}

fn write_kind(
    text: &mut String,
    profile: &Profile,
    kind: &str,
    declared: &NodeKind,
) -> Result<(), String> {
    let _ = writeln!(text, "type {} implements Node {{", type_name(kind));
    text.push_str(NODE_FIELDS);
    let mut fields: BTreeMap<String, &str> = BTreeMap::new();
    for (attr, schema) in &declared.attrs {
        if let Some(other) = fields.insert(attr_field(attr), attr) {
            return Err(format!(
                "kind '{kind}': attrs '{other}' and '{attr}' both encode as '{}'",
                attr_field(attr)
            ));
        }
        let _ = writeln!(text, "    {}: {}", attr_field(attr), attr_type(schema));
    }
    for (edge, declared_edge) in profile.edge_kinds() {
        if declared_edge.allowed.iter().any(|(src, _)| src == kind) {
            let _ = writeln!(text, "    {}: [Node!]!", out_field(edge));
        }
        if declared_edge.allowed.iter().any(|(_, tgt)| tgt == kind) {
            let _ = writeln!(text, "    {}: [Node!]!", in_field(edge));
        }
    }
    text.push_str("}\n");
    Ok(())
}
