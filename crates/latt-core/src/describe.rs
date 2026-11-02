//! `describe`: what a profile declares and what the adapter read under it, in
//! one answer, so orienting on a register needs neither the profile file nor a
//! run of counting commands.
//!
//! Nothing here is validated. Counts and files come from the graph's own
//! provenance through the kind tally `validate` also reads; the `adapter:` block
//! is reproduced as the adapter receives it, so the core interprets no adapter key.

use crate::graph::LatticeGraph;
use crate::profile::{Profile, ProfileError, to_json};
use crate::scope::KindTally;
use crate::types::{
    DescribeAdapter, DescribeEdgeKind, DescribeNodeKind, DescribeProfile, DescribeReport,
    DescribeValidation,
};
use crate::validate::{apply_messages, apply_suppressions, resolve_adapter_issues};

/// Build the orientation report for `graph` as `profile` declares it; `adapter`
/// is the adapter that produced the graph, as the user named it or the profile
/// resolved it.
///
/// # Errors
///
/// [`ProfileError::Invalid`] when the profile's `adapter:` block holds a value
/// JSON cannot carry, as [`resolved_document`](crate::profile::resolved_document)
/// refuses it.
#[doc(hidden)]
pub fn build_describe_report(
    profile: &Profile,
    graph: &LatticeGraph,
    adapter: String,
) -> Result<DescribeReport, ProfileError> {
    let tally = KindTally::of(graph, profile);
    let node_kinds = profile
        .node_kinds()
        .iter()
        .map(|(kind, declared)| {
            let (count, files) = tally.nodes.get(kind).cloned().unwrap_or_default();
            DescribeNodeKind {
                kind: kind.clone(),
                id_pattern: declared.id_pattern_source.clone(),
                count,
                files: files.into_iter().collect(),
            }
        })
        .collect();
    let edge_kinds = profile
        .edge_kinds()
        .iter()
        .map(|(kind, declared)| DescribeEdgeKind {
            kind: kind.clone(),
            allowed: declared.allowed.clone(),
            count: tally.edges.get(kind).copied().unwrap_or_default(),
        })
        .collect();
    // By code, declaration order within a code: the question is "what is
    // configured for X", and a profile's own order answers nothing a reader
    // asks here.
    let mut validations: Vec<DescribeValidation> = profile
        .validation_plan()
        .entries()
        .iter()
        .map(|entry| DescribeValidation {
            code: entry.code.clone(),
            severity: entry.severity,
        })
        .collect();
    validations.sort_by(|a, b| a.code.cmp(&b.code));
    // The adapter's own findings as `validate` reports them: resolved
    // severities, the profile's suppressions, then its messages, in
    // `validate`'s order.
    let mut findings = resolve_adapter_issues(graph, profile);
    apply_suppressions(findings.iter_mut(), profile);
    apply_messages(findings.iter_mut(), profile, graph);
    findings.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    let (config, config_yaml) = match profile.adapter_block() {
        Some(block) => {
            let yaml = serde_norway::to_string(block).map_err(|e| {
                ProfileError::Invalid(format!("could not render the adapter: block: {e}"))
            })?;
            (Some(to_json(block)?), Some(yaml))
        }
        None => (None, None),
    };
    Ok(DescribeReport {
        profile: DescribeProfile {
            name: profile.name().to_owned(),
            version: profile.profile_version().to_owned(),
        },
        adapter: DescribeAdapter {
            name: adapter,
            config,
            config_yaml,
        },
        node_kinds,
        edge_kinds,
        validations,
        findings,
    })
}
