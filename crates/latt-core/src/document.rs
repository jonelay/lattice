//! Ingest: an adapter's interface document becomes a `LatticeGraph`.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::collections::hash_map::RandomState;
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use latt_adapter::SortedMap;
use latt_adapter::process::{BoundedError, run_bounded};

use serde_json::{Map, Value};

use crate::graph::{EdgeSpec, LatticeGraph};
use crate::profile::Profile;
use crate::types::{Issue, Provenance, Severity};

mod wire;

/// The newest interface version this core accepts; [`SUPPORTED_INTERFACE_VERSIONS`]
/// lists every one.
///
/// Deliberately defined on both sides of the interface. The adapters carry their
/// own copy; that is the interface being agreed, not drift.
pub const INTERFACE_VERSION: &str = "1.4";

/// Every interface version this core can ingest.
///
/// A document outside this set is a [`ContractError`], not a finding: the core
/// has no trustworthy view of the register.
/// 1.1 extends 1.0's severity vocabulary with `hint`; 1.2 renames the adapter
/// issue channel to findings and adds optional edge attrs; 1.3 adds the optional
/// edge `declared_by`.
pub const SUPPORTED_INTERFACE_VERSIONS: &[&str] = &["1.0", "1.1", "1.2", "1.3", INTERFACE_VERSION];

/// A document the core cannot ingest: unparseable, wrong version, or off-schema.
///
/// Distinct from an `Issue` on purpose. An issue is something wrong with the
/// register; this is something wrong with the adapter, which means the core has no
/// trustworthy view of the register at all. The CLI maps it to exit 2.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
#[non_exhaustive]
pub struct ContractError {
    message: String,
}

impl ContractError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Return the explanation, the same text `Display` writes. It is for
    /// people and may change editorially; do not parse it.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// No adapter to run: none was chosen and the profile names none, or the
/// profile names a native adapter this build does not carry.
///
/// A setup problem, not a finding about the register. The CLI maps it to exit 2.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
#[non_exhaustive]
pub struct ResolveError {
    message: String,
}

impl ResolveError {
    /// Return the explanation, the same text `Display` writes. It is for
    /// people and may change editorially; do not parse it.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

pub(crate) fn err<T>(message: impl Into<String>) -> Result<T, ContractError> {
    Err(ContractError::new(message.into()))
}

/// The JSON value's runtime type: `string`, `int`, `float`, `bool`, `list`,
/// `null`, or `object`. Declared semantic types such as `date` remain strings
/// at this layer and are named separately when an expected schema type is known.
pub(crate) fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                "int"
            } else {
                "float"
            }
        }
        Value::String(_) => "string",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}

pub(crate) fn require<'a>(
    mapping: &'a Value,
    key: &str,
    where_: &str,
) -> Result<&'a Value, ContractError> {
    let Value::Object(object) = mapping else {
        return err(format!(
            "{where_}: expected an object, got {}",
            type_name(mapping)
        ));
    };
    match object.get(key) {
        Some(value) => Ok(value),
        None => err(format!("{where_}: missing '{key}'")),
    }
}

pub(crate) fn require_str<'a>(
    mapping: &'a Value,
    key: &str,
    where_: &str,
) -> Result<&'a str, ContractError> {
    let value = require(mapping, key, where_)?;
    match value {
        Value::String(text) => Ok(text),
        _ => err(format!(
            "{where_}: '{key}' must be a string, got {}",
            type_name(value)
        )),
    }
}

fn take(mapping: &mut Value, key: &str, where_: &str) -> Result<Value, ContractError> {
    let Value::Object(object) = mapping else {
        return err(format!(
            "{where_}: expected an object, got {}",
            type_name(mapping)
        ));
    };
    object
        .remove(key)
        .ok_or_else(|| ContractError::new(format!("{where_}: missing '{key}'")))
}

fn take_string(mapping: &mut Value, key: &str, where_: &str) -> Result<String, ContractError> {
    let value = take(mapping, key, where_)?;
    match value {
        Value::String(text) => Ok(text),
        other => err(format!(
            "{where_}: '{key}' must be a string, got {}",
            type_name(&other)
        )),
    }
}

/// An absent key and an explicit null both read as `None`; any other non-string
/// is an error.
fn take_optional_string(
    mapping: &mut Value,
    key: &str,
    where_: &str,
) -> Result<Option<String>, ContractError> {
    let Value::Object(object) = mapping else {
        return err(format!(
            "{where_}: expected an object, got {}",
            type_name(mapping)
        ));
    };
    match object.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => err(format!("{where_}: '{key}' must be a string or null")),
    }
}

/// Refuse an object holding a key outside `allowed`, naming every such key.
///
/// The interface's objects are closed: a misspelt key would otherwise read as an
/// absent one, and an absent optional key means "none", so the typo would become a
/// silent answer. Only the contents of `attrs` are open, and they never pass here.
fn reject_unknown_keys(
    mapping: &Value,
    allowed: &[&str],
    where_: &str,
) -> Result<(), ContractError> {
    let Value::Object(object) = mapping else {
        return err(format!(
            "{where_}: expected an object, got {}",
            type_name(mapping)
        ));
    };
    let mut unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    let plural = if unknown.len() == 1 { "" } else { "s" };
    let names = unknown
        .iter()
        .map(|key| format!("'{key}'"))
        .collect::<Vec<_>>()
        .join(", ");
    err(format!("{where_}: unknown key{plural} {names}"))
}

/// The keys a document may carry at its top level, under every accepted version.
const DOCUMENT_KEYS: &[&str] = &[
    "interface_version",
    "contract_version",
    "nodes",
    "edges",
    "pathways",
    "findings",
    "issues",
];
const NODE_KEYS: &[&str] = &["id", "kind", "attrs", "provenance", "attr_provenance"];
const EDGE_KEYS: &[&str] = &["src", "tgt", "kind", "attrs", "provenance", "declared_by"];
const PATHWAY_KEYS: &[&str] = &["name", "order", "current"];
const FINDING_KEYS: &[&str] = &[
    "severity",
    "code",
    "message",
    "provenance",
    "node_id",
    "state",
];
const PROVENANCE_KEYS: &[&str] = &["file", "line"];

fn take_provenance(entry: &mut Value, where_: &str) -> Result<Provenance, ContractError> {
    let raw = take(entry, "provenance", where_)?;
    match well_formed_provenance(raw) {
        Ok(provenance) => Ok(provenance),
        Err(raw) => parse_provenance(raw, &format!("{where_} provenance")),
    }
}

/// Read a node's optional `attr_provenance`: attr name to where it was written.
///
/// Each value is a provenance with the same closed keys, and each name must be an
/// attr the node holds: a location for an attr nobody wrote would place a finding
/// at a value that does not exist.
fn take_attr_provenance(
    entry: &mut Value,
    attrs: &Map<String, Value>,
    where_: &str,
) -> Result<SortedMap<String, Provenance>, ContractError> {
    let Some(raw) = entry
        .as_object_mut()
        .and_then(|o| o.remove("attr_provenance"))
    else {
        return Ok(SortedMap::new());
    };
    let where_ = || format!("{where_} attr_provenance");
    let Value::Object(object) = raw else {
        return err(format!(
            "{}: must be an object, got {}",
            where_(),
            type_name(&raw)
        ));
    };
    let mut located = SortedMap::new();
    for (attr, raw) in object {
        if !attrs.contains_key(&attr) {
            return err(format!(
                "{}: names attr '{attr}', which the node's attrs do not hold",
                where_()
            ));
        }
        let provenance = match well_formed_provenance(raw) {
            Ok(provenance) => provenance,
            Err(raw) => parse_provenance(raw, &format!("{} '{attr}'", where_()))?,
        };
        located.insert(attr, provenance);
    }
    Ok(located)
}

/// Read a provenance that is exactly `{"file": <string>, "line": <u32>}`, or hand
/// the value back for [`parse_provenance`] to refuse with its context. One is read
/// per located attr, so the common case builds no context string.
fn well_formed_provenance(raw: Value) -> Result<Provenance, Value> {
    let Value::Object(mut object) = raw else {
        return Err(raw);
    };
    let line = object
        .get("line")
        .and_then(Value::as_u64)
        .and_then(|line| u32::try_from(line).ok());
    if let Some(line) = line
        && object.len() == 2
        && object.get("file").is_some_and(Value::is_string)
        && let Some(Value::String(file)) = object.remove("file")
    {
        return Ok(Provenance::new(file, line));
    }
    Err(Value::Object(object))
}

fn parse_provenance(mut raw: Value, provenance_where: &str) -> Result<Provenance, ContractError> {
    reject_unknown_keys(&raw, PROVENANCE_KEYS, provenance_where)?;
    let file = take_string(&mut raw, "file", provenance_where)?;
    let line = take(&mut raw, "line", provenance_where)?;
    // Read as u64 before narrowing, so a line beyond i64 is refused as too large,
    // not as "not an integer".
    match (line.as_i64(), line.as_u64()) {
        (Some(line), _) if line < 0 => err(format!(
            "{provenance_where}: 'line' must not be negative, got {line}"
        )),
        (_, Some(wide)) => match u32::try_from(wide) {
            Ok(line) => Ok(Provenance::new(file, line)),
            Err(_) => err(format!(
                "{provenance_where}: 'line' must be at most {}, got {wide}",
                u32::MAX
            )),
        },
        _ => err(format!(
            "{provenance_where}: 'line' must be an integer, got {}",
            type_name(&line)
        )),
    }
}

fn check_version(document: &Value) -> Result<(), ContractError> {
    let version = match document.get("interface_version") {
        Some(_) => require_str(document, "interface_version", "document")?,
        None if document.get("contract_version").is_some() => {
            require_str(document, "contract_version", "document")?
        }
        None => require_str(document, "interface_version", "document")?,
    };
    if !SUPPORTED_INTERFACE_VERSIONS.contains(&version) {
        let supported = SUPPORTED_INTERFACE_VERSIONS.join(", ");
        return err(format!(
            "unsupported interface version '{version}'; this lattice supports {supported}"
        ));
    }
    Ok(())
}

pub(crate) fn entries<'a>(document: &'a Value, key: &str) -> Result<&'a [Value], ContractError> {
    let Value::Object(object) = document else {
        return err(format!(
            "document: expected an object, got {}",
            type_name(document)
        ));
    };
    match object.get(key) {
        // An absent key defaults to empty; an explicit null does not. A document
        // that says `"nodes": null` is malformed, and reading it as "no nodes"
        // would report an empty register as clean.
        None => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(other) => err(format!(
            "document: '{key}' must be a list, got {}",
            type_name(other)
        )),
    }
}

fn take_entries(document: &mut Value, key: &str) -> Result<Vec<Value>, ContractError> {
    let Value::Object(object) = document else {
        return err(format!(
            "document: expected an object, got {}",
            type_name(document)
        ));
    };
    match object.remove(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => Ok(items),
        Some(other) => err(format!(
            "document: '{key}' must be a list, got {}",
            type_name(&other)
        )),
    }
}

/// A node entry refused for repeating an earlier ID, held until the edges are
/// read so its finding can say how many of its declared edges went unread, and
/// how many from its ID were read because nothing attributed them.
struct RejectedEntry {
    index: usize,
    duplicate: crate::graph::DuplicateNode,
    unread_edges: usize,
    unattributed_edges: usize,
}

/// Add nodes in document order, collecting each repeated ID; return the number
/// of node entries and the rejected ones.
///
/// Document order is what "first occurrence wins" means, so this is a plain
/// sequential pass and not a sort.
fn ingest_nodes(
    graph: &mut LatticeGraph,
    document: &mut Value,
) -> Result<(usize, Vec<RejectedEntry>), ContractError> {
    let entries = take_entries(document, "nodes")?;
    let count = entries.len();
    let mut rejected = Vec::new();
    for (index, mut entry) in entries.into_iter().enumerate() {
        let where_ = format!("node {index}");
        reject_unknown_keys(&entry, NODE_KEYS, &where_)?;
        let id = take_string(&mut entry, "id", &where_)?;
        let kind = take_string(&mut entry, "kind", &where_)?;
        let attrs = match take(&mut entry, "attrs", &where_)? {
            Value::Object(object) => object,
            other => {
                return err(format!(
                    "{where_}: 'attrs' must be an object, got {}",
                    type_name(&other)
                ));
            }
        };
        let prov = take_provenance(&mut entry, &where_)?;
        let attr_provenance = take_attr_provenance(&mut entry, &attrs, &where_)?;
        place_node(
            graph,
            &mut rejected,
            index,
            NodeEntry {
                id,
                kind,
                attrs,
                provenance: prov,
                attr_provenance,
            },
        );
    }
    Ok((count, rejected))
}

/// A node entry as both readers hand it over, named so `id` and `kind` cannot
/// trade places.
struct NodeEntry {
    id: String,
    kind: String,
    attrs: Map<String, Value>,
    provenance: Provenance,
    attr_provenance: SortedMap<String, Provenance>,
}

/// Add node entry `index`, or hold it as rejected when its ID repeats.
///
/// Both readers call this in ascending `index` order, which keeps `rejected`
/// sorted for [`place_edge`]'s search.
fn place_node(
    graph: &mut LatticeGraph,
    rejected: &mut Vec<RejectedEntry>,
    index: usize,
    entry: NodeEntry,
) {
    let NodeEntry {
        id,
        kind,
        attrs,
        provenance,
        attr_provenance,
    } = entry;
    if let Err(duplicate) = graph.add_located_node(id, kind, attrs, provenance, attr_provenance) {
        rejected.push(RejectedEntry {
            index,
            duplicate,
            unread_edges: 0,
            unattributed_edges: 0,
        });
    }
}

/// Report each rejected entry, in document order, naming the edges it declared
/// that were not read, and the edges from its ID that were read only because none
/// says which entry declared it: the core cannot claim those were not this one's.
fn report_rejected(graph: &mut LatticeGraph, rejected: Vec<RejectedEntry>) {
    for RejectedEntry {
        duplicate,
        unread_edges,
        unattributed_edges,
        ..
    } in rejected
    {
        let mut message = format!(
            "duplicate node '{}': first at {}",
            duplicate.node_id, duplicate.existing_provenance
        );
        match unread_edges {
            0 => {}
            1 => message.push_str("; its 1 edge was not read"),
            n => message.push_str(&format!("; its {n} edges were not read")),
        }
        match unattributed_edges {
            0 => {}
            1 => message.push_str(&format!(
                "; 1 edge from '{}' has no declared_by and was read, \
                 though this entry may have declared it",
                duplicate.node_id
            )),
            n => message.push_str(&format!(
                "; {n} edges from '{}' have no declared_by and were read, \
                 though this entry may have declared them",
                duplicate.node_id
            )),
        }
        graph.add_issue(Issue::new(
            Severity::Error,
            latt_adapter::codes::PARSE_ERROR,
            message,
            duplicate.new_provenance,
            Some(duplicate.node_id),
        ));
    }
}

/// Read `declared_by`, the index of the node entry that declared the edge.
///
/// An index that is not a non-negative integer, or names no entry, means the
/// adapter is broken: guessing which row the edge came from could attach it to
/// the wrong node.
fn take_declared_by(
    object: &mut serde_json::Map<String, Value>,
    node_count: usize,
    where_: &str,
) -> Result<Option<usize>, ContractError> {
    let Some(raw) = object.remove("declared_by") else {
        return Ok(None);
    };
    let Some(index) = raw.as_u64() else {
        return err(format!(
            "{where_}: 'declared_by' must be a non-negative integer, got {}",
            match &raw {
                Value::Number(n) if n.is_i64() => n.to_string(),
                other => type_name(other).to_owned(),
            }
        ));
    };
    match usize::try_from(index) {
        Ok(index) if index < node_count => Ok(Some(index)),
        _ => err(format!(
            "{where_}: 'declared_by' {index} names no node entry; the document has {node_count}"
        )),
    }
}

/// Add edges in document order, skipping each one a rejected entry declared and
/// counting it against that entry. An edge with no `declared_by` is read, and
/// counted against every rejected entry repeating its `src`.
fn ingest_edges(
    graph: &mut LatticeGraph,
    document: &mut Value,
    node_count: usize,
    rejected: &mut [RejectedEntry],
) -> Result<(), ContractError> {
    for (index, mut entry) in take_entries(document, "edges")?.into_iter().enumerate() {
        let where_ = format!("edge {index}");
        reject_unknown_keys(&entry, EDGE_KEYS, &where_)?;
        let entry_type = type_name(&entry);
        let Value::Object(object) = &mut entry else {
            return err(format!("{where_}: expected an object, got {entry_type}"));
        };
        let attrs = match object.remove("attrs") {
            None => serde_json::Map::new(),
            Some(Value::Object(object)) => object,
            Some(other) => {
                return err(format!(
                    "{where_}: 'attrs' must be an object, got {}",
                    type_name(&other)
                ));
            }
        };
        let declared_by = take_declared_by(object, node_count, &where_)?;
        let edge = EdgeSpec {
            src: take_string(&mut entry, "src", &where_)?,
            tgt: take_string(&mut entry, "tgt", &where_)?,
            kind: take_string(&mut entry, "kind", &where_)?,
            attrs,
        };
        let provenance = take_provenance(&mut entry, &where_)?;
        place_edge(graph, rejected, declared_by, edge, provenance);
    }
    Ok(())
}

/// Add an edge, or count it against the rejected entry that declared it.
///
/// An edge with no `declared_by` is added, and counted as possibly declared by
/// every rejected entry whose ID is its `src`.
fn place_edge(
    graph: &mut LatticeGraph,
    rejected: &mut [RejectedEntry],
    declared_by: Option<usize>,
    edge: EdgeSpec,
    provenance: Provenance,
) {
    // `rejected` is in ascending index order, as the nodes were read.
    if let Some(declared_by) = declared_by
        && let Ok(position) = rejected.binary_search_by_key(&declared_by, |r| r.index)
    {
        rejected[position].unread_edges += 1;
        return;
    }
    if declared_by.is_none() {
        for entry in rejected.iter_mut() {
            if entry.duplicate.node_id == edge.src {
                entry.unattributed_edges += 1;
            }
        }
    }
    graph.add_edge(edge, provenance);
}

/// Attach each pathway, refusing an invalid one rather than reporting it.
///
/// Validity is the adapter's job; it read the declaration and can name the file.
/// An invalid pathway arriving here means the adapter is broken, and a pathway whose
/// `current` is outside its order would place every bound finding both before and
/// after it.
fn ingest_axes(graph: &mut LatticeGraph, document: &mut Value) -> Result<(), ContractError> {
    for (index, mut entry) in take_entries(document, "pathways")?.into_iter().enumerate() {
        let where_ = format!("pathway {index}");
        reject_unknown_keys(&entry, PATHWAY_KEYS, &where_)?;
        let pathway_name = take_string(&mut entry, "name", &where_)?;
        let raw = take(&mut entry, "order", &where_)?;
        let Value::Array(items) = raw else {
            return err(format!(
                "{where_} '{pathway_name}': 'order' must be a list of strings"
            ));
        };
        let mut order = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::String(text) => order.push(text),
                _ => {
                    return err(format!(
                        "{where_} '{pathway_name}': 'order' must be a list of strings"
                    ));
                }
            }
        }
        let current = take_string(&mut entry, "current", &where_)?;
        if let Err(pathway_error) = graph.set_pathway(pathway_name, order, current) {
            return err(format!("{where_}: {pathway_error}"));
        }
    }
    Ok(())
}

/// Ingest the adapter's findings from whichever alias carries them.
///
/// `issues` (before 1.2) and `findings` (1.2) name one channel. Both are read, so
/// an empty `findings` cannot hide a populated `issues`. Two populated arrays are
/// refused: merging them would invent an order, and choosing one would drop the
/// other's findings.
fn ingest_issues(graph: &mut LatticeGraph, document: &mut Value) -> Result<(), ContractError> {
    let findings = take_entries(document, "findings")?;
    let issues = take_entries(document, "issues")?;
    let entries = match (findings.is_empty(), issues.is_empty()) {
        (_, true) => findings,
        (true, false) => issues,
        (false, false) => {
            return err("document: 'findings' and 'issues' are both non-empty; \
                 they are aliases, so emit one");
        }
    };
    for (index, mut entry) in entries.into_iter().enumerate() {
        let where_ = format!("finding {index}");
        reject_unknown_keys(&entry, FINDING_KEYS, &where_)?;
        let raw_severity = take_string(&mut entry, "severity", &where_)?;
        let Some(severity) = Severity::parse(&raw_severity) else {
            return err(format!("{where_}: unknown severity '{raw_severity}'"));
        };
        let node_id = take_optional_string(&mut entry, "node_id", &where_)?;
        let state = take_optional_string(&mut entry, "state", &where_)?;
        let mut issue = Issue::new(
            severity,
            take_string(&mut entry, "code", &where_)?,
            take_string(&mut entry, "message", &where_)?,
            take_provenance(&mut entry, &where_)?,
            node_id,
        );
        issue.state = state;
        graph.add_issue(issue);
    }
    Ok(())
}

/// Parse an adapter's stdout into a document, or fail with `ContractError`.
///
/// # Errors
///
/// [`ContractError`] when the text is empty or whitespace — silence is never an
/// empty register — or is not JSON. Shape is checked later, by
/// [`ingest_document`].
#[must_use = "the parsed document is the ingest input"]
pub fn parse_document(text: &str) -> Result<Value, ContractError> {
    if text.trim().is_empty() {
        return err("adapter emitted no output");
    }
    serde_json::from_str(text)
        .map_err(|e| ContractError::new(format!("could not parse adapter output as JSON: {e}")))
}

/// The adapter's own diagnosis, trimmed. It usually explains why the run failed.
fn stderr_tail(stderr: &str) -> String {
    const LIMIT: usize = 2000;
    let text = stderr.trim();
    if text.chars().count() > LIMIT {
        let skip = text.chars().count() - LIMIT;
        return format!("…{}", text.chars().skip(skip).collect::<String>());
    }
    text.to_string()
}

/// Attempt removal on normal return or unwinding; aborts and removal errors may leave the file.
struct ScratchFile(std::path::PathBuf);

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch_nonce() -> u64 {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    RandomState::new().hash_one((
        std::process::id(),
        timestamp,
        SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed),
    ))
}

fn create_scratch_file_in(
    directory: &Path,
    mut next_nonce: impl FnMut() -> u64,
) -> std::io::Result<(ScratchFile, File)> {
    loop {
        let path = directory.join(format!(
            "lattice-profile-{}-{:016x}.json",
            std::process::id(),
            next_nonce()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((ScratchFile(path), file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

impl ScratchFile {
    fn write(self, mut file: File, contents: &str) -> Result<Self, ContractError> {
        file.write_all(contents.as_bytes()).map_err(|error| {
            ContractError::new(format!(
                "could not write resolved profile {}: {error}",
                self.0.display()
            ))
        })?;
        // The adapter opens the file itself; release the writer first.
        drop(file);
        Ok(self)
    }
}

/// `latt-adapter-<name>` for a bare program name that does not already carry the
/// prefix; `None` for anything with a directory component.
fn adapter_prefixed_name(program: &Path) -> Option<std::path::PathBuf> {
    let name = program.file_name()?;
    if name != program.as_os_str() || name.to_string_lossy().starts_with("latt-adapter-") {
        return None;
    }
    let mut prefixed = std::ffi::OsString::from("latt-adapter-");
    prefixed.push(name);
    Some(prefixed.into())
}

fn resolved_profile_scratch(contents: &str) -> Result<ScratchFile, ContractError> {
    let (scratch, file) =
        create_scratch_file_in(&std::env::temp_dir(), scratch_nonce).map_err(|error| {
            ContractError::new(format!(
                "could not create resolved profile scratch file: {error}"
            ))
        })?;
    scratch.write(file, contents)
}

/// Run an adapter program and ingest the document it writes to stdout.
///
/// The resolved profile travels as a scratch-file path so the interface stays
/// a file the adapter reads with its own library, per the adapter contract.
///
/// A bare `program` that is not found is retried as `latt-adapter-<program>`, the
/// name a release install puts on PATH; a path is run as given.
///
/// Every failure is a `ContractError`, because an adapter that did not complete
/// produced no view of the register. Reporting that as "no findings" would be
/// wrong rather than merely incomplete.
///
/// # Errors
///
/// [`ContractError`] when the scratch file cannot be written, the program cannot
/// be started, exits non-zero or by a signal, exceeds `bound`, writes stdout
/// that is not UTF-8, or writes a document that fails [`parse_document`] or
/// [`ingest_document`].
#[doc(hidden)]
pub fn run_adapter(
    program: &Path,
    resolved_profile: &str,
    target_path: &Path,
    bound: Duration,
) -> Result<LatticeGraph, ContractError> {
    run_external(program, resolved_profile, target_path, bound, None)
}

/// [`run_adapter`], naming `profile_path` where the adapter's stderr names the
/// scratch copy, as [`native_failure`] does for a native adapter.
fn run_external(
    program: &Path,
    resolved_profile: &str,
    target_path: &Path,
    bound: Duration,
    profile_path: Option<&Path>,
) -> Result<LatticeGraph, ContractError> {
    let scratch = resolved_profile_scratch(resolved_profile)?;
    let command = |program: &Path| {
        let mut command = Command::new(program);
        command
            .arg("--profile")
            .arg(&scratch.0)
            .arg("--target")
            .arg(target_path);
        command
    };
    let mut result = run_bounded(command(program), bound);
    // A release install has `latt-adapter-<name>` on PATH beside `latt`, and the
    // shims and docs spell the adapter `<name>`. Only a bare name is retried:
    // a path is the user's choice, and a fuse source resolves against its
    // manifest directory, never through PATH.
    let fallback = match &result {
        Err(BoundedError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            adapter_prefixed_name(program)
        }
        _ => None,
    };
    if let Some(fallback) = &fallback {
        result = run_bounded(command(fallback), bound);
    }
    let output = result.map_err(|e| match e {
        BoundedError::TimedOut { bound, .. } => ContractError::new(format!(
            "adapter '{}' exceeded {} s",
            program.display(),
            bound.as_secs()
        )),
        BoundedError::Io(e) => {
            let tried = fallback.as_deref().map_or_else(String::new, |fallback| {
                format!(" (also tried '{}' on PATH)", fallback.display())
            });
            ContractError::new(format!(
                "could not run adapter '{}'{tried}: {e}; {}",
                program.display(),
                native_availability()
            ))
        }
        e => ContractError::new(format!(
            "could not run adapter '{}': {e}",
            program.display()
        )),
    })?;

    if !output.status.success() {
        let code = output
            .status
            .code()
            .map_or_else(|| "by a signal".to_string(), |c| c.to_string());
        let message = format!("adapter '{}' exited {code}", program.display());
        let shown = profile_path.map_or_else(
            || "the resolved profile".to_owned(),
            |path| path.display().to_string(),
        );
        // Before the tail is cut, so a cut cannot leave half a scratch path.
        let detail = stderr_tail(
            &String::from_utf8_lossy(&output.stderr)
                .replace(&scratch.0.display().to_string(), &shown),
        );
        return Err(ContractError::new(if detail.is_empty() {
            message
        } else {
            format!("{message}: {detail}")
        }));
    }

    // A lossy decode would ingest U+FFFD in place of what the adapter wrote.
    let stdout = std::str::from_utf8(&output.stdout).map_err(|e| {
        ContractError::new(format!(
            "adapter '{}' output is not UTF-8: {e}",
            program.display()
        ))
    })?;
    ingest_text(stdout)
}

/// Build a graph from an interface document's text.
///
/// The graph and every error are those of [`parse_document`] followed by
/// [`ingest_document`]. A well-formed document is read entry by entry into the
/// graph, so no JSON tree of the whole document is held beside it; anything else
/// is read through those two functions, which say what is wrong. Prefer this to
/// the pair when the document is text: on a large register the tree, not the
/// graph, sets peak memory.
///
/// # Errors
///
/// [`ContractError`] in every case [`parse_document`] or [`ingest_document`]
/// refuses, with the same message.
///
/// ```
/// use latt_core::document::{ingest_document, ingest_text, parse_document};
///
/// let text = r#"{"interface_version": "1.2", "nodes": [{"id": "REQ-1", "kind": "req",
///     "attrs": {"title": "t"}, "attr_provenance": {"title": {"file": "r.md", "line": 2}},
///     "provenance": {"file": "r.md", "line": 1}}]}"#;
/// let graph = ingest_text(text)?;
/// assert_eq!(graph, ingest_document(parse_document(text)?)?);
///
/// let error = ingest_text(r#"{"interface_version": "1.2", "node": []}"#).unwrap_err();
/// assert_eq!(error.message(), "document: unknown key 'node'");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn ingest_text(text: &str) -> Result<LatticeGraph, ContractError> {
    match wire::ingest(text) {
        Some(graph) => Ok(graph),
        None => ingest_document(parse_document(text)?),
    }
}

/// Build a graph from an adapter's interface document.
///
/// Fails for anything off-schema. Adapter-collected issues are ingested before
/// duplicate findings so the adapter's own account of the register reads first.
///
/// A repeated node ID is a finding, not an error: the first occurrence is kept
/// and the second becomes a `PARSE_ERROR` naming both locations.
///
/// # Errors
///
/// [`ContractError`] when the document is not an object, declares an interface
/// version the core does not support, carries a key the interface does not
/// define on itself or on any entry or provenance (the contents of `attrs` stay
/// open), populates both `findings` and its pre-1.2 alias `issues`, or has an
/// entry of the wrong shape.
///
/// ```
/// use latt_core::document::ingest_document;
/// use serde_json::json;
///
/// let node = |line| json!({"id": "REQ-1", "kind": "req", "attrs": {},
///                          "provenance": {"file": "r.md", "line": line}});
/// let graph = ingest_document(json!({
///     "interface_version": "1.2", "nodes": [node(1), node(7)]
/// }))?;
/// assert_eq!(graph.iter_nodes().count(), 1);
/// assert_eq!(graph.adapter_issues()[0].code, "PARSE_ERROR");
/// assert_eq!(graph.adapter_issues()[0].origin.to_string(), "r.md:7");
/// assert!(graph.adapter_issues()[0].message.contains("r.md:1"), "names the first occurrence");
///
/// let error = ingest_document(json!({"interface_version": "9.9", "nodes": []})).unwrap_err();
/// assert!(error.to_string().contains("unsupported interface version '9.9'"), "{error}");
///
/// // A misspelt key is refused, not read as an absent one.
/// let error = ingest_document(json!({"interface_version": "1.2", "node": []})).unwrap_err();
/// assert_eq!(error.message(), "document: unknown key 'node'");
///
/// // An empty `findings` does not hide a populated `issues`.
/// let finding = json!({"severity": "error", "code": "PARSE_ERROR", "message": "m",
///                      "provenance": {"file": "r.md", "line": 1}, "node_id": null});
/// let graph = ingest_document(json!({
///     "interface_version": "1.2", "findings": [], "issues": [finding]
/// }))?;
/// assert_eq!(graph.adapter_issues().len(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn ingest_document(mut document: Value) -> Result<LatticeGraph, ContractError> {
    if !document.is_object() {
        return err(format!(
            "document: expected an object, got {}",
            type_name(&document)
        ));
    }
    check_version(&document)?;
    reject_unknown_keys(&document, DOCUMENT_KEYS, "document")?;

    let mut graph = LatticeGraph::new();
    ingest_issues(&mut graph, &mut document)?;
    ingest_axes(&mut graph, &mut document)?;
    let (node_count, mut rejected) = ingest_nodes(&mut graph, &mut document)?;
    ingest_edges(&mut graph, &mut document, node_count, &mut rejected)?;
    report_rejected(&mut graph, rejected);
    Ok(graph)
}

/// How to run the adapter: as an external program or a native (in-process) adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdapterSpec {
    /// A program run with `--profile` and `--target`, its stdout read as the
    /// interface document.
    External(std::path::PathBuf),
    /// An adapter compiled into this crate, run in process.
    Native(NativeAdapter),
}

/// A native adapter compiled into the binary.
///
/// Every variant exists only with the `native-adapters` feature (on by
/// default). Without it this enum is empty, and [`resolve_adapter`] treats every
/// adapter name as an external program, so code that matches on the variants
/// must be gated on the same feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeAdapter {
    /// `md`: tables in markdown files.
    #[cfg(feature = "native-adapters")]
    Md,
    /// `toml`: arrays of tables in TOML files.
    #[cfg(feature = "native-adapters")]
    Toml,
    /// `prose-xref`: `§` cross-references in Markdown prose.
    #[cfg(feature = "native-adapters")]
    ProseXref,
}

impl NativeAdapter {
    /// Return the name `--adapter` and `adapter.native` spell this adapter by.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            #[cfg(feature = "native-adapters")]
            Self::Md => "md",
            #[cfg(feature = "native-adapters")]
            Self::Toml => "toml",
            #[cfg(feature = "native-adapters")]
            Self::ProseXref => "prose-xref",
        }
    }
}

const NATIVE_ADAPTER_NAMES: &[&str] = &[
    #[cfg(feature = "native-adapters")]
    "md",
    #[cfg(feature = "native-adapters")]
    "toml",
    #[cfg(feature = "native-adapters")]
    "prose-xref",
];

fn native_availability() -> String {
    if NATIVE_ADAPTER_NAMES.is_empty() {
        "native adapters are not compiled in (feature 'native-adapters' is disabled)".to_owned()
    } else {
        format!(
            "available native adapters: {}",
            NATIVE_ADAPTER_NAMES.join(", ")
        )
    }
}

/// Resolve the adapter from an explicit choice and the profile.
///
/// Precedence: `cli_adapter` first, then the profile's `adapter.native`. An
/// explicit value naming a compiled-in native adapter (`md`, `toml`,
/// `prose-xref`) runs it in
/// process; anything else — including `./md`, and `md` itself when the
/// `native-adapters` feature is disabled — is an external program name or path.
///
/// # Errors
///
/// [`ResolveError`] when there is no explicit choice and the profile names no
/// native adapter, or names one that is not compiled in. The CLI maps it to
/// exit 2.
///
/// ```
/// use latt_core::document::{AdapterSpec, resolve_adapter};
/// use latt_core::profile::load_profile_value;
///
/// let yaml = "name: p\nprofile_version: '1.0.0'\nnode_kinds: {req: {id_pattern: '^REQ-'}}\nedge_kinds: {}\n";
/// let profile = load_profile_value(latt_core::serde_norway::from_str(yaml)?, "example")?;
/// assert_eq!(
///     resolve_adapter(Some("./md".as_ref()), &profile).unwrap(),
///     AdapterSpec::External("./md".into()),
/// );
/// let error = resolve_adapter(None, &profile).unwrap_err();
/// assert!(error.message().starts_with("no adapter specified"), "{error}");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn resolve_adapter(
    cli_adapter: Option<&std::ffi::OsStr>,
    profile: &Profile,
) -> Result<AdapterSpec, ResolveError> {
    if let Some(value) = cli_adapter {
        let text = value.to_string_lossy();
        if let Some(native) = parse_native_name(&text) {
            return Ok(AdapterSpec::Native(native));
        }
        return Ok(AdapterSpec::External(std::path::PathBuf::from(value)));
    }
    if let Some(name) = profile.native_adapter() {
        if let Some(native) = parse_native_name(name) {
            return Ok(AdapterSpec::Native(native));
        }
        return Err(ResolveError {
            message: format!(
                "profile declares adapter.native: '{name}', but that is not a known native adapter; {}",
                native_availability()
            ),
        });
    }
    Err(ResolveError {
        message: "no adapter specified: pass --adapter <name-or-path>, or declare adapter.native in the profile"
            .to_owned(),
    })
}

/// True when `value` is a compiled-in native adapter name, spelled exactly.
pub(crate) fn is_native_name(value: &std::ffi::OsStr) -> bool {
    value.to_str().and_then(parse_native_name).is_some()
}

fn parse_native_name(name: &str) -> Option<NativeAdapter> {
    #[cfg(feature = "native-adapters")]
    match name {
        "md" => return Some(NativeAdapter::Md),
        "toml" => return Some(NativeAdapter::Toml),
        "prose-xref" => return Some(NativeAdapter::ProseXref),
        _ => {}
    }
    let _ = name;
    None
}

/// Run a native adapter in-process, serializing through the JSON interface.
#[cfg(feature = "native-adapters")]
fn run_native(
    native: NativeAdapter,
    resolved_profile: &str,
    target_path: &Path,
    profile_path: Option<&Path>,
) -> Result<LatticeGraph, ContractError> {
    let scratch = resolved_profile_scratch(resolved_profile)?;

    let name = match native {
        NativeAdapter::Md => "md",
        NativeAdapter::Toml => "toml",
        NativeAdapter::ProseXref => "prose-xref",
    };
    // As text, not a JSON value: the typed reader then holds no tree of the
    // whole document beside the graph.
    let mut text = Vec::new();
    write_native_document(name, &scratch.0, target_path, &mut text)
        .map_err(|e| ContractError::new(native_failure(name, &scratch.0, profile_path, &e)))?;
    let text = std::str::from_utf8(&text).map_err(|e| {
        ContractError::new(format!("native adapter '{name}' output is not UTF-8: {e}"))
    })?;
    ingest_text(text)
}

/// Write the interface document native adapter `name` reads from `target` to `out`:
/// what `latt adapter <name>` prints. `profile` is a resolved profile document, as
/// `latt show` writes it, not the profile's YAML.
///
/// # Errors
///
/// The adapter's own message when it cannot load `profile` or write `out`, and a
/// message listing the available native adapters when `name` is not one of them.
#[doc(hidden)]
pub fn write_native_document(
    name: &str,
    profile: &Path,
    target: &Path,
    out: &mut impl std::io::Write,
) -> Result<(), String> {
    let Some(native) = parse_native_name(name) else {
        return Err(format!(
            "'{name}' is not a native adapter; {}",
            native_availability()
        ));
    };
    #[cfg(feature = "native-adapters")]
    match native {
        NativeAdapter::Md => crate::adapters::md::write_document(profile, target, out),
        NativeAdapter::Toml => crate::adapters::toml::write_document(profile, target, out),
        NativeAdapter::ProseXref => {
            crate::adapters::prose_xref::write_document(profile, target, out)
        }
    }
    #[cfg(not(feature = "native-adapters"))]
    {
        let _ = (profile, target, out);
        match native {}
    }
}

/// Name the profile the user wrote, not the scratch copy the adapter read.
///
/// The adapter's profile-load errors start `could not load profile <scratch>:`;
/// the rest is serde's field and position, kept as is. Any other error keeps
/// its text, with the scratch path replaced where it appears.
#[cfg(feature = "native-adapters")]
fn native_failure(name: &str, scratch: &Path, profile_path: Option<&Path>, error: &str) -> String {
    let scratch = scratch.display().to_string();
    let load_prefix = format!("could not load profile {scratch}: ");
    if let Some(detail) = error.strip_prefix(&load_prefix) {
        return match profile_path {
            Some(path) => format!(
                "could not load profile {} for adapter {name}: {detail}",
                path.display()
            ),
            None => format!("could not load the resolved profile for adapter {name}: {detail}"),
        };
    }
    let shown = profile_path.map_or_else(
        || "the resolved profile".to_owned(),
        |path| path.display().to_string(),
    );
    format!(
        "native adapter {name} failed: {}",
        error.replace(&scratch, &shown)
    )
}

/// Run an adapter (external or native) and ingest the resulting document.
///
/// `resolved_profile` is the JSON handoff document from
/// [`resolved_document`](crate::profile::resolved_document) — not the profile's
/// YAML text, and not a path. `bound` limits an external program's run; see
/// [`Profile::adapter_timeout`](crate::profile::Profile::adapter_timeout).
///
/// # Errors
///
/// [`ContractError`] when the adapter cannot be run, exits non-zero or by a
/// signal, exceeds `bound`, or emits a document that fails [`parse_document`]
/// or [`ingest_document`], and for any native spec when the `native-adapters`
/// feature is disabled. Malformed register input is not an error: it arrives as
/// findings in a valid document.
///
/// ```
/// # #[cfg(feature = "native-adapters")] {
/// use latt_core::document::{resolve_adapter, run_adapter_spec};
/// use latt_core::profile::{load_profile, resolved_document};
///
/// let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
/// let profile = load_profile(&root.join("profiles/md.yaml"))?;
/// let resolved = resolved_document(&profile)?;
/// let spec = resolve_adapter(None, &profile)?; // adapter.native: md
/// let graph = run_adapter_spec(
///     &spec,
///     &resolved,
///     &root.join("tests/fixtures/mini-md"),
///     profile.adapter_timeout(),
/// )?;
/// // Zero findings over zero edges proves nothing: check both counts.
/// assert!(graph.iter_nodes().count() > 0);
/// assert!(graph.iter_edges().count() > 0);
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn run_adapter_spec(
    spec: &AdapterSpec,
    resolved_profile: &str,
    target_path: &Path,
    bound: Duration,
) -> Result<LatticeGraph, ContractError> {
    run_adapter_spec_for_profile(spec, resolved_profile, target_path, bound, None)
}

/// [`run_adapter_spec`] for a profile loaded from `profile_path`: an adapter's
/// profile-load error names that path, which is the file the person
/// reading the error edited, instead of the scratch copy the adapter read.
///
/// # Errors
///
/// As [`run_adapter_spec`].
#[doc(hidden)]
pub fn run_adapter_spec_for_profile(
    spec: &AdapterSpec,
    resolved_profile: &str,
    target_path: &Path,
    bound: Duration,
    profile_path: Option<&Path>,
) -> Result<LatticeGraph, ContractError> {
    match spec {
        AdapterSpec::External(program) => {
            run_external(program, resolved_profile, target_path, bound, profile_path)
        }
        AdapterSpec::Native(native) => {
            #[cfg(feature = "native-adapters")]
            {
                run_native(*native, resolved_profile, target_path, profile_path)
            }
            #[cfg(not(feature = "native-adapters"))]
            {
                let _ = (native, resolved_profile, target_path, profile_path);
                Err(ContractError::new(
                    "native adapters are not compiled in (feature 'native-adapters' is disabled)"
                        .to_owned(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::create_scratch_file_in;

    /// The fast path accepts only what the full reader accepts, reads it the same,
    /// and hands everything else back unchanged for the reader to refuse.
    #[test]
    fn well_formed_provenance_agrees_with_parse_provenance() {
        use serde_json::{Value, json};
        let files = [json!("a.toml"), json!(""), json!(3), Value::Null];
        let lines = [
            json!(0),
            json!(7),
            json!(u32::MAX),
            json!(u64::from(u32::MAX) + 1),
            json!(-1),
            json!(2.5),
            json!("3"),
        ];
        let mut cases = vec![json!([]), json!("x"), json!({})];
        for file in &files {
            for line in &lines {
                cases.push(json!({"file": file, "line": line}));
                cases.push(json!({"file": file, "line": line, "extra": 1}));
                cases.push(json!({"line": line, "extra": 1}));
            }
            cases.push(json!({"file": file}));
            cases.push(json!({"file": file, "extra": 1}));
        }
        let mut accepted = 0;
        for case in cases {
            let full = super::parse_provenance(case.clone(), "p");
            match super::well_formed_provenance(case.clone()) {
                Ok(fast) => {
                    accepted += 1;
                    assert_eq!(full.ok(), Some(fast), "{case}");
                }
                Err(back) => assert_eq!(back, case),
            }
        }
        // Two files × three in-range lines.
        assert_eq!(accepted, 6);
    }

    #[cfg(unix)]
    #[test]
    fn scratch_file_is_private_at_creation() {
        use std::os::unix::fs::PermissionsExt;
        let (scratch, file) =
            create_scratch_file_in(&std::env::temp_dir(), super::scratch_nonce).unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o077, 0);
        drop(file);
        drop(scratch);
    }

    #[test]
    fn scratch_file_is_removed_after_write_failure() {
        let (scratch, file) =
            create_scratch_file_in(&std::env::temp_dir(), super::scratch_nonce).unwrap();
        let path = scratch.0.clone();
        drop(file);
        let read_only = std::fs::File::open(&path).unwrap();
        let error = scratch
            .write(read_only, "profile")
            .err()
            .expect("read-only handle cannot write");
        assert!(error.message().contains("could not write resolved profile"));
        assert!(!path.exists());
    }

    #[test]
    fn scratch_file_is_removed_during_unwinding() {
        let (scratch, file) =
            create_scratch_file_in(&std::env::temp_dir(), super::scratch_nonce).unwrap();
        let path = scratch.0.clone();
        let result = std::panic::catch_unwind(move || {
            let _scratch = scratch;
            let _file = file;
            panic!("simulate adapter unwinding");
        });
        assert!(result.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn scratch_file_creation_retries_without_overwriting_an_existing_file() {
        let nonce = 0x1234;
        let next_nonce = 0x5678;
        let directory = std::env::temp_dir().join(format!(
            "lattice-document-test-{}-{next_nonce:016x}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let existing = directory.join(format!(
            "lattice-profile-{}-{nonce:016x}.json",
            std::process::id()
        ));
        std::fs::write(&existing, "do not overwrite").unwrap();
        let mut nonces = [nonce, next_nonce].into_iter();

        let (scratch, mut file) =
            create_scratch_file_in(&directory, || nonces.next().unwrap()).unwrap();
        file.write_all(b"new profile").unwrap();

        assert_ne!(scratch.0, existing);
        assert_eq!(
            std::fs::read_to_string(&existing).unwrap(),
            "do not overwrite"
        );

        drop(file);
        drop(scratch);
        std::fs::remove_file(existing).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    mod provenance_line {
        use proptest::prelude::*;
        use serde_json::json;

        use super::super::parse_provenance;

        proptest! {
            // A line built from a known u64 is accepted exactly when it fits 32
            // bits, and the stored line is the one written.
            #[test]
            fn a_line_is_accepted_exactly_when_it_fits(raw in prop_oneof![
                0_u64..=u64::from(u32::MAX),
                u64::from(u32::MAX) + 1..=u64::MAX,
                Just(u64::from(u32::MAX)),
                Just(u64::from(u32::MAX) + 1),
            ]) {
                let parsed = parse_provenance(json!({"file": "r.md", "line": raw}), "node");
                match u32::try_from(raw) {
                    Ok(fits) => prop_assert_eq!(parsed.expect("a line that fits").wire_line(), fits),
                    Err(_) => {
                        let error = parsed.expect_err("a line beyond u32 is refused");
                        let named = error.message().contains(&format!("got {raw}"));
                        prop_assert!(named, "line {} unnamed: {}", raw, error.message());
                    }
                }
            }
        }
    }
}
