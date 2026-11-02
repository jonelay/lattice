//! Seeded graphs for a profile, with the shapes that trip validators.
//!
//! A small deterministic generator, no dependency: a graph per `(profile, seed)`
//! holding self-loops, 2- and 3-cycles, dangling targets, nodes missing optional
//! attrs, attrs holding null, and attrs of the wrong type. A failing cross-check
//! prints the seed, which reproduces the graph.

use latt_core::document::{INTERFACE_VERSION, ingest_document};
use latt_core::graph::LatticeGraph;
use latt_core::profile::{AttrSchema, Profile};
use serde_json::{Map, Value, json};

/// A 64-bit linear congruential generator: enough spread for a few dozen
/// choices per graph, and reproducible from the seed alone.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            items.get(self.below(items.len() as u64) as usize)
        }
    }
}

/// One attr value as the register might write it: typed, absent, null, or
/// mistyped, in roughly that order of likelihood.
fn attr_value(rng: &mut Rng, schema: &AttrSchema) -> Option<Value> {
    match rng.below(8) {
        0 => None,
        1 => Some(Value::Null),
        2 => Some(Value::String("not-the-declared-type".into())),
        _ => Some(typed_value(
            rng,
            schema.kind.as_str(),
            schema.items.map(|i| i.as_str()),
            schema,
        )),
    }
}

fn typed_value(rng: &mut Rng, kind: &str, items: Option<&str>, schema: &AttrSchema) -> Value {
    match kind {
        "int" => json!(i64::try_from(rng.below(7)).unwrap_or_default() - 2),
        "bool" => json!(rng.below(2) == 0),
        "date" => json!(format!("2026-0{}-1{}", 1 + rng.below(9), rng.below(9))),
        "enum" => schema
            .values
            .as_ref()
            .and_then(|values| rng.pick(values).cloned())
            .map_or_else(|| json!("other"), Value::String),
        "list" => {
            let n = rng.below(3);
            Value::Array(
                (0..n)
                    .map(|_| typed_value(rng, items.unwrap_or("string"), None, schema))
                    .collect(),
            )
        }
        _ => {
            const WORDS: [&str; 5] = ["", "alpha", "beta", "REQ", "done"];
            json!(rng.pick(&WORDS).copied().unwrap_or_default())
        }
    }
}

/// Build the graph for `(profile, seed)`.
///
/// # Panics
///
/// When the generated document does not ingest, which would be a generator bug.
#[must_use]
pub fn generate(profile: &Profile, seed: u64) -> LatticeGraph {
    let mut rng = Rng(seed
        .wrapping_mul(2_862_933_555_777_941_757)
        .wrapping_add(seed ^ 0x9E37));
    let kinds: Vec<&String> = profile.node_kinds().keys().collect();
    let mut nodes = Vec::new();
    let mut ids_of_kind: Vec<(String, Vec<String>)> = Vec::new();
    for (kind, declared) in profile.node_kinds() {
        let count = 2 + rng.below(4);
        let mut ids = Vec::new();
        for n in 0..count {
            let id = format!("{}-{n}", kind.to_uppercase());
            let mut attrs = Map::new();
            for (attr, schema) in &declared.attrs {
                if let Some(value) = attr_value(&mut rng, schema) {
                    attrs.insert(attr.clone(), value);
                }
            }
            nodes.push(json!({
                "id": id,
                "kind": kind,
                "attrs": attrs,
                "provenance": {"file": format!("{kind}.md"), "line": n + 1},
            }));
            ids.push(id);
        }
        ids_of_kind.push((kind.clone(), ids));
    }
    let ids = |kind: &str| -> Vec<String> {
        ids_of_kind
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, ids)| ids.clone())
            .unwrap_or_default()
    };

    let mut edges = Vec::new();
    let mut edge = |src: &str, tgt: &str, kind: &str, line: usize| {
        edges.push(json!({
            "src": src, "tgt": tgt, "kind": kind, "attrs": {},
            "provenance": {"file": "edges.md", "line": line},
        }));
    };
    let mut line = 0;
    for (edge_kind, declared) in profile.edge_kinds() {
        for (src_kind, tgt_kind) in &declared.allowed {
            let (sources, targets) = (ids(src_kind), ids(tgt_kind));
            let count = rng.below(5);
            for _ in 0..count {
                line += 1;
                let Some(src) = rng.pick(&sources) else {
                    continue;
                };
                match rng.below(6) {
                    // A dangling target.
                    0 => edge(
                        src,
                        &format!("{}-999", tgt_kind.to_uppercase()),
                        edge_kind,
                        line,
                    ),
                    // A self-loop where the pair allows one.
                    1 if src_kind == tgt_kind => edge(src, src, edge_kind, line),
                    // A 2-cycle where the pair allows one.
                    2 if src_kind == tgt_kind => {
                        if let Some(other) = rng.pick(&targets) {
                            edge(src, other, edge_kind, line);
                            line += 1;
                            edge(other, src, edge_kind, line);
                        }
                    }
                    // A 3-cycle where the pair allows one.
                    3 if src_kind == tgt_kind && targets.len() >= 3 => {
                        let (a, b, c) = (&targets[0], &targets[1], &targets[2]);
                        edge(a, b, edge_kind, line);
                        line += 1;
                        edge(b, c, edge_kind, line);
                        line += 1;
                        edge(c, a, edge_kind, line);
                    }
                    _ => {
                        if let Some(tgt) = rng.pick(&targets) {
                            edge(src, tgt, edge_kind, line);
                        }
                    }
                }
            }
        }
    }
    // One edge of an undeclared kind would be UNKNOWN_KIND, which no translation
    // covers; the generator stays inside the declared vocabulary.
    let _ = kinds;
    let document = json!({
        "interface_version": INTERFACE_VERSION,
        "nodes": nodes,
        "edges": edges,
    });
    ingest_document(document).unwrap_or_else(|e| panic!("seed {seed}: {e}"))
}
