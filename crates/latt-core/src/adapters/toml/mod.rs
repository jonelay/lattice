//! TOML adapter: reads flat array-of-tables registers under a profile's file globs and
//! table-to-kind mappings, and emits a Lattice interface document.

mod config;
mod tree;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use latt_adapter::codes::UNREAD_ENTRIES;
use latt_adapter::{Document, Edge, Issue, Node, Pathway, Provenance, Severity};
use serde_json::{Number, Value as JsonValue};
use tree::{Kind, Map, Text, Val};

use config::{
    AttrMapping, Config, HeaderConfig, IdPrefix, KeyMap, Mode, NonEmptyString, PathwayConfig,
    Shapes, TableConfig, TableIdPrefix,
};

fn json_value(value: &Val) -> Result<JsonValue, &'static str> {
    match &value.kind {
        Kind::String(value) | Kind::Datetime(value) => Ok(JsonValue::String(value.clone())),
        Kind::Integer(value) => Ok(JsonValue::Number(Number::from(*value))),
        Kind::Float(value) => Number::from_f64(*value)
            .map(JsonValue::Number)
            .ok_or("non-finite float"),
        Kind::Boolean(value) => Ok(JsonValue::Bool(*value)),
        // An array of scalars is a list attr; one holding an array or a table
        // has no attr shape, and reporting it beats flattening it.
        Kind::Array(entries) => entries
            .iter()
            .map(|entry| match entry.kind {
                Kind::Array(_) | Kind::Table(_) => Err("array holding a table or array"),
                _ => json_value(entry),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(JsonValue::Array),
        Kind::Table(_) => Err("table"),
    }
}

/// Longest an entry is shown in a finding; a longer one is cut with an ellipsis,
/// so one runaway entry cannot swamp a plain report line.
const ENTRY_SHOWN: usize = 120;

/// Name an array entry by position and as written: `entry 2 of 3: { glob = "b" }`.
fn entry_label(text: &Text<'_>, entry: &Val, index: usize, count: usize) -> String {
    let written = text
        .source(entry.span.as_ref(), ENTRY_SHOWN)
        .unwrap_or_else(|| format!("<{}>", entry.type_name()));
    format!("entry {} of {count}: {written}", index + 1)
}

/// Describe an unread array entry by position, as written, and its type:
/// `entry 2 of 3: { glob = "b" } is a table`.
fn entry_phrase(text: &Text<'_>, entry: &Val, index: usize, count: usize) -> String {
    let shape = entry.type_name();
    let article = if shape.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!(
        "{} is {article} {shape}",
        entry_label(text, entry, index, count)
    )
}

/// Where a finding about a mapped key goes: its file, the file's text, and the
/// node it belongs to when there is one.
struct At<'a> {
    file: &'a str,
    text: &'a Text<'a>,
    node_id: Option<&'a str>,
}

impl At<'_> {
    /// Plain output carries no `node_id`, so a message names the item itself.
    fn subject(&self, key: &str) -> String {
        self.node_id.map_or(format!("mapped key '{key}'"), |id| {
            format!("item '{id}' has mapped key '{key}'")
        })
    }

    fn push(
        &self,
        document: &mut Document,
        code: (Severity, &'static str),
        message: String,
        line: u32,
    ) {
        let mut finding = Issue::new(code.0, code.1, message, Provenance::new(self.file, line));
        if let Some(node_id) = self.node_id {
            finding = finding.with_node_id(node_id.to_owned());
        }
        document.findings.push(finding);
    }
}

/// Read a key whose profile declares entry shapes: each declared attr is the
/// entries of its shape, so it is set (an empty list when none appear) unless an
/// entry it claims cannot be read. An entry no shape claims is reported and
/// costs no attr.
fn read_shapes(
    document: &mut Document,
    key: &str,
    value: &Val,
    shapes: &Shapes,
    at: &At<'_>,
    attrs: &mut BTreeMap<String, JsonValue>,
) {
    let text = at.text;
    let Kind::Array(entries) = &value.kind else {
        at.push(
            document,
            (Severity::Error, latt_adapter::codes::PARSE_ERROR),
            format!(
                "{}: {} of type {}, expected an array for its declared shapes",
                at.file,
                at.subject(key),
                value.type_name()
            ),
            text.line(value.span.as_ref()),
        );
        return;
    };
    let mut lists: BTreeMap<&str, Option<Vec<JsonValue>>> = shapes
        .scalars
        .iter()
        .chain(shapes.tables.values())
        .map(|attr| (attr.as_str(), Some(Vec::new())))
        .collect();
    let count = entries.len();
    for (index, entry) in entries.iter().enumerate() {
        let line = text.line(entry.span.as_ref());
        // Each value this entry gives, with the attr that claims it; a table
        // entry gives one per declared field it holds.
        let claimed: Vec<(&str, &Val, Option<&str>)> = match &entry.kind {
            Kind::Array(_) => Vec::new(),
            Kind::Table(fields) => shapes
                .tables
                .iter()
                .filter_map(|(name, attr)| {
                    Some((
                        attr.as_str(),
                        fields.get(name.as_str())?,
                        Some(name.as_str()),
                    ))
                })
                .collect(),
            _ => shapes
                .scalars
                .iter()
                .map(|attr| (attr.as_str(), entry, None))
                .collect(),
        };
        if claimed.is_empty() {
            at.push(
                document,
                (Severity::Warning, UNREAD_ENTRIES),
                format!(
                    "{}: {} {} no declared shape claims, not read",
                    at.file,
                    at.subject(key),
                    entry_phrase(text, entry, index, count)
                ),
                line,
            );
            continue;
        }
        for (attr, found, field) in claimed {
            let read = match (&found.kind, field) {
                (Kind::Array(_) | Kind::Table(_), Some(field)) => Err(format!(
                    "has '{field}' of type {}, expected a scalar",
                    found.type_name()
                )),
                _ => json_value(found).map_err(|shape| match field {
                    Some(field) => {
                        format!("has '{field}' that is a {shape}, which JSON cannot hold")
                    }
                    None => format!("is a {shape}, which JSON cannot hold"),
                }),
            };
            match read {
                Ok(read) => {
                    if let Some(Some(list)) = lists.get_mut(attr) {
                        list.push(read);
                    }
                }
                Err(why) => {
                    at.push(
                        document,
                        (Severity::Error, latt_adapter::codes::PARSE_ERROR),
                        format!(
                            "{}: {} {} {why}; '{attr}' is not set",
                            at.file,
                            at.subject(key),
                            entry_label(text, entry, index, count)
                        ),
                        line,
                    );
                    lists.insert(attr, None);
                }
            }
        }
    }
    for (attr, list) in lists {
        if let Some(list) = list {
            attrs.insert(attr.to_owned(), JsonValue::Array(list));
        }
    }
}

/// Build the ID a row of `table` gets in this file: the table's own prefix
/// first, then the file stem. An edge naming its target `table` uses the same
/// rule, so the target is the ID that table's row with that name really has.
fn compose_id(
    table: &TableConfig,
    raw: &str,
    file_stem: &str,
    id_prefix: Option<IdPrefix>,
) -> String {
    let local = match table.id_prefix {
        Some(TableIdPrefix::Kind) => format!("{}/{raw}", table.kind),
        None => raw.to_owned(),
    };
    match id_prefix {
        Some(IdPrefix::FileStem) => format!("{file_stem}/{local}"),
        None => local,
    }
}

/// Locate each attr `mapping` set from `source` at the line of the key it came from.
///
/// The node sits at its table header; a finding about one value belongs on that
/// value's line, which only the adapter knows. Only attrs actually set are named:
/// the core refuses a location for an attr the node does not hold. In mapping
/// order; where two keys set one attr the later location wins, as the node keeps it.
fn attr_locations(
    source: &Map,
    mapping: &KeyMap,
    attrs: &BTreeMap<String, JsonValue>,
    at: (&str, &Text<'_>),
) -> Vec<(String, Provenance)> {
    let (file, text) = at;
    let mut located = Vec::new();
    for (key, mapping) in mapping {
        let KeyValue::Found(value) = key_value(source, key.as_str()) else {
            continue;
        };
        let line = text.line(value.span.as_ref());
        let mut place = |output: &NonEmptyString| {
            if attrs.contains_key(output.as_str()) {
                located.push((output.to_string(), Provenance::new(file, line)));
            }
        };
        match mapping {
            AttrMapping::Whole(output) => place(output),
            AttrMapping::Shapes(shapes) => shapes
                .scalars
                .iter()
                .chain(shapes.tables.values())
                .for_each(place),
        }
    }
    located
}

fn mapped_attributes(
    document: &mut Document,
    source: &Map,
    mapping: &KeyMap,
    at: (&str, &Text<'_>),
    node_id: Option<&str>,
) -> BTreeMap<String, JsonValue> {
    let (file, text) = at;
    let mut attrs = BTreeMap::new();
    for (key, mapping) in mapping {
        let value = match key_value(source, key.as_str()) {
            KeyValue::Found(value) => value,
            KeyValue::Absent => continue,
            KeyValue::Blocked {
                prefix,
                shape,
                line,
            } => {
                let message = format!(
                    "{file}: mapped key '{key}' cannot resolve: '{prefix}' has type {shape}, \
                     expected a table"
                );
                let line = text.line(line);
                if let Some(node_id) = node_id {
                    document.node_parse_error(message, file, line, node_id.to_owned());
                } else {
                    document.parse_error(message, file, line);
                }
                continue;
            }
        };
        let line = text.line(value.span.as_ref());
        let output = match mapping {
            AttrMapping::Whole(output) => output,
            AttrMapping::Shapes(shapes) => {
                let at = At {
                    file,
                    text,
                    node_id,
                };
                read_shapes(document, key.as_str(), value, shapes, &at, &mut attrs);
                continue;
            }
        };
        // Unlike an edge array, whose string entries are each a true edge, a
        // list is one value: its scalars alone would be a list the register
        // never declared. An entry of another shape rejects the attr whole, and
        // since the TOML is valid that is a warning, not a parse error. Each such
        // entry is its own finding, as written and at its line, so the user sees
        // the gap rather than a count of it. An array of scalars falls through,
        // so a scalar JSON cannot hold is still reported below rather than
        // dropped from the list.
        if let Kind::Array(entries) = &value.kind {
            // An entry is read only as a scalar: `json_value` would take an
            // array of scalars too, making a nested list a list element.
            let unread: Vec<(usize, &Val)> = entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| matches!(entry.kind, Kind::Array(_) | Kind::Table(_)))
                .collect();
            if !unread.is_empty() {
                // Plain output carries no node_id, so the message names the item.
                let subject = node_id.map_or(format!("mapped key '{key}'"), |id| {
                    format!("item '{id}' has mapped key '{key}'")
                });
                for (index, entry) in unread {
                    let mut finding = Issue::new(
                        Severity::Warning,
                        UNREAD_ENTRIES,
                        format!(
                            "{file}: {subject} {}, not read; the attr is not set",
                            entry_phrase(text, entry, index, entries.len())
                        ),
                        Provenance::new(file, text.line(entry.span.as_ref())),
                    );
                    if let Some(node_id) = node_id {
                        finding = finding.with_node_id(node_id.to_owned());
                    }
                    document.findings.push(finding);
                }
                continue;
            }
        }
        match json_value(value) {
            Ok(value) => {
                attrs.insert(output.to_string(), value);
            }
            Err(shape) => {
                let message = format!(
                    "{file}: mapped key '{key}' has type {shape}, expected a scalar value or an array of scalars"
                );
                if let Some(node_id) = node_id {
                    document.node_parse_error(message, file, line, node_id.to_owned());
                } else {
                    document.parse_error(message, file, line);
                }
            }
        }
    }
    attrs
}

fn read_header(
    document: &mut Document,
    config: &HeaderConfig,
    root: &Map,
    at: (&str, &Text<'_>),
    file_stem: &str,
) {
    let (file, text) = at;
    let Some(value) = root.get(config.table.as_str()) else {
        document.parse_error(
            format!("{file}: no [{}] header table", config.table),
            file,
            0,
        );
        return;
    };
    let line = text.line(value.span.as_ref());
    let Kind::Table(table) = &value.kind else {
        document.parse_error(
            format!(
                "{file}: header '{}' has type {}, expected a table",
                config.table,
                value.type_name()
            ),
            file,
            line,
        );
        return;
    };
    let id = if config.id.as_str() == "file_stem" {
        file_stem.to_owned()
    } else {
        match table.get(config.id.as_str()) {
            Some(value) if value.as_str().is_some() => {
                value.as_str().unwrap_or_default().to_owned()
            }
            Some(value) => {
                document.parse_error(
                    format!(
                        "{file}: header ID key '{}' has type {}, expected a string",
                        config.id,
                        value.type_name()
                    ),
                    file,
                    text.line(value.span.as_ref()),
                );
                return;
            }
            None => {
                document.parse_error(
                    format!("{file}: header declares no ID key '{}'", config.id),
                    file,
                    line,
                );
                return;
            }
        }
    };
    let attrs = mapped_attributes(document, table, &config.key_map, at, Some(&id));
    let located = attr_locations(table, &config.key_map, &attrs, at);
    let mut node =
        Node::new(id, config.kind.as_str(), Provenance::new(file, line)).with_attrs(attrs);
    for (attr, provenance) in located {
        node = node.with_attr_provenance(attr, provenance);
    }
    document.nodes.push(node);
}

/// Where an edge or mapped key leads in one record.
enum KeyValue<'v> {
    Found(&'v Val),
    /// The record does not declare the key: an optional edge, not a finding.
    Absent,
    /// A dotted key runs into a value that is not a table, so it cannot resolve.
    Blocked {
        prefix: String,
        shape: &'static str,
        line: Option<&'v std::ops::Range<usize>>,
    },
}

/// Resolve an edge or mapped key, as written first and then as a dotted path through tables.
///
/// `people.leads` names `leads` inside `[people]`; read only as a literal key it
/// matched nothing, and the edges or attr were dropped with no finding. A key that
/// really contains a dot (a quoted TOML key) still matches as written.
fn key_value<'v>(row: &'v Map, key: &str) -> KeyValue<'v> {
    if let Some(value) = row.get(key) {
        return KeyValue::Found(value);
    }
    let mut components = key.split('.');
    let Some(first) = components.next() else {
        return KeyValue::Absent;
    };
    let Some(mut current) = row.get(first) else {
        return KeyValue::Absent;
    };
    let mut walked = first.len();
    for component in components {
        let Some(table) = current.as_table() else {
            return KeyValue::Blocked {
                prefix: key[..walked].to_owned(),
                shape: current.type_name(),
                line: current.span.as_ref(),
            };
        };
        let Some(next) = table.get(component) else {
            return KeyValue::Absent;
        };
        current = next;
        walked += 1 + component.len();
    }
    KeyValue::Found(current)
}

/// The target an edge entry names: a string, or with `entry_key` the string
/// that field holds in a table entry.
fn edge_target<'v>(entry: &'v Val, entry_key: Option<&str>) -> Option<&'v str> {
    match &entry.kind {
        Kind::String(target) => Some(target.as_str()),
        Kind::Table(fields) => fields.get(entry_key?)?.as_str(),
        _ => None,
    }
}

fn read_edges(
    document: &mut Document,
    config: &Config,
    table: &TableConfig,
    row: &Map,
    node_id: &str,
    declared_by: usize,
    at: (&str, &str, &Text<'_>),
) {
    let (file, file_stem, text) = at;
    for (key, edge) in &table.edge_keys {
        let kind = &edge.kind;
        let found = match (key_value(row, key.as_str()), &edge.fallback) {
            (KeyValue::Absent, Some(fallback)) => key_value(row, fallback.as_str()),
            (found, _) => found,
        };
        let value = match found {
            KeyValue::Found(value) => value,
            KeyValue::Absent => continue,
            KeyValue::Blocked {
                prefix,
                shape,
                line,
            } => {
                document.node_parse_error(
                    format!(
                        "{file}: item '{node_id}' has edge key '{key}', but '{prefix}' has type \
                         {shape}, expected a table"
                    ),
                    file,
                    text.line(line),
                    node_id.to_owned(),
                );
                continue;
            }
        };
        let line = text.line(value.span.as_ref());
        // Each target with the line it was written on: an entry of an array is
        // its own edge, at its own line.
        let entry_key = edge.entry_key.as_ref().map(|k| k.as_str());
        // Built only when an error names it, never for an edge that reads.
        let expected = || {
            entry_key.map_or(Cow::Borrowed("a target string"), |k| {
                Cow::Owned(format!("a target string or a table with '{k}'"))
            })
        };
        // A scalar is one target and needs no Vec; an array fills `many`.
        let (single, many) = match &value.kind {
            Kind::Array(entries) => {
                // A string entry is a reference whatever sits beside it; reporting
                // the others must not cost the ones that read. Each other entry
                // is its own error, as written and at its line: an edge it may
                // have meant is missing from the graph.
                let mut targets = Vec::new();
                for (index, entry) in entries.iter().enumerate() {
                    let at = text.line(entry.span.as_ref());
                    if let Some(target) = edge_target(entry, entry_key) {
                        targets.push((target, at));
                        continue;
                    }
                    document.node_parse_error(
                        format!(
                            "{file}: item '{node_id}' has edge key '{key}' {}, not {}; \
                             it was not read",
                            entry_phrase(text, entry, index, entries.len()),
                            expected()
                        ),
                        file,
                        at,
                        node_id.to_owned(),
                    );
                }
                (None, targets)
            }
            _ => {
                if let Some(target) = edge_target(value, entry_key) {
                    (Some((target, line)), Vec::new())
                } else {
                    let array = if entry_key.is_some() {
                        "an array of them"
                    } else {
                        "an array of strings"
                    };
                    document.node_parse_error(
                        format!(
                            "{file}: item '{node_id}' has edge key '{key}' of type {}, expected \
                             {} or {array}",
                            value.type_name(),
                            expected()
                        ),
                        file,
                        line,
                        node_id.to_owned(),
                    );
                    continue;
                }
            }
        };
        let target_table = edge
            .table
            .as_ref()
            .map(|name| &config.tables[name.as_str()]);
        for (target, line) in single.into_iter().chain(many) {
            let target = match target_table {
                Some(target_table) => compose_id(target_table, target, file_stem, config.id_prefix),
                None => target.to_owned(),
            };
            document.edges.push(
                Edge::new(
                    node_id.to_owned(),
                    target,
                    kind.as_str(),
                    Provenance::new(file, line),
                )
                .declared_by(declared_by),
            );
        }
    }
}

fn read_rows(
    document: &mut Document,
    config: &Config,
    table: &TableConfig,
    rows: &[Val],
    table_name: &str,
    at: (&str, &str, &Text<'_>),
) {
    let (file, _, text) = at;
    for (index, value) in rows.iter().enumerate() {
        let line = text.line(value.span.as_ref());
        let Kind::Table(row) = &value.kind else {
            document.parse_error(
                format!(
                    "{file}: {table_name} row {index} has type {}, expected a table",
                    value.type_name()
                ),
                file,
                line,
            );
            continue;
        };
        read_record(
            document,
            config,
            table,
            (row, line),
            format_args!("{table_name} row {index}"),
            at,
        );
    }
}

/// `description` is deferred formatting: it is read only by the error arm, so
/// a successfully parsed row never allocates a string for it.
fn read_record(
    document: &mut Document,
    config: &Config,
    table: &TableConfig,
    record: (&Map, u32),
    description: std::fmt::Arguments<'_>,
    at: (&str, &str, &Text<'_>),
) {
    let (record, line) = record;
    let (file, file_stem, text) = at;
    let Some(raw_id) = record.get(table.id_key.as_str()).and_then(Val::as_str) else {
        document.parse_error(
            format!(
                "{file}: {description} declares no string '{}' ID key",
                table.id_key
            ),
            file,
            line,
        );
        return;
    };
    let id = compose_id(table, raw_id, file_stem, config.id_prefix);
    let attrs = mapped_attributes(document, record, &table.key_map, (file, text), Some(&id));
    let located = attr_locations(record, &table.key_map, &attrs, (file, text));
    let index = document.nodes.len();
    // Edges first, so the node takes the ID rather than a copy of it.
    read_edges(document, config, table, record, &id, index, at);
    let mut node =
        Node::new(id, table.kind.as_str(), Provenance::new(file, line)).with_attrs(attrs);
    for (attr, provenance) in located {
        node = node.with_attr_provenance(attr, provenance);
    }
    document.nodes.push(node);
}

fn read_file(document: &mut Document, config: &Config, value: &Val, at: (&str, &str, &Text<'_>)) {
    let (file, file_stem, text) = at;
    let Some(root) = value.as_table() else {
        document.parse_error(format!("{file}: TOML root is not a table"), file, 0);
        return;
    };

    if let Some(header) = &config.header {
        read_header(document, header, root, (file, text), file_stem);
    }

    let is_rows = |rows: &[Val]| rows.iter().all(|row| matches!(row.kind, Kind::Table(_)));
    for (name, value) in root {
        if let Some(table) = config.tables.get(name.as_str()) {
            match &value.kind {
                Kind::Array(rows) if is_rows(rows) => {
                    read_rows(document, config, table, rows, name, at);
                }
                _ => document.parse_error(
                    format!(
                        "{file}: configured table '{name}' has type {}, expected an array of tables",
                        value.type_name()
                    ),
                    file,
                    text.line(value.span.as_ref()),
                ),
            }
        } else if let Kind::Array(rows) = &value.kind
            && !rows.is_empty()
            && is_rows(rows)
        {
            document.parse_error(
                format!(
                    "{file}: no reader for array-of-tables '{name}' ({} rows not read)",
                    rows.len()
                ),
                file,
                text.line(rows[0].span.as_ref()),
            );
        }
    }
}

fn read_directory_file(
    document: &mut Document,
    config: &Config,
    value: &Val,
    at: (&str, &str, &Text<'_>),
) {
    let file = at.0;
    let Some(root) = value.as_table() else {
        document.parse_error(format!("{file}: TOML root is not a table"), file, 0);
        return;
    };
    let table = config
        .tables
        .values()
        .next()
        .expect("configuration requires at least one table");
    // The record is the whole file, so it has no line of its own.
    read_record(
        document,
        config,
        table,
        (root, 0),
        format_args!("record"),
        at,
    );
}

fn dot_path<'a>(value: &'a Val, path: &str) -> Option<&'a Val> {
    let mut current = value;
    for component in path.split('.') {
        current = current.as_table()?.get(component)?;
    }
    Some(current)
}

fn pathway_source(config: &PathwayConfig) -> String {
    config.source_file.as_str().replace('\\', "/")
}

fn attach_pathway(document: &mut Document, config: &PathwayConfig, source: Option<&Val>) {
    let Some(source) = source else {
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' source file was not read",
                config.source_file, config.name
            ),
            config.source_file.as_str(),
        );
        return;
    };
    let order = dot_path(source, config.order_key.as_str());
    let current = dot_path(source, config.current_key.as_str());
    if order.is_none() && current.is_none() {
        return;
    }
    let (Some(order), Some(current)) = (order, current) else {
        let missing = if order.is_none() {
            &config.order_key
        } else {
            &config.current_key
        };
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' declares only one of the pair, missing '{missing}'",
                config.source_file, config.name
            ),
            config.source_file.as_str(),
        );
        return;
    };
    let (Kind::Array(raw_order), Kind::String(current)) = (&order.kind, &current.kind) else {
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' expects '{}' to be an array of strings and '{}' to be a string",
                config.source_file, config.name, config.order_key, config.current_key
            ),
            config.source_file.as_str(),
        );
        return;
    };
    let Some(order): Option<Vec<String>> = raw_order
        .iter()
        .map(|value| value.as_str().map(str::to_owned))
        .collect()
    else {
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' order contains a non-string value",
                config.source_file, config.name
            ),
            config.source_file.as_str(),
        );
        return;
    };
    let unique: BTreeSet<_> = order.iter().collect();
    if order.is_empty() || unique.len() != order.len() || !order.contains(current) {
        document.pathway_invalid(
            format!(
                "{}: pathway '{}' requires distinct positions and current '{}' among them",
                config.source_file, config.name, current
            ),
            config.source_file.as_str(),
        );
        return;
    }
    document.pathways.push(Pathway::new(
        config.name.to_string(),
        order,
        current.clone(),
    ));
}

fn build_doc(config: &Config, target: &Path) -> Result<Document, String> {
    let matcher = latt_adapter::FileMatcher::new(&config.files)?;
    let mut document = Document::default();
    // Only the pathway source is read again; every other tree drops with its file.
    let pathway_file = config.pathway.as_ref().map(pathway_source);
    let mut pathway_tree = None;
    let target_is_readable = target.is_dir() || target.is_file();
    for (path, file) in latt_adapter::selected_files(&mut document, target, &matcher) {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                document.parse_error(format!("could not read '{file}': {error}"), file, 0);
                continue;
            }
        };
        if let Some(refusal) = super::too_many_lines(&bytes) {
            document.parse_error(format!("{file}: {refusal}"), file, 0);
            continue;
        }
        let text = match latt_adapter::decode_utf8(&bytes) {
            Ok(text) => text,
            Err(_) => {
                // TOML is defined over UTF-8: one such line makes the file not
                // TOML, as a syntax error does, so the whole file goes unread.
                for line in latt_adapter::decode_lines(&bytes).1 {
                    document.parse_error(
                        format!(
                            "{file}: line is not valid UTF-8 at column {}: {}; TOML must be \
                             UTF-8, so the file was not read",
                            line.column, line.as_written
                        ),
                        &file,
                        line.line,
                    );
                }
                continue;
            }
        };
        let lines = Text::new(text);
        let value = match tree::parse(text) {
            Ok(value) => value,
            Err(error) => {
                document.parse_error(
                    format!("{file}: could not parse TOML: {error}"),
                    file,
                    lines.line(error.span().as_ref()),
                );
                continue;
            }
        };
        let file_stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        match config.mode.unwrap_or_default() {
            Mode::Tables => read_file(&mut document, config, &value, (&file, &file_stem, &lines)),
            Mode::Directory => {
                read_directory_file(&mut document, config, &value, (&file, &file_stem, &lines));
            }
        }
        if pathway_file.as_deref() == Some(file.as_str()) {
            pathway_tree = Some(value);
        }
    }
    if target_is_readable && let Some(pathway) = &config.pathway {
        attach_pathway(&mut document, pathway, pathway_tree.as_ref());
    }
    latt_adapter::check_sources(&mut document, &config.node_kinds, target)
        .map_err(|error| format!("could not load profile: {error}"))?;
    Ok(document)
}

/// Build an interface document from a resolved profile and target and write it
/// to `out` as one newline-terminated JSON line.
///
/// # Errors
///
/// As [`build_document_value`], and when `out` cannot be written.
pub fn write_document(
    profile: &Path,
    target: &Path,
    out: &mut impl std::io::Write,
) -> Result<(), String> {
    let config = Config::load(profile)?;
    let target =
        latt_adapter::absolute_path(target.to_owned()).map_err(|error| error.to_string())?;
    let document = build_doc(&config, &target)?;
    serde_json::to_writer(&mut *out, &document)
        .map_err(|error| format!("could not write interface document: {error}"))?;
    out.write_all(b"\n")
        .and_then(|()| out.flush())
        .map_err(|error| format!("could not write interface document: {error}"))
}

/// Build an interface document as a `serde_json::Value`, skipping the
/// string roundtrip for in-process callers.
///
/// `profile` is a path to the *resolved* profile document — the JSON that
/// `latt show` prints, carrying `"resolved_schema": "1"` — not the
/// profile's YAML source. Unreadable register input is not an error: it
/// arrives as `PARSE_ERROR` findings in the returned document.
///
/// # Errors
///
/// A message when the profile cannot be read, is not a resolved document, or
/// configures the adapter wrongly (the CLI's exit 2).
///
/// ```
/// let dir = std::env::temp_dir().join(format!("latt-adapter-toml-doc-{}", std::process::id()));
/// std::fs::create_dir_all(dir.join("target"))?;
/// std::fs::write(dir.join("target/r.toml"), "[[item]]\nid = \"A\"\n\n[[item]]\nid = \"B\"\nrefs = [\"A\"]\n")?;
/// std::fs::write(dir.join("profile.json"), r#"{"resolved_schema": "1",
///     "adapter": {"paths": {"files": ["*.toml"]}, "tables": {"item": {"kind": "item",
///                 "id_key": "id", "key_map": {}, "edge_keys": {"refs": "references"}}}},
///     "node_kinds": {"item": {}}, "edge_kinds": {"references": {}}}"#)?;
/// let document =
///     latt_core::adapters::toml::build_document_value(&dir.join("profile.json"), &dir.join("target"));
/// std::fs::remove_dir_all(&dir)?;
/// let document = document?;
/// assert_eq!(document["nodes"].as_array().unwrap().len(), 2);
/// assert_eq!(document["edges"].as_array().unwrap().len(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn build_document_value(profile: &Path, target: &Path) -> Result<JsonValue, String> {
    let config = Config::load(profile)?;
    let target =
        latt_adapter::absolute_path(target.to_owned()).map_err(|error| error.to_string())?;
    let document = build_doc(&config, &target)?;
    serde_json::to_value(&document)
        .map_err(|error| format!("could not serialize interface document: {error}"))
}

#[cfg(test)]
mod tests {

    use super::config::{Mode, NonEmptyString, PathwayConfig};
    use super::tree::{Kind, Val, parse};
    use super::{KeyValue, attach_pathway, json_value, key_value};
    use latt_adapter::Document;
    use serde_json::json;

    /// A value with no span, for tests about shape alone.
    fn val(kind: Kind) -> Val {
        Val { kind, span: None }
    }

    #[test]
    fn an_edge_key_resolves_as_written_then_as_a_dotted_path() {
        let parsed = parse(
            "\"a.b\" = \"literal\"\nleaf = \"x\"\n[a]\nb = \"nested\"\n[c.d]\ne = [\"deep\"]\n",
        )
        .unwrap();
        let row = parsed.as_table().unwrap();
        let found = |key| match key_value(row, key) {
            KeyValue::Found(value) => json_value(value).ok(),
            _ => None,
        };
        assert_eq!(found("a.b"), Some(json!("literal")));
        assert_eq!(found("c.d.e"), Some(json!(["deep"])));
        assert!(matches!(key_value(row, "c.x.e"), KeyValue::Absent));
        assert!(matches!(
            key_value(row, "leaf.more"),
            KeyValue::Blocked { prefix, shape: "string", .. } if prefix == "leaf"
        ));
    }

    #[test]
    fn scalar_types_map_without_stringifying() {
        assert_eq!(json_value(&val(Kind::Integer(3))).unwrap(), json!(3));
        assert_eq!(json_value(&val(Kind::Boolean(true))).unwrap(), json!(true));
        let scalars = val(Kind::Array(vec![
            val(Kind::String("a".into())),
            val(Kind::Integer(2)),
        ]));
        assert_eq!(json_value(&scalars).unwrap(), json!(["a", 2]));
        assert_eq!(
            json_value(&val(Kind::Array(Vec::new()))).unwrap(),
            json!([])
        );
        let nested = val(Kind::Array(vec![val(Kind::Array(Vec::new()))]));
        assert!(json_value(&nested).is_err());
    }

    #[test]
    fn directory_mode_config_deserializes() {
        let mode: Mode = serde_json::from_str(r#""directory""#).unwrap();
        assert!(matches!(mode, Mode::Directory));
    }

    #[test]
    fn sources_row_naming_a_deleted_file_is_reported() {
        let dir =
            std::env::temp_dir().join(format!("latt-adapter-toml-sources-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/here.md"), "").unwrap();
        std::fs::write(
            dir.join("register.toml"),
            "[[doc]]\nid = \"docs/here.md\"\n\n[[doc]]\nid = \"docs/gone.md\"\n",
        )
        .unwrap();
        let profile = dir.join("profile.json");
        std::fs::write(
            &profile,
            r#"{"resolved_schema": "1",
                "adapter": {"paths": {"files": ["*.toml"]},
                            "tables": {"doc": {"kind": "doc", "id_key": "id",
                                               "key_map": {}, "edge_keys": {}}}},
                "node_kinds": {"doc": {"id_names": "file"}}}"#,
        )
        .unwrap();
        let document = super::build_document_value(&profile, &dir);
        std::fs::remove_dir_all(&dir).unwrap();
        let findings = document.unwrap()["findings"].clone();
        let findings = findings.as_array().unwrap();
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0]["code"], "SOURCE_MISSING");
        assert_eq!(findings[0]["severity"], "warning");
        assert_eq!(findings[0]["node_id"], "docs/gone.md");
        assert_eq!(findings[0]["provenance"]["file"], "register.toml");
    }

    /// The pathway is read from the one selected file its `source_file` names, a
    /// `\`-separated name included, among others whose trees are not kept.
    #[test]
    fn pathway_is_read_from_the_file_its_source_names() {
        let dir =
            std::env::temp_dir().join(format!("latt-adapter-toml-pathway-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("registers")).unwrap();
        std::fs::write(dir.join("registers/a.toml"), "[[item]]\nid = \"a\"\n").unwrap();
        std::fs::write(
            dir.join("registers/index.toml"),
            "[register]\nstage_order = [\"s1\", \"s2\"]\ncurrent_stage = \"s2\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("registers/z.toml"), "[[item]]\nid = \"z\"\n").unwrap();
        let profile = dir.join("profile.json");
        std::fs::write(
            &profile,
            r#"{"resolved_schema": "1",
                "adapter": {"paths": {"files": ["registers/*.toml"]},
                            "tables": {"item": {"kind": "item", "id_key": "id",
                                                "key_map": {}, "edge_keys": {}}},
                            "pathway": {"source_file": "registers\\index.toml",
                                        "name": "stage",
                                        "order_key": "register.stage_order",
                                        "current_key": "register.current_stage"}},
                "node_kinds": {"item": {}}}"#,
        )
        .unwrap();
        let document = super::build_document_value(&profile, &dir);
        std::fs::remove_dir_all(&dir).unwrap();
        let document = document.unwrap();
        assert_eq!(document["findings"], serde_json::json!([]), "{document}");
        assert_eq!(document["nodes"].as_array().unwrap().len(), 2);
        let pathways = document["pathways"].as_array().unwrap();
        assert_eq!(pathways.len(), 1, "{pathways:?}");
        assert_eq!(pathways[0]["name"], "stage");
        assert_eq!(pathways[0]["current"], "s2");
    }

    #[test]
    fn valid_pathway_is_attached() {
        let source =
            parse("[register]\nstage_order = [\"s1\", \"s2\"]\ncurrent_stage = \"s2\"\n").unwrap();
        let key = |value: &str| NonEmptyString::new(value.to_owned(), "test").unwrap();
        let config = PathwayConfig {
            source_file: key("registers/index.toml"),
            name: key("stage"),
            order_key: key("register.stage_order"),
            current_key: key("register.current_stage"),
        };
        let mut document = Document::default();
        attach_pathway(&mut document, &config, Some(&source));
        assert_eq!(document.pathways.len(), 1);
        assert_eq!(document.pathways[0].current, "s2");
    }
}
