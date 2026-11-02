use super::{RESET, color_for, ljust, render_json, rjust, validate};
use crate::types::{DescribeReport, Issue, Severity};

/// Indent a YAML block two spaces per line, so it reads as a child of its label.
fn indented(block: &str) -> String {
    block
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn pairs(allowed: &[(String, String)]) -> String {
    allowed
        .iter()
        .map(|(src, tgt)| format!("{src}->{tgt}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn format_describe_plain(report: &DescribeReport) -> String {
    let mut lines = vec![
        format!("profile {} {}", report.profile.name, report.profile.version),
        format!("adapter {}", report.adapter.name),
    ];
    if let Some(config) = &report.adapter.config_yaml {
        lines.push("adapter.config:".to_owned());
        lines.push(indented(config));
    }
    for kind in &report.node_kinds {
        let mut line = format!("node {} {} {}", kind.kind, kind.id_pattern, kind.count);
        if !kind.files.is_empty() {
            line.push_str(&format!(" files {}", kind.files.join(", ")));
        }
        lines.push(line);
    }
    for kind in &report.edge_kinds {
        lines.push(format!(
            "edge {} {} allowed {}",
            kind.kind,
            kind.count,
            pairs(&kind.allowed)
        ));
    }
    for validation in &report.validations {
        let mut line = format!("validation {}", validation.code);
        if let Some(severity) = validation.severity {
            line.push_str(&format!(" severity {}", severity.as_str()));
        }
        lines.push(line);
    }
    let findings: Vec<&Issue> = report.findings.iter().collect();
    if !findings.is_empty() {
        lines.push(validate::format_plain(&findings));
    }
    lines.join("\n")
}

pub(super) fn format_describe_json(report: &DescribeReport) -> String {
    #[derive(serde::Serialize)]
    struct Profile<'a> {
        name: &'a str,
        version: &'a str,
    }
    #[derive(serde::Serialize)]
    struct Adapter<'a> {
        config: Option<super::sorted::SortedValue<'a>>,
        name: &'a str,
    }
    #[derive(serde::Serialize)]
    struct NodeKind<'a> {
        count: usize,
        files: &'a [String],
        id_pattern: &'a str,
        kind: &'a str,
    }
    #[derive(serde::Serialize)]
    struct EdgeKind<'a> {
        allowed: Vec<[&'a str; 2]>,
        count: usize,
        kind: &'a str,
    }
    #[derive(serde::Serialize)]
    struct Validation<'a> {
        code: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        severity: Option<&'a str>,
    }
    #[derive(serde::Serialize)]
    struct Root<'a> {
        adapter: Adapter<'a>,
        edge_kinds: Vec<EdgeKind<'a>>,
        findings: Vec<super::FindingJson<'a>>,
        node_kinds: Vec<NodeKind<'a>>,
        profile: Profile<'a>,
        validations: Vec<Validation<'a>>,
    }
    render_json(&Root {
        adapter: Adapter {
            config: report
                .adapter
                .config
                .as_ref()
                .map(super::sorted::SortedValue),
            name: &report.adapter.name,
        },
        edge_kinds: report
            .edge_kinds
            .iter()
            .map(|kind| EdgeKind {
                allowed: kind
                    .allowed
                    .iter()
                    .map(|(src, tgt)| [src.as_str(), tgt.as_str()])
                    .collect(),
                count: kind.count,
                kind: &kind.kind,
            })
            .collect(),
        findings: report
            .findings
            .iter()
            .map(super::FindingJson::from)
            .collect(),
        node_kinds: report
            .node_kinds
            .iter()
            .map(|kind| NodeKind {
                count: kind.count,
                files: &kind.files,
                id_pattern: &kind.id_pattern,
                kind: &kind.kind,
            })
            .collect(),
        profile: Profile {
            name: &report.profile.name,
            version: &report.profile.version,
        },
        validations: report
            .validations
            .iter()
            .map(|validation| Validation {
                code: &validation.code,
                severity: validation.severity.map(Severity::as_str),
            })
            .collect(),
    })
}

pub(super) fn format_describe_rich(report: &DescribeReport) -> String {
    let mut lines = vec![
        format!(
            "Profile {} ({})",
            report.profile.name, report.profile.version
        ),
        format!("Adapter {}", report.adapter.name),
    ];
    if let Some(config) = &report.adapter.config_yaml {
        lines.push(indented(config));
    }
    lines.push("Node kinds".to_owned());
    for kind in &report.node_kinds {
        lines.push(format!(
            "  {} {} {}",
            ljust(&kind.kind, 20),
            rjust(&kind.count.to_string(), 5),
            kind.id_pattern
        ));
        for file in &kind.files {
            lines.push(format!("    {file}"));
        }
    }
    lines.push("Edge kinds".to_owned());
    for kind in &report.edge_kinds {
        lines.push(format!(
            "  {} {} {}",
            ljust(&kind.kind, 20),
            rjust(&kind.count.to_string(), 5),
            pairs(&kind.allowed)
        ));
    }
    lines.push("Validations".to_owned());
    for validation in &report.validations {
        let severity = validation.severity.map_or_else(String::new, |severity| {
            format!("{}{}{}", color_for(severity), severity.as_str(), RESET)
        });
        lines.push(format!("  {} {severity}", ljust(&validation.code, 20)));
    }
    lines.push("Findings".to_owned());
    let findings: Vec<&Issue> = report.findings.iter().collect();
    lines.push(indented(&validate::format_rich(&findings)));
    lines.join("\n")
}
