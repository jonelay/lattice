use serde::ser::{Serialize, SerializeSeq, Serializer};

use super::{FindingJson, RESET, color_for, ljust, render_json};
use crate::types::{Issue, Severity};

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

pub(super) fn format_plain(issues: &[&Issue]) -> String {
    issues
        .iter()
        .filter(|i| !i.suppressed)
        .map(|i| {
            format!(
                "{} {} {} {}",
                i.severity.as_upper(),
                i.code,
                i.provenance,
                i.message
            )
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
        .map(|i| {
            format!(
                "{}{}{} {} {} {}",
                color_for(i.severity),
                ljust(i.severity.as_upper(), 7),
                RESET,
                ljust(&i.code, 20),
                ljust(&i.provenance.to_string(), 30),
                i.message
            )
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
