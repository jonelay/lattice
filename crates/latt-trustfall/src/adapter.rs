//! Serve a `LatticeGraph` to Trustfall.
//!
//! Built on `latt-core`'s public items alone — `iter_nodes`, `iter_edges`,
//! `node`, `has_node` and the supported `AdjacencyIndex` — so the cross-check
//! also tests whether the embedding API is enough for a foreign engine.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use latt_core::graph::{Edge, LatticeGraph, Node};
use latt_core::profile::{AttrSchema, Profile};
use latt_core::query::AdjacencyIndex;
use serde_json::Value;
use trustfall_core::interpreter::helpers::{
    resolve_coercion_with, resolve_neighbors_with, resolve_property_with,
};
use trustfall_core::interpreter::{
    Adapter, AsVertex, ContextIterator, ContextOutcomeIterator, ResolveEdgeInfo, ResolveInfo,
    VertexIterator,
};
use trustfall_core::ir::{EdgeParameters, FieldValue};

use crate::mangle::decode;
use crate::schema::type_name;

/// A vertex: one of the graph's nodes or one of its edges as a record.
#[derive(Clone, Debug)]
pub enum Vertex<'g> {
    Node(&'g Node),
    Edge(&'g Edge),
}

/// Which end of its edges a field follows.
#[derive(Clone, Copy, Debug)]
enum Way {
    Out,
    In,
}

/// The adapter over one graph under one profile.
pub struct LatticeAdapter<'g> {
    graph: &'g LatticeGraph,
    index: AdjacencyIndex<'g>,
    /// Declared attr schemas per kind, for typed property values.
    attrs: HashMap<&'g str, &'g BTreeMap<String, AttrSchema>>,
    /// Edge field name → (direction, edge kind), for every declared edge kind.
    edge_fields: HashMap<String, (Way, String)>,
}

impl<'g> LatticeAdapter<'g> {
    #[must_use]
    pub fn new(graph: &'g LatticeGraph, profile: &'g Profile) -> Self {
        let mut edge_fields = HashMap::new();
        for edge in profile.edge_kinds().keys() {
            edge_fields.insert(crate::schema::out_field(edge), (Way::Out, edge.clone()));
            edge_fields.insert(crate::schema::in_field(edge), (Way::In, edge.clone()));
        }
        Self {
            graph,
            index: AdjacencyIndex::build(graph),
            attrs: profile
                .node_kinds()
                .iter()
                .map(|(kind, declared)| (kind.as_str(), &declared.attrs))
                .collect(),
            edge_fields,
        }
    }

    fn all_nodes(&self) -> VertexIterator<'g, Vertex<'g>> {
        Box::new(self.graph.iter_nodes().map(Vertex::Node))
    }
}

/// A JSON value as the declared type, or null when it is not of that type: the
/// schema promised the type, and lattice reports the value as `ATTR_TYPE`.
pub(crate) fn typed(value: &Value, kind: &str, items: Option<&str>) -> FieldValue {
    match (kind, value) {
        ("int", Value::Number(n)) => n
            .as_i64()
            .map(FieldValue::Int64)
            .or_else(|| n.as_u64().map(FieldValue::Uint64))
            .unwrap_or(FieldValue::Null),
        ("bool", Value::Bool(b)) => FieldValue::Boolean(*b),
        ("list", Value::Array(items_)) => {
            let item_kind = items.unwrap_or("string");
            FieldValue::List(
                items_
                    .iter()
                    .map(|item| typed(item, item_kind, None))
                    .collect::<Vec<_>>()
                    .into(),
            )
        }
        ("string" | "date" | "enum", Value::String(s)) => FieldValue::String(s.as_str().into()),
        _ => FieldValue::Null,
    }
}

fn string(text: &str) -> FieldValue {
    FieldValue::String(text.into())
}

impl<'g> Adapter<'g> for LatticeAdapter<'g> {
    type Vertex = Vertex<'g>;

    fn resolve_starting_vertices(
        &self,
        edge_name: &Arc<str>,
        parameters: &EdgeParameters,
        _resolve_info: &ResolveInfo,
    ) -> VertexIterator<'g, Self::Vertex> {
        match edge_name.as_ref() {
            "Nodes" => self.all_nodes(),
            "NodesOfKind" => {
                let kind = parameters
                    .get("kind")
                    .and_then(FieldValue::as_str)
                    .map(str::to_owned)
                    .unwrap_or_default();
                Box::new(
                    self.graph
                        .iter_nodes()
                        .filter(move |node| node.kind == kind)
                        .map(Vertex::Node),
                )
            }
            "Edges" => Box::new(self.graph.iter_edges().map(Vertex::Edge)),
            other => panic!("unknown entry point {other}"),
        }
    }

    fn resolve_property<V: AsVertex<Self::Vertex> + 'g>(
        &self,
        contexts: ContextIterator<'g, V>,
        type_name: &Arc<str>,
        property_name: &Arc<str>,
        _resolve_info: &ResolveInfo,
    ) -> ContextOutcomeIterator<'g, V, FieldValue> {
        let property = property_name.to_string();
        if type_name.as_ref() == "EdgeRecord" {
            let graph = self.graph;
            return resolve_property_with(contexts, move |vertex| {
                let Vertex::Edge(edge) = vertex else {
                    return FieldValue::Null;
                };
                match property.as_str() {
                    "__typename" => string("EdgeRecord"),
                    "kind" => string(&edge.kind),
                    "source_id" => string(&edge.src),
                    "target_id" => string(&edge.tgt),
                    "resolved" => {
                        FieldValue::Boolean(graph.has_node(&edge.src) && graph.has_node(&edge.tgt))
                    }
                    "file" => string(edge.provenance.file()),
                    "line" => FieldValue::Int64(
                        edge.provenance
                            .line()
                            .map_or(0, |line| i64::from(line.get())),
                    ),
                    "attrs_json" => string(&Value::Object(edge.attrs.clone()).to_string()),
                    other => panic!("EdgeRecord has no property {other}"),
                }
            });
        }
        let attrs = self.attrs.clone();
        resolve_property_with(contexts, move |vertex| {
            let Vertex::Node(node) = vertex else {
                return FieldValue::Null;
            };
            match property.as_str() {
                "__typename" => string(&crate::schema::type_name(&node.kind)),
                "id" => string(&node.id),
                "kind" => string(&node.kind),
                "file" => string(node.provenance.file()),
                "line" => FieldValue::Int64(
                    node.provenance
                        .line()
                        .map_or(0, |line| i64::from(line.get())),
                ),
                "attrs_json" => string(&Value::Object(node.attrs.clone()).to_string()),
                other => {
                    let attr = other
                        .strip_prefix("a_")
                        .and_then(decode)
                        .unwrap_or_else(|| panic!("Node has no property {other}"));
                    let Some(schema) = attrs.get(node.kind.as_str()).and_then(|a| a.get(&attr))
                    else {
                        return FieldValue::Null;
                    };
                    node.attrs.get(&attr).map_or(FieldValue::Null, |value| {
                        typed(
                            value,
                            schema.kind.as_str(),
                            schema.items.map(|i| i.as_str()),
                        )
                    })
                }
            }
        })
    }

    fn resolve_neighbors<V: AsVertex<Self::Vertex> + 'g>(
        &self,
        contexts: ContextIterator<'g, V>,
        type_name: &Arc<str>,
        edge_name: &Arc<str>,
        _parameters: &EdgeParameters,
        _resolve_info: &ResolveEdgeInfo,
    ) -> ContextOutcomeIterator<'g, V, VertexIterator<'g, Self::Vertex>> {
        let graph = self.graph;
        let index = self.index.clone();
        let edge = edge_name.to_string();
        if type_name.as_ref() == "EdgeRecord" {
            return resolve_neighbors_with(contexts, move |vertex| {
                let Vertex::Edge(record) = vertex else {
                    return Box::new(std::iter::empty());
                };
                let id = match edge.as_str() {
                    "source" => &record.src,
                    "target" => &record.tgt,
                    other => panic!("EdgeRecord has no edge {other}"),
                };
                Box::new(graph.node(id).map(Vertex::Node).into_iter())
            });
        }
        let field = self.edge_fields.get(&edge).cloned();
        resolve_neighbors_with(contexts, move |vertex| {
            let Vertex::Node(node) = vertex else {
                return Box::new(std::iter::empty());
            };
            match (edge.as_str(), &field) {
                ("out_edges", _) => Box::new(
                    index
                        .edges_from(&node.id)
                        .iter()
                        .map(|e| Vertex::Edge(e))
                        .collect::<Vec<_>>()
                        .into_iter(),
                ),
                ("in_edges", _) => Box::new(
                    index
                        .edges_to(&node.id)
                        .iter()
                        .map(|e| Vertex::Edge(e))
                        .collect::<Vec<_>>()
                        .into_iter(),
                ),
                (_, Some((Way::Out, kind))) => {
                    let kind = kind.clone();
                    Box::new(
                        index
                            .edges_from(&node.id)
                            .iter()
                            .filter(move |e| e.kind == kind)
                            .filter_map(|e| graph.node(&e.tgt))
                            .map(Vertex::Node)
                            .collect::<Vec<_>>()
                            .into_iter(),
                    )
                }
                (_, Some((Way::In, kind))) => {
                    let kind = kind.clone();
                    Box::new(
                        index
                            .edges_to(&node.id)
                            .iter()
                            .filter(move |e| e.kind == kind)
                            .filter_map(|e| graph.node(&e.src))
                            .map(Vertex::Node)
                            .collect::<Vec<_>>()
                            .into_iter(),
                    )
                }
                (other, None) => panic!("Node has no edge {other}"),
            }
        })
    }

    fn resolve_coercion<V: AsVertex<Self::Vertex> + 'g>(
        &self,
        contexts: ContextIterator<'g, V>,
        _type_name: &Arc<str>,
        coerce_to_type: &Arc<str>,
        _resolve_info: &ResolveInfo,
    ) -> ContextOutcomeIterator<'g, V, bool> {
        let wanted = coerce_to_type.to_string();
        resolve_coercion_with(contexts, move |vertex| match vertex {
            Vertex::Node(node) => type_name(&node.kind) == wanted,
            Vertex::Edge(_) => wanted == "EdgeRecord",
        })
    }
}
