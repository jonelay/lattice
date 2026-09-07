//! Ingest: an adapter's contract document becomes a `LatticeGraph`.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::graph::LatticeGraph;
use crate::types::{Issue, Provenance, Severity};

/// The contract version this core emits and prefers.
///
/// Deliberately defined on both sides of the contract — the adapters carry their
/// own copy. That is the contract being agreed, not drift.
pub const CONTRACT_VERSION: &str = "1.1";

/// Every contract version this core can ingest. A document outside this set is
/// exit 2, not a finding: the core has no trustworthy view of the register.
/// 1.1 extends 1.0's severity vocabulary with `hint` and is otherwise
/// identical, so one parser serves both.
pub const SUPPORTED_CONTRACT_VERSIONS: &[&str] = &["1.0", CONTRACT_VERSION];

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

/// The value's type in the profile's own attr-type vocabulary — `string`,
/// `int`, `float`, `bool`, `list` — extended with `null` and `object`, so a
/// schema failure names types in the same words a profile author writes.
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

fn provenance(entry: &Value, where_: &str) -> Result<Provenance, ContractError> {
    let raw = require(entry, "provenance", where_)?;
    let file = require_str(raw, "file", &format!("{where_} provenance"))?.to_string();
    let line = require(raw, "line", &format!("{where_} provenance"))?;
    match line.as_i64() {
        // A JSON float or boolean line number is a schema failure rather than a
        // line the core should round or invent.
        Some(line) => Ok(Provenance { file, line }),
        None => err(format!(
            "{where_} provenance: 'line' must be an integer, got {}",
            type_name(line)
        )),
    }
}

fn check_version(document: &Value) -> Result<(), ContractError> {
    let version = require_str(document, "contract_version", "document")?;
    if !SUPPORTED_CONTRACT_VERSIONS.contains(&version) {
        let supported = SUPPORTED_CONTRACT_VERSIONS.join(", ");
        return err(format!(
            "unsupported contract version '{version}'; this lattice supports {supported}"
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

/// Add nodes in document order, turning each repeated ID into a finding.
///
/// Document order is what "first occurrence wins" means, so this is a plain
/// sequential pass and not a sort.
fn ingest_nodes(graph: &mut LatticeGraph, document: &Value) -> Result<(), ContractError> {
    for (index, entry) in entries(document, "nodes")?.iter().enumerate() {
        let where_ = format!("node {index}");
        let id = require_str(entry, "id", &where_)?.to_string();
        let kind = require_str(entry, "kind", &where_)?.to_string();
        let attrs = match require(entry, "attrs", &where_)? {
            Value::Object(object) => object.clone(),
            other => {
                return err(format!(
                    "{where_}: 'attrs' must be an object, got {}",
                    type_name(other)
                ));
            }
        };
        let prov = provenance(entry, &where_)?;
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

fn ingest_edges(graph: &mut LatticeGraph, document: &Value) -> Result<(), ContractError> {
    for (index, entry) in entries(document, "edges")?.iter().enumerate() {
        let where_ = format!("edge {index}");
        graph.add_edge(
            require_str(entry, "src", &where_)?.to_string(),
            require_str(entry, "tgt", &where_)?.to_string(),
            require_str(entry, "kind", &where_)?.to_string(),
            provenance(entry, &where_)?,
        );
    }
    Ok(())
}

/// Attach each axis, refusing an invalid one rather than reporting it.
///
/// Validity is the adapter's job — it read the declaration and can name the file.
/// An invalid axis arriving here means the adapter is broken, and an axis whose
/// `current` is outside its order would place every bound finding both before and
/// after it.
fn ingest_axes(graph: &mut LatticeGraph, document: &Value) -> Result<(), ContractError> {
    for (index, entry) in entries(document, "axes")?.iter().enumerate() {
        let where_ = format!("axis {index}");
        let axis_name = require_str(entry, "name", &where_)?.to_string();
        let raw = require(entry, "order", &where_)?;
        let Value::Array(items) = raw else {
            return err(format!(
                "{where_} '{axis_name}': 'order' must be a list of strings"
            ));
        };
        let mut order = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::String(text) => order.push(text.clone()),
                _ => {
                    return err(format!(
                        "{where_} '{axis_name}': 'order' must be a list of strings"
                    ));
                }
            }
        }
        let current = require_str(entry, "current", &where_)?.to_string();
        if let Err(axis_error) = graph.set_axis(axis_name, order, current) {
            return err(format!("{where_}: {axis_error}"));
        }
    }
    Ok(())
}

fn ingest_issues(graph: &mut LatticeGraph, document: &Value) -> Result<(), ContractError> {
    for (index, entry) in entries(document, "issues")?.iter().enumerate() {
        let where_ = format!("issue {index}");
        let raw_severity = require_str(entry, "severity", &where_)?;
        let Some(severity) = Severity::parse(raw_severity) else {
            return err(format!("{where_}: unknown severity '{raw_severity}'"));
        };
        let node_id = match entry.get("node_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.clone()),
            Some(_) => return err(format!("{where_}: 'node_id' must be a string or null")),
        };
        graph.add_issue(Issue::new(
            severity,
            require_str(entry, "code", &where_)?,
            require_str(entry, "message", &where_)?,
            provenance(entry, &where_)?,
            node_id,
        ));
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
    let scratch = ScratchFile(
        std::env::temp_dir().join(format!("lattice-profile-{}.json", std::process::id())),
    );
    std::fs::write(&scratch.0, resolved_profile).map_err(|e| {
        ContractError(format!(
            "could not write resolved profile {}: {e}",
            scratch.0.display()
        ))
    })?;
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
    ingest_document(&parse_document(&stdout)?)
}

/// Build a graph from an adapter's contract document.
///
/// Fails for anything off-schema. Adapter-collected issues are ingested before
/// duplicate findings so the adapter's own account of the register reads first.
pub fn ingest_document(document: &Value) -> Result<LatticeGraph, ContractError> {
    if !document.is_object() {
        return err(format!(
            "document: expected an object, got {}",
            type_name(document)
        ));
    }
    check_version(document)?;

    let mut graph = LatticeGraph::new();
    ingest_issues(&mut graph, document)?;
    ingest_axes(&mut graph, document)?;
    ingest_nodes(&mut graph, document)?;
    ingest_edges(&mut graph, document)?;
    Ok(graph)
}
