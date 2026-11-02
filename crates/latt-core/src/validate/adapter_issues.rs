//! Resolving adapter findings: severity overrides, axis resolution, faults.

use std::collections::BTreeSet;

use crate::graph::LatticeGraph;
use crate::profile::Profile;
use crate::types::{Issue, Origin, Severity};

use super::{FindingCode, default_severity, issue, resolve_axes};

/// Apply the profile's severity overrides to issues emitted outside core.
///
/// Guards on the severity an issue *arrives* at rather than on its code's
/// shipped default: core has no default for a code it does not implement, so
/// an externally emitted hint would otherwise be promoted with nothing said.
pub(super) fn apply_overrides(issues: &[Issue], profile: &Profile) -> Vec<Issue> {
    let mut refused: BTreeSet<&str> = BTreeSet::new();
    let mut out: Vec<Issue> = issues
        .iter()
        .map(|i| {
            let mut out = i.clone();
            if let Some(o) = profile.validation_overrides().get(&i.code).copied() {
                if i.severity == Severity::Hint && o != Severity::Hint {
                    // A code core ships as hint is already reported by the
                    // pre-pass in `validate`, which sees the override whether or
                    // not a finding arrives under it. Reporting it here too
                    // would name one fault twice.
                    if FindingCode::from_str(&i.code)
                        .is_none_or(|code| default_severity(code) != Severity::Hint)
                    {
                        refused.insert(i.code.as_str());
                    }
                } else {
                    out.severity = o;
                }
            }
            out
        })
        .collect();

    // One per code, not per finding: the fault is the override, and repeating it
    // per finding would bury the findings it reports about.
    out.extend(refused.into_iter().map(|code| {
        issue(
            FindingCode::ConfigError,
            format!(
                "severity override for '{code}': cannot promote from hint; \
                 the override is ignored"
            ),
            Origin::Profile,
            profile,
            None,
        )
    }));
    out
}

/// Adapter issues at the severities the profile and its pathways decide.
#[must_use]
pub fn resolve_adapter_issues(graph: &LatticeGraph, profile: &Profile) -> Vec<Issue> {
    let mut issues = apply_overrides(graph.adapter_issues(), profile);
    resolve_axes(&mut issues, graph, profile);
    issues
}
