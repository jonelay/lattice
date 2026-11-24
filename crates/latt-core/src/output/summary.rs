use std::collections::BTreeMap;

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

use super::{capitalize, ljust, render_json, rjust, sanitize_text, truncate};
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
                .map(|k| format!("{}={}", sanitize_text(k), count(counts, k)))
                .collect();
            format!(
                "{}: {} total={}",
                sanitize_text(group),
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
        // Cut before escaping, so the cut never lands inside an escape.
        .map(|k| sanitize_text(&truncate(&capitalize(k), 5)).into_owned())
        .collect();
    labels.push("Total".to_string());

    let header = format!(
        "{} {}",
        ljust(&sanitize_text(&capitalize(&report.group_key)), 30),
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
            ljust(&sanitize_text(group), 30),
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
        lines.push(format!("node {} {n}", sanitize_text(kind)));
    }
    for (kind, n) in &report.edge_counts {
        lines.push(format!("edge {} {n}", sanitize_text(kind)));
    }
    for tally in &report.finding_counts {
        lines.push(format!(
            "finding {} {} {}",
            sanitize_text(&tally.code),
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
    let row = |label: &str, n: i64| {
        format!(
            "  {} {}",
            ljust(&sanitize_text(label), 30),
            rjust(&n.to_string(), 5)
        )
    };
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::format_summary_rich;
    use crate::types::StatusRollup;

    // Requirement: output/Untrusted text in human formats
    // A status header is capitalised and cut to five characters before it is
    // escaped, so the cut never leaves half an escape (`Sum\u`).
    #[test]
    fn rich_summary_cuts_a_status_header_before_escaping_it() {
        let status = "sum\u{1b}[2Jmary".to_string();
        let report = StatusRollup {
            group_key: "stage".into(),
            status_keys: vec![status.clone()],
            groups: vec![("g".into(), BTreeMap::from([(status, 1)]))],
        };
        let rich = format_summary_rich(&report);
        assert!(rich.contains("Sum\\u{1b}["), "{rich}");
        assert!(!rich.contains("\u{1b}[2"), "{rich}");
    }
}
