//! The differences between lattice and the translations that are meant.
//!
//! Each rule names a behaviour lattice keeps on purpose, and the operator
//! matrix test asserts the rule still fires on generated graphs, so an entry
//! here cannot outlive the behaviour it excuses. A rule is consulted only for a
//! node the two sides disagree on; it never widens agreement.

use latt_core::graph::LatticeGraph;
use latt_core::profile::{Condition, ConditionOp, Profile, ValidationSpec};
use latt_core::types::Issue;
use trustfall_core::ir::FieldValue;

use crate::adapter::typed;

/// One deliberate difference.
#[derive(Clone, Copy, Debug)]
pub struct Deliberate {
    pub rule: &'static str,
    pub reason: &'static str,
}

/// Every listed difference.
pub const RULES: &[Deliberate] = &[Deliberate {
    rule: "present-on-untyped-value",
    reason: "lattice's `present` reads whether the attr exists, so an attr holding null or a \
             value of another type (reported as `ATTR_TYPE`) is present; the typed property \
             is null for both, which is all Trustfall's `is_null`/`is_not_null` can see",
}];

/// The conditions a code's entries evaluate against a node of `kind`, for the
/// shapes that read node attrs.
fn conditions_for<'p>(profile: &'p Profile, code: &str, kind: &str) -> Vec<&'p Condition> {
    let mut out = Vec::new();
    for entry in profile.validation_plan().entries() {
        if entry.code != code {
            continue;
        }
        match &entry.spec {
            ValidationSpec::Constraint(index) => {
                let rule = &profile.constraint_configs()[*index];
                if rule.kind == kind {
                    out.extend(rule.when.iter().chain(&rule.expect).chain(&rule.reject));
                }
            }
            ValidationSpec::Coverage(spec) if spec.target_kind == kind => {
                out.extend(&spec.where_);
            }
            ValidationSpec::CoverageDeep(spec) if spec.target_kind == kind => {
                out.extend(&spec.where_);
            }
            ValidationSpec::EdgeCount(index) => {
                let rule = &profile.edge_count_configs()[*index];
                if rule.node_kind == kind {
                    out.extend(&rule.where_);
                }
            }
            ValidationSpec::EdgeWhere(index) => {
                let rule = &profile.edge_where_configs()[*index];
                for block in [&rule.when, &rule.expect, &rule.reject] {
                    out.extend(block.source.iter().chain(&block.target));
                }
            }
            _ => {}
        }
    }
    out
}

/// The rule explaining why `node_id` sits on one side only, if one does.
/// `only_lattice` says which side reported it.
#[must_use]
pub fn explained_by(
    profile: &Profile,
    graph: &LatticeGraph,
    _issues: &[Issue],
    code: &str,
    node_id: &str,
    _only_lattice: bool,
) -> Option<Deliberate> {
    // The node (or, for EDGE_WHERE, an endpoint of one of its edges) holds an
    // attr the typed property renders as null — explicit null or a value of
    // another type — that a `present` condition of this code reads.
    let untyped_attrs = |id: &str| -> Vec<(String, String)> {
        let Some(node) = graph.node(id) else {
            return Vec::new();
        };
        let Some(declared) = profile.node_kinds().get(&node.kind) else {
            return Vec::new();
        };
        node.attrs
            .iter()
            .filter(|(attr, value)| {
                declared.attrs.get(*attr).is_some_and(|schema| {
                    typed(
                        value,
                        schema.kind.as_str(),
                        schema.items.map(|i| i.as_str()),
                    ) == FieldValue::Null
                })
            })
            .map(|(attr, _)| (node.kind.clone(), attr.clone()))
            .collect()
    };
    let mut candidates = untyped_attrs(node_id);
    if code == "EDGE_WHERE" {
        for edge in graph.iter_edges().filter(|e| e.src == node_id) {
            candidates.extend(untyped_attrs(&edge.tgt));
        }
    }
    for (kind, attr) in candidates {
        let reads_present = conditions_for(profile, code, &kind)
            .into_iter()
            .any(|c| c.attr() == attr && matches!(c.op(), ConditionOp::Present(_)));
        if reads_present {
            return Some(RULES[0]);
        }
    }
    None
}
