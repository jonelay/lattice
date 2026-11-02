//! Loading and checking the fuse profile: qualified kinds, edges, validations.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::types::Severity;
use crate::validate::resolve_severity;

use super::FuseError;
use super::manifest::{load_yaml, nonempty};

#[derive(Debug, Deserialize)]
pub(super) struct EdgeDeclaration {
    pub(super) allowed: Vec<(String, String)>,
}

/// A checked fuse profile, built only by [`load_fuse_profile`].
#[derive(Debug)]
pub struct FuseProfile {
    pub(super) edges: BTreeMap<String, EdgeDeclaration>,
    pub(super) validations: Vec<(String, Map<String, Value>)>,
}

fn qualified_kind(kind: &str) -> Result<(), String> {
    match kind.split_once(':') {
        Some((source, kind)) if !source.is_empty() && !kind.is_empty() && !kind.contains(':') => {
            Ok(())
        }
        _ => Err(format!(
            "expected a source-qualified kind 'source:kind', got '{kind}'"
        )),
    }
}

/// Read a fuse profile: cross-source `edge_kinds` and validations.
///
/// Edge-kind endpoints are `source:kind` pairs. Refuses `node_kinds` because
/// those belong to the source profiles; a copy here would drift from them.
///
/// # Errors
///
/// [`FuseError::Read`] or [`FuseError::Yaml`] for a file that cannot be read or
/// parsed, and [`FuseError::FuseProfile`] for a profile that is not a mapping,
/// has an unknown key, declares `node_kinds` or `annotations`, or has a
/// malformed edge kind or validation.
pub fn load_fuse_profile(path: &Path) -> Result<FuseProfile, FuseError> {
    let Value::Object(top) = load_yaml(path)? else {
        return Err(FuseError::FuseProfile(
            "fuse profile must be a YAML mapping".into(),
        ));
    };
    check_fuse_profile(top).map_err(FuseError::FuseProfile)
}

fn check_fuse_profile(mut top: Map<String, Value>) -> Result<FuseProfile, String> {
    const KNOWN: &[&str] = &[
        "annotations",
        "name",
        "profile_version",
        "edge_kinds",
        "node_kinds",
        "validations",
    ];
    for key in top.keys() {
        if !KNOWN.contains(&key.as_str()) {
            return Err(format!("fuse profile: unknown key '{key}'"));
        }
    }
    if top.contains_key("annotations") {
        return Err(
            "fuse profile must not declare 'annotations'; notes belong to source profiles".into(),
        );
    }
    if top.contains_key("node_kinds") {
        return Err(
            "fuse profile must not declare 'node_kinds'; node kinds belong to source profiles"
                .into(),
        );
    }
    let edges: BTreeMap<String, EdgeDeclaration> = serde_json::from_value(
        top.remove("edge_kinds")
            .ok_or("fuse profile requires 'edge_kinds'")?,
    )
    .map_err(|e| format!("fuse profile edge_kinds: {e}"))?;
    for (name, edge) in &edges {
        nonempty(name, "edge kind")?;
        for (src, tgt) in &edge.allowed {
            qualified_kind(src)?;
            qualified_kind(tgt)?;
        }
    }
    let mut validations = Vec::new();
    if let Some(raw) = top.remove("validations").filter(|v| !v.is_null()) {
        let Value::Array(entries) = raw else {
            return Err("'validations' must be a list".into());
        };
        // Every entry is checked and every fault kept, as the profile loader
        // does, so one refusal names them all.
        let mut faults: Vec<String> = Vec::new();
        for entry in entries {
            match check_fuse_validation(entry) {
                Ok(checked) => validations.push(checked),
                Err(fault) => faults.push(fault),
            }
        }
        if !faults.is_empty() {
            return Err(faults.join("\n"));
        }
    }
    Ok(FuseProfile { edges, validations })
}

/// Check one fuse-profile validation entry, returning its code and config.
fn check_fuse_validation(entry: Value) -> Result<(String, Map<String, Value>), String> {
    let (code, config) = match entry {
        Value::Object(entry) if entry.len() == 1 => entry.into_iter().next().expect("one entry"),
        _ => return Err("expected one validation code mapping".into()),
    };
    let Value::Object(mut config) = config else {
        return Err("validation config must be a mapping".into());
    };
    if let Some(refusal) = crate::profile::retired_code_refusal(&code) {
        return Err(format!("fuse profile: {refusal}"));
    }
    if config.contains_key("message") {
        return Err(format!(
            "fuse profile: validation entry '{code}': 'message' is not supported in a fuse \
             profile; declare it in the source profile"
        ));
    }
    if let Some(severity) = config.get("severity").filter(|v| !v.is_null()) {
        let text = severity.as_str().ok_or_else(|| {
            format!("fuse profile: validation override for '{code}': 'severity' must be a string")
        })?;
        crate::profile::parse_severity_word(text, &code).map_err(|diagnostic| {
            format!("fuse profile: validation override for '{code}': {diagnostic}")
        })?;
    }
    if code == "COVERAGE" {
        const COVERAGE_KEYS: &[&str] = &[
            "severity",
            "target_kind",
            "edge_kind",
            "where",
            "source_where",
        ];
        let mut target_kind = "";
        for key in ["target_kind", "edge_kind"] {
            let value = config
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("COVERAGE requires '{key}'"))?;
            nonempty(value, key)?;
            if key == "target_kind" {
                target_kind = value;
            }
        }
        qualified_kind(target_kind)?;
        for key in config.keys() {
            if !COVERAGE_KEYS.contains(&key.as_str()) {
                return Err(format!("COVERAGE: unknown config key '{key}'"));
            }
        }
        for key in ["where", "source_where"] {
            if let Some(where_val) = config.get(key).filter(|v| !v.is_null()) {
                let norway_val = serde_norway::to_value(where_val)
                    .map_err(|e| format!("COVERAGE {key}: {e}"))?;
                crate::profile::parse_condition_block(Some(&norway_val), "COVERAGE", key)
                    .map_err(|e| format!("fuse profile: {e}"))?;
            }
        }
    }
    if code == "SUPPRESS" {
        // The same rules as a source profile, checked here so a
        // CONFIG_ERROR suppression fails before any source runs.
        let norway =
            serde_norway::to_value(&config).map_err(|e| format!("fuse profile SUPPRESS: {e}"))?;
        let serde_norway::Value::Mapping(mapping) = &norway else {
            return Err("fuse profile SUPPRESS: config did not convert to a mapping".into());
        };
        crate::profile::parse_suppress(mapping).map_err(|e| format!("fuse profile: {e}"))?;
    }
    config.retain(|_, v| !v.is_null());
    Ok((code, config))
}

impl FuseProfile {
    pub(super) fn severity(&self, code: &str, default: Severity) -> Severity {
        let override_ = self
            .validations
            .iter()
            .filter(|(c, _)| c == code)
            .filter_map(|(_, config)| {
                config
                    .get("severity")
                    .and_then(Value::as_str)
                    .and_then(Severity::parse)
            })
            .next_back();
        resolve_severity(default, override_)
    }
}
