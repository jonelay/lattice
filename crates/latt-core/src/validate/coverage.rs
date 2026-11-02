//! The coverage context shared by the coverage checks and the coverage report.

use std::collections::{BTreeSet, HashSet};

use crate::graph::LatticeGraph;
use crate::profile::Profile;
use crate::types::{Issue, Origin};

use super::conditions::eval_conditions;
use super::{FindingCode, issue};

pub(crate) struct CoverageContext<'a> {
    pub covered: HashSet<&'a str>,
    pub source_kinds: BTreeSet<&'a str>,
    pub population: usize,
    pub unattributed: usize,
    /// True when `source_where` narrowed the population, which the hint says so
    /// that "7 of 65" is not read against the kind's full count.
    pub filtered: bool,
}

/// Unlike `connected` ([`AdjacencyIndex`](crate::query::AdjacencyIndex)), coverage counts only edges whose endpoints
/// the register actually declared: an edge from a source that does not exist
/// is not evidence that the target is verified. Attribution asks less than
/// coverage does: an outgoing edge of this kind shows the author attributed
/// the node, even when its target dangles.
pub(crate) fn coverage_context<'a>(
    graph: &'a LatticeGraph,
    profile: &'a Profile,
    target: &str,
    edge_kind_name: &str,
    source_where: &[crate::profile::Condition],
) -> CoverageContext<'a> {
    // A source `source_where` excludes neither covers a target nor counts
    // towards the unattributed population: a retired requirement is evidence
    // of nothing, and is meant to carry no edge.
    let admitted = |id: &str| {
        graph
            .node(id)
            .is_some_and(|node| eval_conditions(source_where, &node.attrs, true))
    };
    let mut covered: HashSet<&str> = HashSet::new();
    let mut attributed: HashSet<&str> = HashSet::new();
    for edge in graph.iter_edges() {
        if edge.kind != edge_kind_name {
            continue;
        }
        attributed.insert(edge.src.as_str());
        if graph.has_node(&edge.tgt) && admitted(&edge.src) {
            covered.insert(edge.tgt.as_str());
        }
    }
    let edge_kind = &profile.edge_kinds()[edge_kind_name];
    let source_kinds: BTreeSet<&str> = edge_kind
        .allowed
        .iter()
        .filter(|(_, t)| *t == target)
        .map(|(s, _)| s.as_str())
        .collect();
    let (mut population, mut unattributed) = (0, 0);
    for node in graph.iter_nodes().filter(|n| {
        source_kinds.contains(n.kind.as_str()) && eval_conditions(source_where, &n.attrs, true)
    }) {
        population += 1;
        if !attributed.contains(node.id.as_str()) {
            unattributed += 1;
        }
    }
    CoverageContext {
        covered,
        source_kinds,
        population,
        unattributed,
        filtered: !source_where.is_empty(),
    }
}

impl CoverageContext<'_> {
    /// While any admitted source carries no edge of the kind, "no incoming
    /// edge" cannot distinguish a gap from a missing attribution, so the
    /// per-node state is unknown rather than unverified.
    pub(crate) fn state(&self) -> &'static str {
        if self.unattributed == 0 {
            "unverified"
        } else {
            "unknown"
        }
    }

    /// The base population rides along: "287 carry no edge" over an
    /// unstated 1939 reads as an unwired layer, not a 14.8% gap.
    pub(crate) fn emit_unknown(
        &self,
        edge_kind_name: &str,
        target: &str,
        qualifier: &str,
        profile: &Profile,
        issues: &mut Vec<Issue>,
    ) {
        if self.unattributed == 0 {
            return;
        }
        let kinds = self
            .source_kinds
            .iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let noun = if self.source_kinds.len() == 1 {
            "kind"
        } else {
            "kinds"
        };
        let population = self.population;
        let unattributed = self.unattributed;
        let matching = if self.filtered {
            " matching source_where"
        } else {
            ""
        };
        let pct = unattributed as f64 * 100.0 / population as f64;
        issues.push(issue(
            FindingCode::CoverageUnknown,
            format!(
                "{unattributed} of {population} node(s) of {noun} {kinds}{matching} ({pct:.1}%) \
                 carry no outgoing '{edge_kind_name}' edge; {qualifier}coverage state for \
                 '{target}' is unknown"
            ),
            Origin::Profile,
            profile,
            None,
        ));
    }
}
