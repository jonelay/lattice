use crate::graph::LatticeGraph;
use crate::profile::{Profile, ValidationSpec};
use crate::types::{CoverageGap, RuleCoverageReport};
use crate::validate::{coverage::coverage_context, eval_conditions, resolve_adapter_issues};

use super::QueryError;

pub(crate) fn rule_coverage(
    graph: &LatticeGraph,
    profile: &Profile,
    rule: usize,
) -> Result<RuleCoverageReport, QueryError> {
    let index = rule
        .checked_sub(1)
        .ok_or_else(|| QueryError::new("coverage --rule starts at 1"))?;
    let entry = profile
        .validation_plan()
        .entries()
        .iter()
        .filter(|entry| entry.code == "COVERAGE")
        .nth(index)
        .ok_or_else(|| {
            QueryError::new(format!(
                "COVERAGE rule {rule} does not exist in the resolved profile"
            ))
        })?;
    let ValidationSpec::Coverage(spec) = &entry.spec else {
        return Err(QueryError::new(format!(
            "COVERAGE rule {rule} requires target_kind and edge_kind"
        )));
    };
    let ctx = coverage_context(
        graph,
        profile,
        &spec.target_kind,
        &spec.edge_kind,
        &spec.source_where,
    );
    let mut total = 0;
    let mut missing = Vec::new();
    for node in graph.iter_nodes().filter(|node| {
        node.kind == spec.target_kind && eval_conditions(&spec.where_, &node.attrs, true)
    }) {
        total += 1;
        if !ctx.covered.contains(node.id.as_str()) {
            missing.push(CoverageGap {
                id: node.id.clone(),
                provenance: node.provenance.clone(),
                state: ctx.state(),
            });
        }
    }
    missing.sort_by(|a, b| a.id.cmp(&b.id));
    let covered = total - missing.len();
    let (where_, source_where) = profile
        .coverage_filters(index)
        .map_err(|error| QueryError::new(error.to_string()))?;
    let mut findings = resolve_adapter_issues(graph, profile);
    findings.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    Ok(RuleCoverageReport {
        rule,
        target_kind: spec.target_kind.clone(),
        edge_kind: spec.edge_kind.clone(),
        where_,
        source_where,
        total,
        covered,
        covered_pct: (total > 0).then(|| (covered as f64 * 1000.0 / total as f64).round() / 10.0),
        missing,
        source_kinds: ctx
            .source_kinds
            .iter()
            .map(|kind| (*kind).to_owned())
            .collect(),
        source_total: ctx.population,
        unattributed: ctx.unattributed,
        input_partial: findings.iter().any(|issue| issue.code == "PARSE_ERROR"),
        findings,
    })
}
