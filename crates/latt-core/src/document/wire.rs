//! Typed reading of an interface document straight from its text.
//!
//! The `Value` path in the parent module holds the whole document as a JSON tree
//! before any of it reaches the graph; for a large register that tree, not the
//! graph, sets peak memory. This path deserializes each entry into its own type
//! and moves it into the graph. It accepts only documents the `Value` path
//! accepts and builds the same graph from them; anything it does not accept, or
//! would have to refuse, it hands back, and the `Value` path reads the text again
//! to say exactly what is wrong. So every error keeps the `Value` path's message.

use std::fmt;
use std::marker::PhantomData;

use latt_adapter::SortedMap;
use serde::Deserialize;
use serde::de::value::MapAccessDeserializer;
use serde::de::{Deserializer, Error as _, MapAccess, Visitor};
use serde_json::{Map, Value};

use super::{
    NodeEntry, RejectedEntry, SUPPORTED_INTERFACE_VERSIONS, place_edge, place_node, report_rejected,
};
use crate::graph::{EdgeSpec, LatticeGraph};
use crate::types::{Issue, Provenance, Severity};

/// A value read only from a JSON object. A derived struct also reads a JSON
/// array, field by field in order; the `Value` path refuses that shape.
struct MapOnly<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for MapOnly<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Only<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Only<T> {
            type Value = T;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map))
            }
        }
        deserializer.deserialize_map(Only(PhantomData)).map(MapOnly)
    }
}

/// Read an optional key that is present; with `#[serde(default)]` an absent one
/// is `None`, and an explicit `null` is refused, as the `Value` path refuses it.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

/// A severity in its wire spelling, parsed where it is read.
fn severity<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Severity, D::Error> {
    let text = String::deserialize(deserializer)?;
    Severity::parse(&text).ok_or_else(|| D::Error::custom("unknown severity"))
}

/// Proof that the document declares an interface version this core supports;
/// holding one, nothing re-checks the version.
struct SupportedVersion;

impl<'de> Deserialize<'de> for SupportedVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = String::deserialize(deserializer)?;
        if SUPPORTED_INTERFACE_VERSIONS.contains(&version.as_str()) {
            Ok(SupportedVersion)
        } else {
            Err(D::Error::custom("unsupported interface version"))
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    /// Under either name. Both keys at once is a repeated field here, so that
    /// document goes to the `Value` path, which reads `interface_version`.
    #[serde(alias = "contract_version")]
    interface_version: SupportedVersion,
    #[serde(default)]
    nodes: Vec<MapOnly<Node>>,
    #[serde(default)]
    edges: Vec<MapOnly<Edge>>,
    #[serde(default)]
    pathways: Vec<MapOnly<Pathway>>,
    #[serde(default)]
    findings: Vec<MapOnly<Finding>>,
    #[serde(default)]
    issues: Vec<MapOnly<Finding>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    id: String,
    kind: String,
    attrs: Map<String, Value>,
    provenance: MapOnly<Location>,
    #[serde(default)]
    attr_provenance: SortedMap<String, MapOnly<Location>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edge {
    src: String,
    tgt: String,
    kind: String,
    #[serde(default)]
    attrs: Map<String, Value>,
    provenance: MapOnly<Location>,
    #[serde(default, deserialize_with = "present")]
    declared_by: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pathway {
    name: String,
    order: Vec<String>,
    current: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finding {
    #[serde(deserialize_with = "severity")]
    severity: Severity,
    code: String,
    message: String,
    provenance: MapOnly<Location>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Location {
    file: String,
    line: u32,
}

impl From<MapOnly<Location>> for Provenance {
    fn from(MapOnly(location): MapOnly<Location>) -> Self {
        Provenance::new(location.file, location.line)
    }
}

/// Build the graph from `text`, or `None` for the `Value` path to read it.
pub(super) fn ingest(text: &str) -> Option<LatticeGraph> {
    let MapOnly(document) = serde_json::from_str::<MapOnly<Document>>(text).ok()?;
    document.ingest()
}

impl Document {
    /// Build the graph in the `Value` path's order: version, findings, pathways,
    /// nodes, edges, then the rejected entries. Return `None` where that path
    /// would refuse the document.
    fn ingest(self) -> Option<LatticeGraph> {
        // The version was checked when it was read.
        let Document {
            interface_version: SupportedVersion,
            nodes,
            edges,
            pathways,
            findings,
            issues,
        } = self;
        let findings = match (findings.is_empty(), issues.is_empty()) {
            (_, true) => findings,
            (true, false) => issues,
            (false, false) => return None,
        };
        let mut graph = LatticeGraph::new();
        for MapOnly(finding) in findings {
            let mut issue = Issue::new(
                finding.severity,
                finding.code,
                finding.message,
                Provenance::from(finding.provenance),
                finding.node_id,
            );
            issue.state = finding.state;
            graph.add_issue(issue);
        }
        for MapOnly(pathway) in pathways {
            graph
                .set_pathway(pathway.name, pathway.order, pathway.current)
                .ok()?;
        }
        let node_count = nodes.len();
        let mut rejected: Vec<RejectedEntry> = Vec::new();
        for (index, MapOnly(node)) in nodes.into_iter().enumerate() {
            // Sorted and unique already, so collecting the converted entries keeps
            // their order and allocates once.
            let mut located = Vec::with_capacity(node.attr_provenance.len());
            for (attr, location) in node.attr_provenance {
                if !node.attrs.contains_key(&attr) {
                    return None;
                }
                located.push((attr, Provenance::from(location)));
            }
            let located: SortedMap<String, Provenance> = located.into_iter().collect();
            place_node(
                &mut graph,
                &mut rejected,
                index,
                NodeEntry {
                    id: node.id,
                    kind: node.kind,
                    attrs: node.attrs,
                    provenance: node.provenance.into(),
                    attr_provenance: located,
                },
            );
        }
        for MapOnly(edge) in edges {
            let declared_by = match edge.declared_by {
                Some(index) if index >= node_count => return None,
                declared_by => declared_by,
            };
            let spec = EdgeSpec {
                src: edge.src,
                tgt: edge.tgt,
                kind: edge.kind,
                attrs: edge.attrs,
            };
            place_edge(
                &mut graph,
                &mut rejected,
                declared_by,
                spec,
                edge.provenance.into(),
            );
        }
        report_rejected(&mut graph, rejected);
        Some(graph)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::super::{ingest_document, ingest_text, parse_document};

    /// The law: a graph from the typed path is the graph the `Value` path builds,
    /// and the typed path builds none where the `Value` path refuses. Whichever
    /// path reads it, ingesting the text answers as the `Value` path does: the
    /// same graph, or the same error message.
    fn agrees(text: &str) -> Option<bool> {
        let fast = super::ingest(text);
        let slow = parse_document(text).and_then(ingest_document);
        match (ingest_text(text), &slow) {
            (Ok(graph), Ok(expected)) => assert_eq!(&graph, expected, "{text}"),
            (Err(error), Err(expected)) => {
                assert_eq!(error.message(), expected.message(), "{text}");
            }
            (got, expected) => panic!("ingest gave {got:?}, the Value path {expected:?}: {text}"),
        }
        match (fast, slow) {
            (Some(fast), Ok(slow)) => {
                assert_eq!(fast, slow, "{text}");
                Some(true)
            }
            (Some(_), Err(error)) => panic!("typed path took what is refused ({error}): {text}"),
            (None, _) => Some(false),
        }
    }

    fn base() -> Value {
        let at = |line| json!({"file": "r.toml", "line": line});
        json!({
            "interface_version": "1.2",
            "findings": [{"severity": "warning", "code": "X", "message": "m",
                          "provenance": at(1), "node_id": null, "state": "open"}],
            "pathways": [{"name": "ms", "order": ["M0", "M1"], "current": "M1"}],
            "nodes": [
                {"id": "a", "kind": "req", "attrs": {"t": "x", "n": 2},
                 "provenance": at(2), "attr_provenance": {"t": at(3)}},
                {"id": "b", "kind": "req", "attrs": {}, "provenance": at(4)},
                {"id": "a", "kind": "req", "attrs": {}, "provenance": at(5)}
            ],
            "edges": [
                {"src": "a", "tgt": "b", "kind": "k", "provenance": at(2), "declared_by": 0},
                {"src": "a", "tgt": "b", "kind": "k", "provenance": at(5), "declared_by": 2},
                {"src": "a", "tgt": "x", "kind": "k", "attrs": {"w": 1}, "provenance": at(6)}
            ]
        })
    }

    #[test]
    fn a_well_formed_document_is_read_typed_and_reads_the_same() {
        assert_eq!(agrees(&base().to_string()), Some(true));
    }

    /// Each spelling the `Value` path accepts for a typed field is read typed, not
    /// deferred: a deferral builds the same graph, so only this test notices one.
    #[test]
    fn every_accepted_spelling_of_a_typed_field_is_read_typed() {
        let mut texts = vec![
            base()
                .to_string()
                .replace("interface_version", "contract_version"),
        ];
        for version in super::SUPPORTED_INTERFACE_VERSIONS {
            let mut document = base();
            set(&mut document, "/interface_version", json!(version));
            texts.push(document.to_string());
        }
        for severity in ["error", "warning", "info", "hint"] {
            let mut document = base();
            set(&mut document, "/findings/0/severity", json!(severity));
            texts.push(document.to_string());
        }
        for text in texts {
            assert_eq!(agrees(&text), Some(true), "{text}");
        }
    }

    /// One case per refusal the `Value` path makes, and per shape it reads that a
    /// derived type would read differently.
    #[test]
    fn every_refusal_and_odd_shape_is_left_to_the_value_path() {
        let cases: Vec<(&str, Value)> = vec![
            (
                "/nodes/0",
                json!(["a", "req", {}, {"file": "r", "line": 1}]),
            ),
            ("/nodes/0/provenance", json!(["r.toml", 2])),
            ("/nodes", Value::Null),
            ("/nodes/0/attrs", Value::Null),
            ("/nodes/0/attr_provenance", Value::Null),
            (
                "/nodes/0/attr_provenance",
                json!({"missing": {"file": "r", "line": 1}}),
            ),
            ("/nodes/0/provenance/line", json!(-1)),
            ("/nodes/0/provenance/line", json!(2.0)),
            ("/nodes/0/provenance/line", json!(u64::from(u32::MAX) + 1)),
            ("/nodes/0/provenance/extra", json!(1)),
            ("/nodes/0/extra", json!(1)),
            ("/edges/0/declared_by", Value::Null),
            ("/edges/0/declared_by", json!(-1)),
            ("/edges/0/declared_by", json!(1.5)),
            ("/edges/0/declared_by", json!(3)),
            ("/edges/2/attrs", Value::Null),
            ("/findings/0/severity", json!("loud")),
            ("/findings/0/node_id", json!(3)),
            (
                "/issues",
                json!([{"severity": "error", "code": "Y", "message": "m",
                                "provenance": {"file": "r", "line": 1}}]),
            ),
            ("/pathways/0/order/1", json!(1)),
            ("/pathways/0/current", json!("M9")),
            ("/interface_version", Value::Null),
            ("/interface_version", json!("9.9")),
            ("/interface_version", json!(1.2)),
            ("/contract_version", json!("1.2")),
            ("/contract_version", Value::Null),
            ("/extra", json!(1)),
        ];
        for (pointer, value) in cases {
            let mut document = base();
            set(&mut document, pointer, value);
            agrees(&document.to_string());
        }
        // Read by the Value path, refused or deferred here: a whole-document array,
        // only the old version key, and a repeated key in one object.
        for text in [
            "[]".to_owned(),
            base()
                .to_string()
                .replace("interface_version", "contract_version"),
            base()
                .to_string()
                .replacen(r#""kind":"req","#, r#""kind":"x","kind":"req","#, 1),
        ] {
            agrees(&text);
        }
    }

    /// Every position in the document, each replaced by every odd value or
    /// removed: the typed path never takes what the `Value` path refuses, nor
    /// reads it differently.
    #[test]
    fn every_position_replaced_or_removed_agrees() {
        fn pointers(value: &Value, at: String, out: &mut Vec<String>) {
            match value {
                Value::Object(object) => {
                    for (key, inner) in object {
                        let inner_at = format!("{at}/{key}");
                        out.push(inner_at.clone());
                        pointers(inner, inner_at, out);
                    }
                }
                Value::Array(items) => {
                    for (index, inner) in items.iter().enumerate() {
                        let inner_at = format!("{at}/{index}");
                        out.push(inner_at.clone());
                        pointers(inner, inner_at, out);
                    }
                }
                _ => {}
            }
        }
        let odd: Vec<Value> = vec![
            Value::Null,
            json!(true),
            json!(0),
            json!(-1),
            json!(1.0),
            json!(u64::MAX),
            json!(i64::MIN),
            json!(""),
            json!("1.2"),
            json!("warning"),
            json!([]),
            json!(["a"]),
            json!({}),
            json!({"file": "r", "line": 1}),
        ];
        let mut all = Vec::new();
        pointers(&base(), String::new(), &mut all);
        let mut typed = 0;
        for pointer in &all {
            for value in &odd {
                let mut document = base();
                set(&mut document, pointer, value.clone());
                typed += usize::from(agrees(&document.to_string()) == Some(true));
            }
            let mut document = base();
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            match document.pointer_mut(parent).unwrap() {
                Value::Object(object) => {
                    object.remove(key);
                }
                Value::Array(items) => {
                    items.remove(key.parse::<usize>().unwrap());
                }
                _ => unreachable!("a pointer's parent is a container"),
            }
            typed += usize::from(agrees(&document.to_string()) == Some(true));
        }
        // Raw-text spellings the `Value` tree cannot carry.
        for text in [
            r#"{"interface_version":"1.2","nodes":[{"id":"a","kind":"k","attrs":{"t":1,"t":2},"provenance":{"file":"r","line":-0}}]}"#,
            r#"{"interface_version":"1.2","nodes":[{"id":"a","kind":"k","attrs":{"t":1,"t":2},"provenance":{"file":"r","line":1e0}}]}"#,
            r#"{"interface_version":"1.2","nodes":[{"id":"a","kind":"k","attrs":{"t":1,"t":2},"attr_provenance":{"t":{"file":"r","line":"x"},"t":{"file":"r","line":2}},"provenance":{"file":"r","line":1}}]}"#,
            r#"{"interface_version":"1.2","nodes":[{"id":"a","kind":"k","attrs":{"t":1,"t":2},"attr_provenance":{"t":{"file":"r","line":1},"t":{"file":"r","line":2}},"provenance":{"file":"r","line":1}}]}"#,
            r#"{"interface_version":"1.2","nodes":[{"id":"a","kind":"k","attrs":{},"provenance":{"file":"r","line":1}}]}"#,
            r#"{"interface_version":"1.2","edges":[{"src":"a","tgt":"b","kind":"k","provenance":{"file":"r","line":1},"declared_by":0}]}"#,
            r#"{"interface_version":"1.2","contract_version":5}"#,
            r#"{"contract_version":"1.2","interface_version":"1.2"}"#,
        ] {
            typed += usize::from(agrees(text) == Some(true));
        }
        // Either side of the parser's nesting limit, which both paths share.
        for depth in 120..130 {
            let deep = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
            let text = format!(
                r#"{{"interface_version":"1.2","nodes":[{{"id":"a","kind":"k","attrs":{{"t":{deep}}},"provenance":{{"file":"r","line":1}}}}]}}"#
            );
            agrees(&text);
        }
        assert!(typed > 0, "some mutations stay well formed");
    }

    /// Shapes the `Value` path reads one way and a derived type might read
    /// another: keys that only one version key consults, both alias channels
    /// populated or one empty, a replaced pathway, a null optional string, and an
    /// attr value nested to either side of the JSON parser's depth limit.
    #[test]
    fn edge_shapes_read_alike_or_are_left_to_the_value_path() {
        let finding = json!({"severity": "hint", "code": "Y", "message": "m",
                             "provenance": {"file": "r", "line": 1}});
        let mut cases: Vec<(&str, Value)> = vec![
            ("/contract_version", json!("9.9")),
            ("/contract_version", json!(5)),
            ("/contract_version", json!("1.2")),
            ("/findings", json!([])),
            ("/issues", json!([])),
            ("/issues", json!([finding])),
            ("/findings/0/state", Value::Null),
            ("/findings/0/node_id", json!("a")),
            ("/edges/2/attrs", json!({})),
            ("/edges/0/tgt", json!("a")),
            ("/nodes/0/attrs/t", json!("")),
            (
                "/pathways/1",
                json!({"name": "ms", "order": ["M1", "M0"], "current": "M0"}),
            ),
            ("/pathways/0/order", json!(["M1", "M1"])),
        ];
        for depth in [100, 120, 124, 125, 126, 127, 128, 130] {
            let mut nested = json!("leaf");
            for _ in 0..depth {
                nested = json!([nested]);
            }
            cases.push(("/nodes/1/attrs/deep", nested));
        }
        for (pointer, value) in cases {
            let mut document = base();
            if pointer.starts_with("/pathways/1") {
                document["pathways"]
                    .as_array_mut()
                    .unwrap()
                    .push(Value::Null);
            }
            set(&mut document, pointer, value);
            agrees(&document.to_string());
        }
        let only_issues = base()
            .to_string()
            .replacen(r#""findings""#, r#""issues""#, 1);
        assert_eq!(
            agrees(&only_issues),
            Some(true),
            "issues alone is read typed"
        );
    }

    /// Set `pointer` in `document`, creating the last key when it is absent.
    fn set(document: &mut Value, pointer: &str, value: Value) {
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        match document.pointer_mut(parent).unwrap() {
            Value::Object(object) => {
                object.insert(key.to_owned(), value);
            }
            Value::Array(items) => items[key.parse::<usize>().unwrap()] = value,
            other => panic!("{pointer}: parent is {other}"),
        }
    }

    mod properties {
        use proptest::prelude::*;
        use serde_json::{Value, json};

        use super::agrees;

        /// Each field drawn from values that are well formed (`true`) or not, at
        /// `odds` good draws to one bad; `odds` 0 draws only good ones.
        ///
        /// With every field free to go wrong a document is almost never well
        /// formed, so the graphs would rarely be compared, and a refusal the typed
        /// path wrongly skipped would only show where nothing else went wrong.
        /// The documents therefore mix clean ones, ones with about one fault, and
        /// ones with many.
        fn field(
            odds: u32,
            good: Vec<Value>,
            bad: Vec<Value>,
        ) -> impl Strategy<Value = (Value, bool)> {
            let good: Vec<(Value, bool)> = good.into_iter().map(|v| (v, true)).collect();
            let bad: Vec<(Value, bool)> = if odds == 0 {
                good.clone()
            } else {
                bad.into_iter().map(|v| (v, false)).collect()
            };
            prop_oneof![
                odds.max(1) => proptest::sample::select(good),
                1 => proptest::sample::select(bad),
            ]
        }

        fn node(odds: u32) -> impl Strategy<Value = (Value, bool)> {
            (
                field(
                    odds,
                    vec![json!("a"), json!("b")],
                    vec![json!(1), Value::Null],
                ),
                field(
                    odds,
                    vec![json!({}), json!({"t": "x", "n": [1, 2]})],
                    vec![Value::Null, json!([])],
                ),
                field(
                    odds,
                    vec![
                        json!({"file": "r", "line": 0}),
                        json!({"file": "s", "line": 9}),
                    ],
                    vec![json!({"file": "r"}), json!({"file": "r", "line": -2})],
                ),
                field(
                    odds,
                    vec![json!({}), json!({"t": {"file": "r", "line": 4}})],
                    vec![Value::Null, json!({"t": {"file": "r", "line": 4.5}})],
                ),
            )
                .prop_map(|((id, a), (attrs, b), (provenance, c), (located, d))| {
                    // A location for an attr the node lacks is refused by both paths.
                    let well =
                        a && b && c && d && (located == json!({}) || attrs.get("t").is_some());
                    let node = json!({"id": id, "kind": "req", "attrs": attrs,
                                      "provenance": provenance, "attr_provenance": located});
                    (node, well)
                })
        }

        /// An edge with `src` in or out of the graph, `declared_by` absent or an
        /// index below `nodes`, and optional `attrs`. Whether the index names an
        /// entry is decided against the document's actual node count.
        fn edge(odds: u32, nodes: usize) -> impl Strategy<Value = (Value, bool)> {
            let declared: Vec<Value> = (0..nodes).map(|i| json!(i)).collect();
            (
                field(odds, vec![json!("a"), json!("z")], vec![Value::Null]),
                field(odds, declared, vec![json!(nodes), Value::Null, json!(-1)]),
                any::<bool>(),
                optional(
                    odds,
                    vec![json!({}), json!({"w": 1})],
                    vec![Value::Null, json!([])],
                ),
            )
                .prop_map(|((src, a), (declared_by, b), attributed, (attrs, c))| {
                    let mut edge = json!({"src": src, "tgt": "b", "kind": "k",
                                          "provenance": {"file": "r", "line": 1}});
                    if attributed || !b {
                        edge["declared_by"] = declared_by;
                    }
                    if let Some(attrs) = attrs {
                        edge["attrs"] = attrs;
                    }
                    (edge, a && b && c)
                })
        }

        /// An optional key: absent (`None`) or one of `good` read alike, or one
        /// of `bad`, refused.
        fn optional(
            odds: u32,
            good: Vec<Value>,
            bad: Vec<Value>,
        ) -> impl Strategy<Value = (Option<Value>, bool)> {
            prop_oneof![
                1 => Just((None, true)),
                2 => field(odds, good, bad).prop_map(|(v, ok)| (Some(v), ok)),
            ]
        }

        fn finding(odds: u32) -> impl Strategy<Value = (Value, bool)> {
            // An optional string: null reads as absent; any other non-string is refused.
            let string = || optional(odds, vec![Value::Null, json!("a")], vec![json!(3)]);
            (
                field(
                    odds,
                    vec![
                        json!("error"),
                        json!("warning"),
                        json!("info"),
                        json!("hint"),
                    ],
                    vec![json!("loud"), json!(1)],
                ),
                field(
                    odds,
                    vec![json!({"file": "r", "line": 3})],
                    vec![json!({"line": 3}), json!(["r", 3])],
                ),
                string(),
                string(),
            )
                .prop_map(
                    |((severity, a), (provenance, b), (node_id, c), (state, d))| {
                        let mut finding = json!({"severity": severity, "code": "X",
                                             "message": "m", "provenance": provenance});
                        if let Some(node_id) = node_id {
                            finding["node_id"] = node_id;
                        }
                        if let Some(state) = state {
                            finding["state"] = state;
                        }
                        (finding, a && b && c && d)
                    },
                )
        }

        /// A pathway; an order that repeats a position or holds a non-string, or
        /// a current outside the order, is refused. A repeated name replaces.
        fn pathway(odds: u32) -> impl Strategy<Value = (Value, bool)> {
            (
                proptest::sample::select(vec!["ms", "phase"]),
                field(
                    odds,
                    vec![json!(["M0", "M1"]), json!(["M1"])],
                    vec![json!(["M0", "M0"]), json!(["M0", 1]), json!("M0")],
                ),
                field(odds, vec![json!("M1")], vec![json!("M9"), Value::Null]),
            )
                .prop_map(|(name, (order, a), (current, b))| {
                    (
                        json!({"name": name, "order": order, "current": current}),
                        a && b,
                    )
                })
        }

        /// Which alias carries the findings.
        #[derive(Clone, Copy, Debug)]
        enum Channel {
            Findings,
            Issues,
            /// Both aliases, `issues` empty: read as `findings`.
            FindingsBesideEmptyIssues,
            /// Both aliases, the same entries: refused unless they are empty.
            Both,
        }

        /// The parts of one document, each with whether it is well formed.
        type Parts = (
            Vec<(Value, bool)>,
            Vec<(Value, bool)>,
            Vec<(Value, bool)>,
            Vec<(Value, bool)>,
            (Value, bool),
            &'static str,
            Channel,
            Option<&'static str>,
        );

        fn document(odds: u32) -> impl Strategy<Value = Parts> {
            let channels = vec![
                Channel::Findings,
                Channel::Issues,
                Channel::FindingsBesideEmptyIssues,
                Channel::Both,
            ];
            // Where an object gains a key the interface does not define, if anywhere.
            let mut unknown = vec![None; 6];
            unknown.extend([Some(""), Some("/nodes/0"), Some("/nodes/0/provenance")]);
            (
                proptest::collection::vec(node(odds), 1..5),
                proptest::collection::vec(edge(odds, 5), 0..5),
                proptest::collection::vec(finding(odds), 0..3),
                proptest::collection::vec(pathway(odds), 0..3),
                field(
                    odds,
                    vec![json!("1.2"), json!("1.4")],
                    vec![json!("0.1"), Value::Null],
                ),
                proptest::sample::select(vec!["interface_version", "contract_version"]),
                proptest::sample::select(channels),
                proptest::sample::select(unknown),
            )
        }

        proptest! {
            /// Generator coverage: every refusal the generator can make is
            /// predicted in `well`, so a well-formed document must be read typed.
            #[test]
            fn the_typed_path_agrees_with_the_value_path(
                parts in proptest::sample::select(vec![0_u32, 40, 3]).prop_flat_map(document),
            ) {
                let (nodes, edges, findings, pathways, version, version_key, channel, unknown_key) =
                    parts;
                let node_count = nodes.len();
                let (version, mut well) = version;
                let mut take = |entries: Vec<(Value, bool)>| -> Vec<Value> {
                    entries.into_iter().map(|(entry, ok)| { well &= ok; entry }).collect()
                };
                let nodes = take(nodes);
                let edges = take(edges);
                let findings = take(findings);
                let pathways = take(pathways);
                // An index below 5 may still name no entry of a shorter document.
                well &= edges.iter().all(|e| {
                    e.get("declared_by").and_then(Value::as_u64)
                        .is_none_or(|i| usize::try_from(i).is_ok_and(|i| i < node_count))
                });
                let mut document = json!({"nodes": nodes, "edges": edges, "pathways": pathways});
                document[version_key] = version;
                match channel {
                    Channel::Findings => document["findings"] = json!(findings),
                    Channel::Issues => document["issues"] = json!(findings),
                    Channel::FindingsBesideEmptyIssues => {
                        document["findings"] = json!(findings);
                        document["issues"] = json!([]);
                    }
                    Channel::Both => {
                        well &= findings.is_empty();
                        document["findings"] = json!(findings);
                        document["issues"] = json!(findings);
                    }
                }
                // Every object the interface defines is closed.
                if let Some(pointer) = unknown_key
                    && let Some(Value::Object(object)) = document.pointer_mut(pointer)
                {
                    object.insert("extra".to_owned(), json!(1));
                    well = false;
                }
                let typed = agrees(&document.to_string());
                if well {
                    prop_assert_eq!(typed, Some(true), "well formed but not read typed: {}", document);
                }
            }
        }
    }
}
