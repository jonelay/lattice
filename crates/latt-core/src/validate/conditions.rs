//! Evaluating conditions, edge counts and `where` blocks against the graph.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde_json::Value;

use crate::graph::{Edge, LatticeGraph, Node};
use crate::profile::{Comparable, ConditionOp, CountEnd, EdgeConditions, EdgeCountConfig, Profile};
use crate::template::Template;
use crate::types::{Issue, Provenance};

use super::{FindingCode, render_message, severity_for};

fn compare_value_to_comparable(left: &Value, right: &Comparable) -> Option<Ordering> {
    match (left, right) {
        (Value::String(l), Comparable::Str(r)) => Some(l.as_str().cmp(r.as_str())),
        (Value::Number(n), Comparable::Int(r)) => {
            let l = n
                .as_i64()
                .map(i128::from)
                .or_else(|| n.as_u64().map(i128::from))?;
            Some(l.cmp(r))
        }
        _ => None,
    }
}

pub(crate) fn eval_condition(
    cond: &crate::profile::Condition,
    attrs: &serde_json::Map<String, Value>,
) -> bool {
    match &cond.op {
        ConditionOp::Present(expected) => attrs.contains_key(&cond.attr) == *expected,
        ConditionOp::Eq(expected) => attrs.get(&cond.attr) == Some(expected),
        ConditionOp::Ne(expected) => attrs.get(&cond.attr) != Some(expected),
        ConditionOp::In(values) => attrs.get(&cond.attr).is_some_and(|v| values.contains(v)),
        ConditionOp::Lt(expected) => attrs
            .get(&cond.attr)
            .and_then(|value| compare_value_to_comparable(value, expected))
            .is_some_and(Ordering::is_lt),
        ConditionOp::Gt(expected) => attrs
            .get(&cond.attr)
            .and_then(|value| compare_value_to_comparable(value, expected))
            .is_some_and(Ordering::is_gt),
        ConditionOp::Lte(expected) => attrs
            .get(&cond.attr)
            .and_then(|value| compare_value_to_comparable(value, expected))
            .is_some_and(Ordering::is_le),
        ConditionOp::Gte(expected) => attrs
            .get(&cond.attr)
            .and_then(|value| compare_value_to_comparable(value, expected))
            .is_some_and(Ordering::is_ge),
        ConditionOp::Matches(regex) => attrs.get(&cond.attr).is_some_and(|value| match value {
            Value::String(s) => regex.is_match(s),
            Value::Number(_) | Value::Bool(_) => regex.is_match(&value.to_string()),
            Value::Null | Value::Array(_) | Value::Object(_) => false,
        }),
    }
}

/// True when `attrs` satisfies the condition set the way a profile block asks.
///
/// With `expect_all` every condition must hold (an `expect:` block). Without it
/// *no* condition may hold (a `reject:` block) — not "any holds", which the name
/// of the flag might suggest. An empty slice is true either way.
///
/// # Examples
///
/// ```
/// use latt_core::query::parse_filter;
/// use latt_core::validate::eval_conditions;
///
/// let done = [parse_filter("status=done")?];
/// let attrs = serde_json::from_value(serde_json::json!({"status": "done"}))?;
/// assert!(eval_conditions(&done, &attrs, true));
/// assert!(!eval_conditions(&done, &attrs, false), "false means none may match");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
#[doc(hidden)]
pub fn eval_conditions(
    conditions: &[crate::profile::Condition],
    attrs: &serde_json::Map<String, Value>,
    expect_all: bool,
) -> bool {
    if expect_all {
        conditions.iter().all(|c| eval_condition(c, attrs))
    } else {
        conditions.iter().all(|c| !eval_condition(c, attrs))
    }
}

fn condition_op_desc(cond: &crate::profile::Condition) -> String {
    match &cond.op {
        ConditionOp::Eq(v) => format!("expected eq {v}"),
        ConditionOp::Ne(v) => format!("expected ne {v}"),
        ConditionOp::In(_) => "expected in [...]".to_string(),
        ConditionOp::Lt(c) => format!("expected lt {c}"),
        ConditionOp::Gt(c) => format!("expected gt {c}"),
        ConditionOp::Lte(c) => format!("expected lte {c}"),
        ConditionOp::Gte(c) => format!("expected gte {c}"),
        ConditionOp::Matches(_) => "expected matches pattern".to_string(),
        ConditionOp::Present(b) => format!("expected present={b}"),
    }
}

/// Return the first condition whose result is not `expect_all` (under `expect` one
/// that fails, under `reject` one that passes), as its attr and the expectation missed.
pub(super) fn first_failing_condition<'c>(
    conditions: &'c [crate::profile::Condition],
    attrs: &serde_json::Map<String, Value>,
    expect_all: bool,
) -> Option<(&'c str, String)> {
    conditions
        .iter()
        .find(|cond| eval_condition(cond, attrs) != expect_all)
        .map(|cond| (cond.attr.as_str(), condition_op_desc(cond)))
}

/// Count one `EDGE_COUNT` entry's edges per node and per group.
///
/// Only edges whose both ends are declared nodes of declared kinds count: the
/// rest already carry `VACANCY` or `UNKNOWN_KIND`, and a far end that does not
/// exist has no attr to group by.
pub(super) fn check_edge_count(
    rule: &EdgeCountConfig,
    template: Option<&Template>,
    graph: &LatticeGraph,
    profile: &Profile,
    endpoints: &[(Option<usize>, Option<usize>)],
    unknown_kind: &[bool],
    issues: &mut Vec<Issue>,
) {
    let severity = rule
        .severity
        .unwrap_or_else(|| severity_for(FindingCode::EdgeCount, profile));
    let (way, far_end) = match rule.direction {
        CountEnd::Out => ("outgoing", "target"),
        CountEnd::In => ("incoming", "source"),
    };
    // Bucketed by the near end's slot, which `collect` already resolved per edge.
    let mut counted: Vec<Vec<(&Edge, &Node)>> = vec![Vec::new(); graph.node_count()];
    for (edge, &ends) in graph.iter_edges().zip(endpoints) {
        let (Some(src), Some(tgt)) = ends else {
            continue;
        };
        if edge.kind != rule.edge_kind || unknown_kind[src] || unknown_kind[tgt] {
            continue;
        }
        let (near, far) = match rule.direction {
            CountEnd::Out => (src, tgt),
            CountEnd::In => (tgt, src),
        };
        counted[near].push((edge, graph.node_at(far)));
    }
    let bounds = match (rule.min, rule.max) {
        (Some(lo), Some(hi)) if lo == hi => format!("exactly {lo}"),
        (Some(lo), Some(hi)) => format!("{lo} to {hi}"),
        (Some(lo), None) => format!("at least {lo}"),
        (None, Some(hi)) => format!("at most {hi}"),
        (None, None) => "any count".to_string(),
    };
    let mut push = |message: String, provenance: Provenance, node: &Node| {
        issues.push(Issue::new(
            severity,
            FindingCode::EdgeCount.as_str(),
            render_message(
                template,
                message,
                FindingCode::EdgeCount,
                severity,
                Some(node),
            ),
            provenance,
            Some(node.id.clone()),
        ));
    };
    for (node, edges) in graph.iter_nodes().zip(&counted) {
        if node.kind != rule.node_kind || !eval_conditions(&rule.where_, &node.attrs, true) {
            continue;
        }
        // Ungrouped, the node's edges are counted as one borrowed group; only
        // a `group_by` rule builds owned groups.
        let mut groups: BTreeMap<String, Vec<(&Edge, &Node)>> = BTreeMap::new();
        let mut ungrouped = None;
        match &rule.group_by {
            Some(attr) if !edges.is_empty() => {
                for &(edge, far) in edges {
                    if let Some(value) = far.attrs.get(attr).filter(|v| !v.is_null()) {
                        groups
                            .entry(value.to_string())
                            .or_default()
                            .push((edge, far));
                    } else {
                        push(
                            format!(
                                "edge '{}'->'{}' (kind '{}'): {far_end} '{}' has no '{attr}' \
                                 attr to group by",
                                edge.src, edge.tgt, edge.kind, far.id
                            ),
                            edge.provenance.clone(),
                            node,
                        );
                    }
                }
            }
            _ => ungrouped = Some(("", edges.as_slice())),
        }
        let all = groups
            .iter()
            .map(|(value, members)| (value.as_str(), members.as_slice()))
            .chain(ungrouped);
        for (value, members) in all {
            let count = members.len() as u64;
            if rule.min.is_some_and(|lo| count < lo) || rule.max.is_some_and(|hi| count > hi) {
                let group = match &rule.group_by {
                    Some(attr) if !value.is_empty() => {
                        format!(" to a {far_end} with '{attr}' = {value}")
                    }
                    _ => String::new(),
                };
                let listed = members
                    .iter()
                    .map(|(edge, far)| format!("'{}' ({})", far.id, edge.provenance))
                    .collect::<Vec<_>>()
                    .join(", ");
                let tail = if listed.is_empty() {
                    String::new()
                } else {
                    format!(": {listed}")
                };
                push(
                    format!(
                        "node '{}' has {count} {way} '{}' edge(s){group}, expected {bounds}{tail}",
                        node.id, rule.edge_kind
                    ),
                    node.provenance.clone(),
                    node,
                );
            }
        }
    }
}

/// The blocks in the order `collect` pairs them with the edge's ends.
pub(super) fn edge_blocks(conditions: &EdgeConditions) -> [&[crate::profile::Condition]; 3] {
    [&conditions.source, &conditions.target, &conditions.edge]
}
