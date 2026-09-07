//! Building the status rollup `summary` reports.
//!
//! The rollup is computed from the register on every run and never stored. What
//! it counts is entirely the profile's business: which kind, which attr holds a
//! status, which attr groups the rows.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::graph::LatticeGraph;
use crate::profile::Profile;
use crate::types::SummaryReport;
use crate::validate::config_str;

/// A profile that cannot be summarised. Operational rather than a finding: the
/// command has nothing to render, so it exits 2 rather than reporting an
/// empty rollup.
#[derive(Debug)]
pub struct SummaryError(pub String);

impl std::fmt::Display for SummaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SummaryError {}

/// The status a node with no value for the configured attr counts under, and the
/// group an ungrouped node lands in.
const UNKNOWN: &str = "unknown";

/// Compute the rollup a profile's `SUMMARY` config declares.
///
/// Status columns come from the profile's declared enum *and* from what the
/// register actually holds, so a declared status no node carries still gets a
/// zero column and an undeclared one still gets counted.
pub fn build_summary(
    profile: &Profile,
    graph: &LatticeGraph,
) -> Result<SummaryReport, SummaryError> {
    let configs = profile
        .validation_configs
        .get("SUMMARY")
        .map_or(&[][..], Vec::as_slice);
    let config = match configs {
        [] => {
            return Err(SummaryError(
                "profile has no SUMMARY validation config".into(),
            ));
        }
        [only] => only,
        many => {
            // Picking the first would silently discard a config the profile
            // declared, which is the failure this tool exists to prevent.
            return Err(SummaryError(format!(
                "profile declares {} SUMMARY configs; summary renders one rollup \
                 and cannot choose between them",
                many.len()
            )));
        }
    };

    let required = |key: &str| match config_str(config, key) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(SummaryError(
            "SUMMARY config requires node_kind, status_attr, group_by_attr".into(),
        )),
        // A mistyped value is not an absence: rolling up nothing at exit 0
        // would be silence about a config the profile declared.
        Err(got) => Err(SummaryError(format!(
            "SUMMARY config: '{key}' must be a string, got {got}"
        ))),
    };
    let node_kind = required("node_kind")?;
    let status_attr = required("status_attr")?;
    let group_by_attr = required("group_by_attr")?;

    let mut status_values: BTreeSet<String> = profile
        .node_kinds
        .get(&node_kind)
        .and_then(|kind| kind.attrs.get(&status_attr))
        .and_then(|attr| attr.values.as_deref())
        .unwrap_or_default()
        .iter()
        .cloned()
        .collect();

    let of_kind = || graph.iter_nodes().filter(|n| n.kind == node_kind);
    for node in of_kind() {
        status_values.insert(attr_or_unknown(node.attrs.get(&status_attr)));
    }
    let status_keys: Vec<String> = status_values.iter().cloned().collect();

    // Every group carries a column for every status, so a declared status with
    // no nodes reads as a zero rather than as an absent key downstream.
    let blank: BTreeMap<String, i64> = status_keys
        .iter()
        .cloned()
        .chain(["total".to_string()])
        .map(|k| (k, 0))
        .collect();

    let mut by_group: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    for node in of_kind() {
        let group = attr_or_unknown(node.attrs.get(&group_by_attr));
        let status = attr_or_unknown(node.attrs.get(&status_attr));
        let counts = by_group.entry(group).or_insert_with(|| blank.clone());
        *counts.entry(status).or_insert(0) += 1;
        *counts.entry("total".to_string()).or_insert(0) += 1;
    }

    Ok(SummaryReport {
        group_key: group_by_attr,
        status_keys,
        groups: by_group.into_iter().collect(),
    })
}

/// An attr's value as the rollup keys on it, or `unknown` when the node has none.
fn attr_or_unknown(value: Option<&Value>) -> String {
    match value {
        None => UNKNOWN.to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}
