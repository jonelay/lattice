//! Ingest: an adapter's interface document becomes a `LatticeGraph`.

use std::collections::hash_map::RandomState;
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::graph::{EdgeSpec, LatticeGraph};
use crate::types::{Issue, Provenance, Severity};

/// The interface version this core emits and prefers.
///
/// Deliberately defined on both sides of the interface — the adapters carry their
/// own copy. That is the interface being agreed, not drift.
pub const INTERFACE_VERSION: &str = "1.2";

/// Every interface version this core can ingest. A document outside this set is
/// exit 2, not a finding: the core has no trustworthy view of the register.
/// 1.1 extends 1.0's severity vocabulary with `hint`; 1.2 renames the adapter
/// issue channel to findings and adds optional edge attrs.
pub const SUPPORTED_INTERFACE_VERSIONS: &[&str] = &["1.0", "1.1", INTERFACE_VERSION];

/// A document the core cannot ingest: unparseable, wrong version, or off-schema.
///
/// Distinct from an `Issue` on purpose. An issue is something wrong with the
/// register; this is something wrong with the adapter, which means the core has no
/// trustworthy view of the register at all and must exit 2.
#[derive(Debug)]
pub struct ContractError(pub String);

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ContractError {}

pub(crate) fn err<T>(message: impl Into<String>) -> Result<T, ContractError> {
    Err(ContractError(message.into()))
}

/// The JSON value's runtime type — `string`, `int`, `float`, `bool`, `list`,
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
        .ok_or_else(|| ContractError(format!("{where_}: missing '{key}'")))
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

fn take_provenance(entry: &mut Value, where_: &str) -> Result<Provenance, ContractError> {
    let mut raw = take(entry, "provenance", where_)?;
    let provenance_where = format!("{where_} provenance");
    let file = take_string(&mut raw, "file", &provenance_where)?;
    let line = take(&mut raw, "line", &provenance_where)?;
    match line.as_i64() {
        Some(line) => Ok(Provenance { file, line }),
        None => err(format!(
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
        // An absent key defaults to empty; an explicit null does not. The
        // difference matters — a document that says `"nodes": null` is malformed,
        // and reading it as "no nodes" would report an empty register as a clean
        // one, which is the silence the invariants forbid.
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

/// Add nodes in document order, turning each repeated ID into a finding.
///
/// Document order is what "first occurrence wins" means, so this is a plain
/// sequential pass and not a sort.
fn ingest_nodes(graph: &mut LatticeGraph, document: &mut Value) -> Result<(), ContractError> {
    for (index, mut entry) in take_entries(document, "nodes")?.into_iter().enumerate() {
        let where_ = format!("node {index}");
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
        if let Err(duplicate) = graph.add_node(id, kind, attrs, prov) {
            graph.add_issue(Issue::new(
                Severity::Error,
                "PARSE_ERROR",
                format!(
                    "duplicate node '{}': first at {}",
                    duplicate.node_id, duplicate.existing_provenance
                ),
                duplicate.new_provenance,
                Some(duplicate.node_id),
            ));
        }
    }
    Ok(())
}

fn ingest_edges(graph: &mut LatticeGraph, document: &mut Value) -> Result<(), ContractError> {
    for (index, mut entry) in take_entries(document, "edges")?.into_iter().enumerate() {
        let where_ = format!("edge {index}");
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
        graph.add_edge(
            EdgeSpec {
                src: take_string(&mut entry, "src", &where_)?,
                tgt: take_string(&mut entry, "tgt", &where_)?,
                kind: take_string(&mut entry, "kind", &where_)?,
                attrs,
            },
            take_provenance(&mut entry, &where_)?,
        );
    }
    Ok(())
}

/// Attach each pathway, refusing an invalid one rather than reporting it.
///
/// Validity is the adapter's job — it read the declaration and can name the file.
/// An invalid pathway arriving here means the adapter is broken, and a pathway whose
/// `current` is outside its order would place every bound finding both before and
/// after it.
fn ingest_axes(graph: &mut LatticeGraph, document: &mut Value) -> Result<(), ContractError> {
    for (index, mut entry) in take_entries(document, "pathways")?.into_iter().enumerate() {
        let where_ = format!("pathway {index}");
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

fn ingest_issues(graph: &mut LatticeGraph, document: &mut Value) -> Result<(), ContractError> {
    let key = match document {
        Value::Object(object) if object.contains_key("findings") => "findings",
        Value::Object(_) => "issues",
        other => {
            return err(format!(
                "document: expected an object, got {}",
                type_name(other)
            ));
        }
    };
    for (index, mut entry) in take_entries(document, key)?.into_iter().enumerate() {
        let where_ = format!("finding {index}");
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
#[must_use = "the parsed document is the ingest input"]
pub fn parse_document(text: &str) -> Result<Value, ContractError> {
    if text.trim().is_empty() {
        return err("adapter emitted no output");
    }
    serde_json::from_str(text)
        .map_err(|e| ContractError(format!("could not parse adapter output as JSON: {e}")))
}

/// The adapter's own diagnosis, trimmed — it is usually why the run failed.
fn stderr_tail(stderr: &str) -> String {
    const LIMIT: usize = 2000;
    let text = stderr.trim();
    if text.chars().count() > LIMIT {
        let skip = text.chars().count() - LIMIT;
        return format!("…{}", text.chars().skip(skip).collect::<String>());
    }
    text.to_string()
}

/// A scratch file removed when dropped, so no exit path can leak it.
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
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((ScratchFile(path), file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Run an adapter program and ingest the document it writes to stdout.
///
/// The resolved profile travels as a scratch-file path so the interface stays
/// a file the adapter reads with its own library, per the adapter contract.
///
/// Every failure is a `ContractError`, because an adapter that did not complete
/// produced no view of the register — reporting that as "no findings" would be
/// wrong rather than merely incomplete.
pub fn run_adapter(
    program: &Path,
    resolved_profile: &str,
    target_path: &Path,
) -> Result<LatticeGraph, ContractError> {
    let (scratch, mut scratch_file) = create_scratch_file_in(&std::env::temp_dir(), scratch_nonce)
        .map_err(|e| {
            ContractError(format!(
                "could not create resolved profile scratch file: {e}"
            ))
        })?;
    scratch_file
        .write_all(resolved_profile.as_bytes())
        .map_err(|e| {
            ContractError(format!(
                "could not write resolved profile {}: {e}",
                scratch.0.display()
            ))
        })?;
    drop(scratch_file);
    let output = Command::new(program)
        .arg("--profile")
        .arg(&scratch.0)
        .arg("--target")
        .arg(target_path)
        .output()
        .map_err(|e| {
            ContractError(format!(
                "could not run adapter '{}': {e}",
                program.display()
            ))
        })?;

    if !output.status.success() {
        let code = output
            .status
            .code()
            .map_or_else(|| "by a signal".to_string(), |c| c.to_string());
        let message = format!("adapter '{}' exited {code}", program.display());
        let detail = stderr_tail(&String::from_utf8_lossy(&output.stderr));
        return Err(ContractError(if detail.is_empty() {
            message
        } else {
            format!("{message}: {detail}")
        }));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    ingest_document(parse_document(&stdout)?)
}

/// Build a graph from an adapter's interface document.
///
/// Fails for anything off-schema. Adapter-collected issues are ingested before
/// duplicate findings so the adapter's own account of the register reads first.
pub fn ingest_document(mut document: Value) -> Result<LatticeGraph, ContractError> {
    if !document.is_object() {
        return err(format!(
            "document: expected an object, got {}",
            type_name(&document)
        ));
    }
    check_version(&document)?;

    let mut graph = LatticeGraph::new();
    ingest_issues(&mut graph, &mut document)?;
    ingest_axes(&mut graph, &mut document)?;
    ingest_nodes(&mut graph, &mut document)?;
    ingest_edges(&mut graph, &mut document)?;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::create_scratch_file_in;

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
}
