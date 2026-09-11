use std::collections::BTreeMap;

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

use super::{capitalize, ljust, render_json, rjust, truncate};
use crate::types::{FindingTally, StatusRollup, StructuralSummary};

fn count(counts: &BTreeMap<String, i64>, key: &str) -> i64 {
    counts.get(key).copied().unwrap_or(0)
}

fn columns(status_keys: &[String], counts: &BTreeMap<String, i64>) -> String {
    status_keys
        .iter()
        .map(|k| rjust(&count(counts, k).to_string(), 5))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn format_summary_plain(report: &StatusRollup) -> String {
    report
        .groups
        .iter()
        .map(|(group, counts)| {
            let parts: Vec<String> = report
                .status_keys
                .iter()
                .map(|k| format!("{k}={}", count(counts, k)))
                .collect();
            format!(
                "{group}: {} total={}",
                parts.join(" "),
                count(counts, "total")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct SummaryRowsJson<'a>(&'a StatusRollup);

struct SummaryRowJson<'a> {
    group_key: &'a str,
    group: &'a str,
    counts: &'a BTreeMap<String, i64>,
}

impl Serialize for SummaryRowJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let has_collision = self.counts.contains_key(self.group_key);
        let mut mapping =
            serializer.serialize_map(Some(self.counts.len() + usize::from(!has_collision)))?;
        let mut inserted_group = has_collision;
        for (key, value) in self.counts {
            if !inserted_group && self.group_key < key.as_str() {
                mapping.serialize_entry(self.group_key, self.group)?;
                inserted_group = true;
            }
            mapping.serialize_entry(key, value)?;
        }
        if !inserted_group {
            mapping.serialize_entry(self.group_key, self.group)?;
        }
        mapping.end()
    }
}

impl Serialize for SummaryRowsJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.groups.len()))?;
        for (group, counts) in &self.0.groups {
            sequence.serialize_element(&SummaryRowJson {
                group_key: &self.0.group_key,
                group,
                counts,
            })?;
        }
        sequence.end()
    }
}

#[derive(serde::Serialize)]
struct SummaryJson<'a> {
    groups: SummaryRowsJson<'a>,
    totals: &'a BTreeMap<String, i64>,
}

pub(super) fn format_summary_json(report: &StatusRollup) -> String {
    let totals = report.totals();
    render_json(&SummaryJson {
        groups: SummaryRowsJson(report),
        totals: &totals,
    })
}

pub(super) fn format_summary_rich(report: &StatusRollup) -> String {
    let mut labels: Vec<String> = report
        .status_keys
        .iter()
        .map(|k| truncate(&capitalize(k), 5))
        .collect();
    labels.push("Total".to_string());

    let header = format!(
        "{} {}",
        ljust(&capitalize(&report.group_key), 30),
        labels
            .iter()
            .map(|l| rjust(l, 5))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let rule = "-".repeat(header.chars().count());

    let mut lines = vec![header, rule.clone()];
    for (group, counts) in &report.groups {
        lines.push(format!(
            "{} {} {}",
            ljust(group, 30),
            columns(&report.status_keys, counts),
            rjust(&count(counts, "total").to_string(), 5)
        ));
    }

    let totals = report.totals();
    lines.push(rule);
    lines.push(format!(
        "{} {} {}",
        ljust("TOTAL", 30),
        columns(&report.status_keys, &totals),
        rjust(&count(&totals, "total").to_string(), 5)
    ));
    lines.join("\n")
}

pub(super) fn format_structural_plain(report: &StructuralSummary) -> String {
    let mut lines = Vec::new();
    for (kind, n) in &report.node_counts {
        lines.push(format!("node {kind} {n}"));
    }
    for (kind, n) in &report.edge_counts {
        lines.push(format!("edge {kind} {n}"));
    }
    for tally in &report.finding_counts {
        lines.push(format!(
            "finding {} {} {}",
            tally.code,
            tally.severity.as_str(),
            tally.count
        ));
    }
    lines.join("\n")
}

impl Serialize for FindingTally {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut mapping = serializer.serialize_map(Some(3))?;
        mapping.serialize_entry("code", &self.code)?;
        mapping.serialize_entry("count", &self.count)?;
        mapping.serialize_entry("severity", self.severity.as_str())?;
        mapping.end()
    }
}

pub(super) fn format_structural_json(report: &StructuralSummary) -> String {
    #[derive(serde::Serialize)]
    struct StructuralJson<'a> {
        edge_counts: &'a BTreeMap<String, i64>,
        finding_counts: &'a [FindingTally],
        node_counts: &'a BTreeMap<String, i64>,
    }

    render_json(&StructuralJson {
        edge_counts: &report.edge_counts,
        finding_counts: &report.finding_counts,
        node_counts: &report.node_counts,
    })
}

pub(super) fn format_structural_rich(report: &StructuralSummary) -> String {
    let row = |label: &str, n: i64| format!("  {} {}", ljust(label, 30), rjust(&n.to_string(), 5));
    let mut lines = vec!["Nodes".to_string()];
    lines.extend(report.node_counts.iter().map(|(kind, n)| row(kind, *n)));
    lines.push("Edges".to_string());
    lines.extend(report.edge_counts.iter().map(|(kind, n)| row(kind, *n)));
    lines.push("Findings".to_string());
    lines.extend(report.finding_counts.iter().map(|tally| {
        row(
            &format!("{} {}", tally.code, tally.severity.as_str()),
            tally.count,
        )
    }));
    lines.join("\n")
}
