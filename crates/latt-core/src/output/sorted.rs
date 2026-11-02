//! Map-shaped JSON in byte order of its keys, whatever `serde_json` features
//! the embedding host unified.
//!
//! Under `preserve_order` a `serde_json::Map` iterates in insertion order, and
//! a host's dependency graph can switch that on for this crate. Struct fields
//! keep their declared order, which the byte baselines pin; attrs, and every
//! object nested in an attr value, go through these wrappers instead.

#![allow(clippy::disallowed_types)]

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::{Map, Value};

/// An attr map serialized with its keys in ascending byte order.
pub(super) struct SortedMap<'a>(pub &'a Map<String, Value>);

impl Serialize for SortedMap<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut entries: Vec<(&String, &Value)> = self.0.iter().collect();
        if !entries.is_sorted_by(|a, b| a.0 <= b.0) {
            entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        }
        let mut map = serializer.serialize_map(Some(entries.len()))?;
        for (key, value) in entries {
            map.serialize_entry(key, &SortedValue(value))?;
        }
        map.end()
    }
}

/// A value whose objects, at any depth, serialize as [`SortedMap`] does.
pub(super) struct SortedValue<'a>(pub &'a Value);

impl Serialize for SortedValue<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            Value::Object(map) => SortedMap(map).serialize(serializer),
            Value::Array(items) => {
                let mut sequence = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    sequence.serialize_element(&SortedValue(item))?;
                }
                sequence.end()
            }
            scalar => scalar.serialize(serializer),
        }
    }
}

/// A string bare, anything else in its JSON form. An object-valued attr prints
/// in the map's iteration order: plain and rich are stable in meaning only.
pub(super) fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Map, Value, json};

    use crate::output::{Format, output_result};
    use crate::types::{
        DiffReport, EdgeChangedRef, FuseEdge, FuseNode, FuseProvenance, FuseReport, NodeChangedRef,
        Provenance, TraceEdge, TraceEntry, TraceReport,
    };

    /// Keys inserted out of order, one holding a nested object inside an array.
    fn reversed() -> Map<String, Value> {
        let mut nested = Map::new();
        nested.insert("b".into(), json!(1));
        nested.insert("a".into(), json!(2));
        let mut attrs = Map::new();
        attrs.insert("zeta".into(), json!([Value::Object(nested)]));
        attrs.insert("alpha".into(), json!("x"));
        attrs
    }

    /// Count the objects under an `attrs*` key, failing on any whose keys are
    /// out of order. Parsing keeps text order under `preserve_order`, so this
    /// reads the bytes; on the default build it checks nothing and passes.
    fn sorted_attr_objects(value: &Value, inside: bool) -> usize {
        match value {
            Value::Object(map) => {
                let keys: Vec<&String> = map.keys().collect();
                if inside {
                    assert!(keys.is_sorted(), "unsorted keys {keys:?}");
                }
                usize::from(inside)
                    + map
                        .iter()
                        .map(|(k, v)| sorted_attr_objects(v, inside || k.starts_with("attrs")))
                        .sum::<usize>()
            }
            Value::Array(items) => items.iter().map(|v| sorted_attr_objects(v, inside)).sum(),
            _ => 0,
        }
    }

    fn check(rendered: &str, objects: usize) {
        let parsed: Value = serde_json::from_str(rendered).expect("output is JSON");
        assert_eq!(sorted_attr_objects(&parsed, false), objects, "{rendered}");
    }

    fn provenance() -> Provenance {
        Provenance::new("r.toml", 1)
    }

    fn fuse_provenance() -> FuseProvenance {
        FuseProvenance {
            source: "s".into(),
            location: provenance(),
        }
    }

    // Requirement: output/Map-shaped JSON objects sort their keys
    #[test]
    fn trace_attrs_sort_whatever_order_they_were_inserted_in() {
        let report = TraceReport {
            header: BTreeMap::new(),
            entries: vec![TraceEntry {
                id: "R1".into(),
                kind: "req".into(),
                attrs: reversed(),
                provenance: provenance(),
                summary_attr: None,
                edges: vec![TraceEdge {
                    tgt: "R2".into(),
                    kind: "uses".into(),
                    attrs: reversed(),
                    provenance: provenance(),
                }],
                findings: Vec::new(),
            }],
            unattachable_findings: Vec::new(),
            pathways: Vec::new(),
        };
        // Two attr maps, each with one nested object.
        check(&output_result(&report, Format::Json), 4);
    }

    // Requirement: output/Map-shaped JSON objects sort their keys
    #[test]
    fn fuse_attrs_sort_whatever_order_they_were_inserted_in() {
        let report = FuseReport {
            header: BTreeMap::new(),
            nodes: vec![FuseNode {
                id: "R1".into(),
                kind: "req".into(),
                attrs: reversed(),
                provenance: fuse_provenance(),
            }],
            edges: vec![FuseEdge {
                src: "R1".into(),
                tgt: "R2".into(),
                kind: "uses".into(),
                attrs: reversed(),
                provenance: fuse_provenance(),
                source_kind: "req".into(),
                target_kind: None,
                target_source: None,
            }],
            pathways: Vec::new(),
            findings: Vec::new(),
            could_run: true,
        };
        check(&output_result(&report, Format::Json), 4);
    }

    // Requirement: output/Map-shaped JSON objects sort their keys
    #[test]
    fn diff_attrs_sort_whatever_order_they_were_inserted_in() {
        let report = DiffReport {
            rev_a: "a".into(),
            rev_b: "b".into(),
            nodes_added: Vec::new(),
            nodes_removed: Vec::new(),
            nodes_changed: vec![NodeChangedRef {
                id: "R1".into(),
                kind: "req".into(),
                attrs_a: reversed(),
                attrs_b: reversed(),
            }],
            edges_added: Vec::new(),
            edges_removed: Vec::new(),
            edges_changed: vec![EdgeChangedRef {
                src: "R1".into(),
                tgt: "R2".into(),
                kind: "uses".into(),
                attrs_a: reversed(),
                attrs_b: reversed(),
            }],
            pathways_changed: Vec::new(),
        };
        check(&output_result(&report, Format::Json), 8);
    }

    // Requirement: output/Map-shaped JSON objects sort their keys
    #[test]
    fn describe_adapter_config_sorts_whatever_order_it_was_inserted_in() {
        let report = crate::types::DescribeReport {
            profile: crate::types::DescribeProfile {
                name: "p".into(),
                version: "1.0.0".into(),
            },
            adapter: crate::types::DescribeAdapter {
                name: "md".into(),
                config: Some(Value::Object(reversed())),
                config_yaml: Some("zeta: []\nalpha: x\n".into()),
            },
            node_kinds: Vec::new(),
            edge_kinds: Vec::new(),
            validations: Vec::new(),
            findings: Vec::new(),
        };
        let rendered = output_result(&report, Format::Json);
        let parsed: Value = serde_json::from_str(&rendered).expect("output is JSON");
        assert_eq!(
            sorted_attr_objects(&parsed["adapter"]["config"], true),
            2,
            "{rendered}"
        );
    }
}
