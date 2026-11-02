use serde::ser::{Serialize, SerializeSeq, Serializer};

use std::collections::BTreeMap;

use super::{FindingJson, RESET, color_for, ljust, note_text, render_json, with_plain_note};
use crate::types::{Issue, Severity, ValidateReport, ValidateScope};

struct FindingsJson<'a>(pub(super) &'a [&'a Issue]);

impl Serialize for FindingsJson<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for issue in self.0 {
            sequence.serialize_element(&FindingJson::from(*issue))?;
        }
        sequence.end()
    }
}

#[derive(serde::Serialize)]
struct FindingsRoot<'a> {
    findings: FindingsJson<'a>,
}

#[derive(serde::Serialize)]
struct ScopeJson<'a> {
    edges: &'a BTreeMap<String, usize>,
    files: usize,
    nodes: &'a BTreeMap<String, usize>,
}

impl<'a> From<&'a ValidateScope> for ScopeJson<'a> {
    fn from(scope: &'a ValidateScope) -> Self {
        Self {
            edges: &scope.edges,
            files: scope.files,
            nodes: &scope.nodes,
        }
    }
}

/// `validate`'s JSON: the findings as before, and beside them what was read.
#[derive(serde::Serialize)]
struct ValidateRoot<'a> {
    findings: FindingsJson<'a>,
    scope: ScopeJson<'a>,
}

/// Findings in `validate`'s order, whatever order a caller appended them in.
fn sorted(issues: &[Issue]) -> Vec<&Issue> {
    let mut sorted: Vec<&Issue> = issues.iter().collect();
    sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    sorted
}

pub(super) fn format_report_plain(report: &ValidateReport) -> String {
    format_plain(&sorted(&report.findings))
}

pub(super) fn format_report_json(report: &ValidateReport) -> String {
    render_json(&ValidateRoot {
        findings: FindingsJson(&sorted(&report.findings)),
        scope: ScopeJson::from(&report.scope),
    })
}

pub(super) fn format_report_rich(report: &ValidateReport) -> String {
    format_rich(&sorted(&report.findings))
}

/// The one line `plain` and `rich` write to stderr after the findings:
/// `scope: <files> files; nodes <kind> <n>, …; edges <kind> <n>, …`.
///
/// Defined beside the other renderings so the text has one owner; the CLI
/// writes it because the dispatcher returns one string for one stream.
#[must_use]
pub fn scope_line(scope: &ValidateScope) -> String {
    let counts = |map: &BTreeMap<String, usize>| {
        map.iter()
            .map(|(kind, n)| format!("{kind} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "scope: {} files; nodes {}; edges {}",
        scope.files,
        counts(&scope.nodes),
        counts(&scope.edges)
    )
}

pub(super) fn format_plain(issues: &[&Issue]) -> String {
    issues
        .iter()
        .filter(|i| !i.suppressed)
        .map(|i| {
            let line = format!(
                "{} {} {} {}",
                i.severity.as_upper(),
                i.code,
                i.origin,
                i.message
            );
            with_plain_note(line, i)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn format_json(issues: &[&Issue]) -> String {
    render_json(&FindingsRoot {
        findings: FindingsJson(issues),
    })
}

pub(super) fn format_rich(issues: &[&Issue]) -> String {
    let issues: Vec<&Issue> = issues.iter().copied().filter(|i| !i.suppressed).collect();
    if issues.is_empty() {
        return "No findings.".to_string();
    }

    let mut lines: Vec<String> = issues
        .iter()
        .flat_map(|i| {
            let line = format!(
                "{}{}{} {} {} {}",
                color_for(i.severity),
                ljust(i.severity.as_upper(), 7),
                RESET,
                ljust(&i.code, 20),
                ljust(&i.origin.to_string(), 30),
                i.message
            );
            std::iter::once(line)
                .chain(i.note.as_ref().map(|note| format!("  {}", note_text(note))))
        })
        .collect();

    let mut parts = Vec::new();
    for severity in [
        Severity::Error,
        Severity::Warning,
        Severity::Info,
        Severity::Hint,
    ] {
        let count = issues.iter().filter(|i| i.severity == severity).count();
        if count > 0 {
            parts.push(format!("{count} {}(s)", severity.as_str()));
        }
    }
    lines.push(format!("\n{}", parts.join(", ")));
    lines.join("\n")
}
