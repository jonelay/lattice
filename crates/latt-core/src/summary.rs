//! Building the reports `summary` shows.
//!
//! Both are computed from the register on every run and never stored. What the
//! configured rollup counts is entirely the profile's business: which kind,
//! which attr holds a status, which attr groups the rows. The structural report
//! needs none of that; counts by kind and finding tallies come from the graph
//! and the profile's kind declarations alone.

#![deny(missing_docs, clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::graph::LatticeGraph;
use crate::profile::{PlanEntry, Profile, ValidationSpec};
use crate::query::{counts, tally};
use crate::types::{
    CivilDate, FindingTally, Issue, Severity, StatusRollup, StructuralSummary, SummaryReport,
};
use crate::validate::validate_on;

/// A profile that cannot be summarized. Operational rather than a finding: there
/// is nothing to render, so this is returned instead of an empty rollup. The CLI
/// maps it to exit 2.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
#[non_exhaustive]
pub struct SummaryError {
    message: String,
}

impl SummaryError {
    fn new(message: impl Into<String>) -> Self {
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

    /// Return the explanation as an owned string, consuming the error.
    #[must_use]
    pub fn into_message(self) -> String {
        self.message
    }
}

/// The status a node with no value for the configured attr counts under, and the
/// group an ungrouped node lands in.
/// A node whose value is literally `unknown` counts under the same key; the
/// rollup does not distinguish the two.
const UNKNOWN: &str = "unknown";

/// The synthetic column each row's count across every status is kept under.
const TOTAL: &str = "total";

/// Compute the rollup a profile's `SUMMARY` config declares, or the structural
/// report when it declares none.
///
/// Status columns come from the profile's declared enum *and* from what the
/// register actually holds, so a declared status no node carries still gets a
/// zero column and an undeclared one still gets counted.
///
/// The structural report's finding tallies expire notes against today's UTC
/// date; [`build_summary_on`] takes the date.
///
/// # Errors
///
/// [`SummaryError`] when the profile declares more than one `SUMMARY` config,
/// when the config lacks or mistypes `node_kind`, `status_attr` or
/// `group_by_attr`, or when a status value is named `total`.
pub fn build_summary(
    profile: &Profile,
    graph: &LatticeGraph,
) -> Result<SummaryReport, SummaryError> {
    build_summary_on(profile, graph, CivilDate::today_utc())
}

/// Build the summary as [`build_summary`] does, expiring finding notes against
/// `today`.
///
/// # Errors
///
/// As [`build_summary`].
pub fn build_summary_on(
    profile: &Profile,
    graph: &LatticeGraph,
    today: CivilDate,
) -> Result<SummaryReport, SummaryError> {
    let entries: Vec<&PlanEntry> = profile
        .validation_plan()
        .entries()
        .iter()
        .filter(|entry| entry.code == "SUMMARY")
        .collect();
    let entry = match entries.as_slice() {
        [] => {
            let issues = validate_on(graph, profile, false, today);
            return Ok(SummaryReport::Structural(build_structural_summary(
                profile, graph, &issues,
            )));
        }
        [only] => *only,
        many => {
            // Picking the first would silently discard a config the profile
            // declared, which is the failure this tool exists to prevent.
            return Err(SummaryError::new(format!(
                "profile declares {} SUMMARY configs; summary renders one rollup \
                 and cannot choose between them",
                many.len()
            )));
        }
    };
    let spec = match &entry.spec {
        ValidationSpec::Summary(spec) => spec,
        // The loader kept the entry so `validate` could run; the fault is
        // summary's to report.
        ValidationSpec::Faulty(faults) => return Err(SummaryError::new(faults.join("; "))),
        other => {
            return Err(SummaryError::new(format!(
                "SUMMARY entry compiled to {other:?}, which summary cannot run"
            )));
        }
    };
    let (node_kind, status_attr, group_by_attr) = (
        spec.node_kind.as_str(),
        spec.status_attr.as_str(),
        spec.group_by_attr.as_str(),
    );

    let mut status_values: BTreeSet<String> = profile
        .node_kinds()
        .get(node_kind)
        .and_then(|kind| kind.attrs.get(status_attr))
        .and_then(|attr| attr.values.as_deref())
        .unwrap_or_default()
        .iter()
        .cloned()
        .collect();

    let of_kind = || graph.iter_nodes().filter(|n| n.kind == node_kind);
    for node in of_kind() {
        let status = attr_or_unknown(node.attrs.get(status_attr));
        if !status_values.contains(status.as_ref()) {
            status_values.insert(status.into_owned());
        }
    }
    if status_values.contains(TOTAL) {
        // Rows key statuses and the synthetic total in one map, so a status by
        // this name would be counted twice and shown as two columns.
        return Err(SummaryError::new(format!(
            "SUMMARY: status value '{TOTAL}' in '{status_attr}' collides with the \
             rollup's total column"
        )));
    }
    let status_keys: Vec<String> = status_values.iter().cloned().collect();

    // Every group carries a column for every status, so a declared status with
    // no nodes reads as a zero rather than as an absent key downstream.
    let blank: BTreeMap<String, i64> = status_keys
        .iter()
        .cloned()
        .chain([TOTAL.to_string()])
        .map(|k| (k, 0))
        .collect();

    let mut by_group: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    // Keys are owned once per distinct group, not once per node.
    for node in of_kind() {
        let group = attr_or_unknown(node.attrs.get(group_by_attr));
        let status = attr_or_unknown(node.attrs.get(status_attr));
        if !by_group.contains_key(group.as_ref()) {
            by_group.insert(group.clone().into_owned(), blank.clone());
        }
        let Some(counts) = by_group.get_mut(group.as_ref()) else {
            continue;
        };
        tally(counts, &status);
        tally(counts, TOTAL);
    }

    Ok(SummaryReport::Configured(StatusRollup {
        group_key: group_by_attr.to_string(),
        status_keys,
        groups: by_group.into_iter().collect(),
    }))
}

/// Tally the register by kind and the run's findings by code and severity.
///
/// Suppressed findings are left out: the profile said not to report them, and
/// a tally is a report. The severity is the resolved one, so a profile override
/// moves a code's row rather than being second-guessed here.
#[must_use]
#[doc(hidden)]
pub fn build_structural_summary(
    profile: &Profile,
    graph: &LatticeGraph,
    issues: &[Issue],
) -> StructuralSummary {
    let counted = counts(graph, profile, &[]);
    let mut tallies: BTreeMap<(&str, Severity), i64> = BTreeMap::new();
    for issue in issues.iter().filter(|i| !i.suppressed) {
        *tallies.entry((&issue.code, issue.severity)).or_insert(0) += 1;
    }
    StructuralSummary {
        node_counts: counted.nodes,
        edge_counts: counted.edges,
        finding_counts: tallies
            .into_iter()
            .map(|((code, severity), count)| FindingTally {
                code: code.to_string(),
                severity,
                count,
            })
            .collect(),
    }
}

/// An attr's value as the rollup keys on it, or `unknown` when the node has none.
fn attr_or_unknown(value: Option<&Value>) -> Cow<'_, str> {
    match value {
        None => Cow::Borrowed(UNKNOWN),
        Some(Value::String(text)) => Cow::Borrowed(text),
        Some(other) => Cow::Owned(other.to_string()),
    }
}
