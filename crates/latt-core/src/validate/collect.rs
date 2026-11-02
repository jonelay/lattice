//! The per-node and per-edge checks: ID syntax, kinds, attrs, edge kinds, the plan's entries.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::document::type_name;
use crate::graph::LatticeGraph;
use crate::profile::{AttrType, EdgeWhereConfig, PlanEntry, Profile, ValidationSpec};
use crate::query::AdjacencyIndex;
use crate::types::{Issue, Origin, Severity};

use super::adapter_issues::apply_overrides;
use super::conditions::{check_edge_count, edge_blocks, eval_conditions, first_failing_condition};
use super::coverage::coverage_context;
use super::{
    FindingCode, default_severity, entry_severity, issue, render_message, resolve_axes,
    severity_for, vacancy_target_message,
};

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return false;
    }

    let number = |start: usize, end: usize| {
        bytes[start..end]
            .iter()
            .fold(0_u32, |value, digit| value * 10 + u32::from(digit - b'0'))
    };
    let year = number(0, 4);
    let month = number(5, 7);
    let day = number(8, 10);
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days_in_month).contains(&day)
}

fn check_type(value: &Value, expected: AttrType) -> bool {
    match expected {
        AttrType::String | AttrType::Enum => value.is_string(),
        AttrType::Date => value.as_str().is_some_and(is_iso_date),
        AttrType::Int => value.is_i64() || value.is_u64(),
        AttrType::Bool => value.is_boolean(),
        AttrType::List => value.is_array(),
    }
}

fn expected_type_name(expected: AttrType) -> &'static str {
    match expected {
        AttrType::Date => "date (YYYY-MM-DD)",
        other => other.as_str(),
    }
}

/// Every finding at its resolved severity (profile overrides and pathway
/// demotion applied) before `--strict`, stale-suppression detection and
/// suppression. Unsorted.
pub(crate) fn collect(graph: &LatticeGraph, profile: &Profile) -> Vec<Issue> {
    let mut issues: Vec<Issue> = Vec::new();

    for (code, &severity) in profile.validation_overrides() {
        if FindingCode::from_str(code).is_some_and(|code| default_severity(code) == Severity::Hint)
            && severity != Severity::Hint
        {
            issues.push(issue(
                FindingCode::ConfigError,
                format!(
                    "severity override for '{code}': cannot promote from hint; \
                     the override is ignored"
                ),
                Origin::Profile,
                profile,
                None,
            ));
        }
    }

    // A declared kind nothing populated: the reason a clean exit over a
    // register that read no tests, or built no edges, carries on its own output.
    for (kind, side) in crate::scope::KindTally::of(graph, profile).empty_kinds() {
        let message = match side {
            crate::scope::EmptyOf::Node => format!("node kind '{kind}' has no node"),
            crate::scope::EmptyOf::Edge => format!("edge kind '{kind}' has no edge"),
        };
        issues.push(issue(
            FindingCode::EmptyKind,
            message,
            Origin::Profile,
            profile,
            None,
        ));
    }

    // Resolve each edge's endpoints to node slots once. The node pass needs only
    // whether a node has an edge on each side, and the edge pass asks three
    // questions about the same two nodes; hashing the IDs again for each was the
    // per-element cost that grew with the register.
    let endpoints: Vec<(Option<usize>, Option<usize>)> = graph
        .iter_edges()
        .map(|edge| (graph.slot(&edge.src), graph.slot(&edge.tgt)))
        .collect();
    let mut has_in = vec![false; graph.node_count()];
    let mut has_out = vec![false; graph.node_count()];
    for &(src, tgt) in &endpoints {
        if let Some(src) = src {
            has_out[src] = true;
        }
        if let Some(tgt) = tgt {
            has_in[tgt] = true;
        }
    }

    // A direction no allowed pair permits can never be satisfied, so a missing
    // edge there says nothing about the register: check only what an edge
    // could supply. An edge outside the pairs is `EDGE_CONSTRAINT`'s finding.
    let (targets, sources): (HashSet<&str>, HashSet<&str>) = profile
        .edge_kinds()
        .values()
        .flat_map(|edge_kind| &edge_kind.allowed)
        .map(|(src, tgt)| (tgt.as_str(), src.as_str()))
        .unzip();

    let mut unknown_kind = vec![false; graph.node_count()];

    for (slot, node) in graph.iter_nodes().enumerate() {
        let Some(node_kind) = profile.node_kinds().get(&node.kind) else {
            issues.push(issue(
                FindingCode::UnknownKind,
                format!("node '{}' has unknown kind '{}'", node.id, node.kind),
                node.provenance.clone(),
                profile,
                Some(node.id.clone()),
            ));
            unknown_kind[slot] = true;
            continue;
        };

        if !node_kind.id_matches(&node.id) {
            issues.push(issue(
                FindingCode::IdFormat,
                format!(
                    "node '{}' does not match pattern '{}' for kind '{}'",
                    node.id, node_kind.id_pattern_source, node.kind
                ),
                node.provenance.clone(),
                profile,
                Some(node.id.clone()),
            ));
        }

        for (attr_name, schema) in &node_kind.attrs {
            let Some(value) = node.attrs.get(attr_name) else {
                if schema.required {
                    issues.push(issue(
                        FindingCode::AttrRequired,
                        format!("node '{}': missing required attr '{attr_name}'", node.id),
                        node.provenance.clone(),
                        profile,
                        Some(node.id.clone()),
                    ));
                }
                continue;
            };

            if !check_type(value, schema.kind) {
                issues.push(issue(
                    FindingCode::AttrType,
                    format!(
                        "node '{}': attr '{attr_name}' expected type '{}', got {} {}",
                        node.id,
                        expected_type_name(schema.kind),
                        type_name(value),
                        as_written(value)
                    ),
                    node.provenance_of(attr_name).clone(),
                    profile,
                    Some(node.id.clone()),
                ));
                continue;
            }

            if schema.kind == AttrType::Enum
                && let Some(allowed) = &schema.values
            {
                let text = value.as_str().unwrap_or_default();
                if !allowed.iter().any(|v| v == text) {
                    issues.push(issue(
                        FindingCode::AttrEnum,
                        format!(
                            "node '{}': attr '{attr_name}' value '{text}' not in {}",
                            node.id,
                            serde_json::to_string(allowed).expect("strings serialize")
                        ),
                        node.provenance_of(attr_name).clone(),
                        profile,
                        Some(node.id.clone()),
                    ));
                }
            }

            if schema.kind == AttrType::List
                && let (Some(items_type), Some(array)) = (schema.items, value.as_array())
            {
                for (index, item) in array.iter().enumerate() {
                    if !check_type(item, items_type) {
                        issues.push(issue(
                            FindingCode::AttrListItems,
                            format!(
                                "node '{}': attr '{attr_name}' element {index} \
                                     expected type '{}', got {} {}",
                                node.id,
                                expected_type_name(items_type),
                                type_name(item),
                                as_written(item)
                            ),
                            node.provenance_of(attr_name).clone(),
                            profile,
                            Some(node.id.clone()),
                        ));
                    }
                }
            }
        }

        if !node_kind.orphan_ok {
            if !has_in[slot] && targets.contains(node.kind.as_str()) {
                issues.push(issue(
                    FindingCode::Unreferenced,
                    format!("node '{}' has no incoming edges", node.id),
                    node.provenance.clone(),
                    profile,
                    Some(node.id.clone()),
                ));
            }
            if !has_out[slot] && sources.contains(node.kind.as_str()) {
                issues.push(issue(
                    FindingCode::Untraced,
                    format!("node '{}' has no outgoing edges", node.id),
                    node.provenance.clone(),
                    profile,
                    Some(node.id.clone()),
                ));
            }
        }
    }

    // Each kind's EDGE_WHERE entries in plan order, so an edge looks up its own
    // kind's entries once instead of scanning the whole plan.
    let mut edge_where: HashMap<&str, Vec<(&PlanEntry, &EdgeWhereConfig)>> = HashMap::new();
    for entry in profile.validation_plan().entries() {
        if let ValidationSpec::EdgeWhere(index) = entry.spec {
            let rule = &profile.edge_where_configs()[index];
            edge_where
                .entry(rule.edge_kind.as_str())
                .or_default()
                .push((entry, rule));
        }
    }

    for (edge, &(src_slot, tgt_slot)) in graph.iter_edges().zip(&endpoints) {
        let edge_kind = profile.edge_kinds().get(&edge.kind);
        if edge_kind.is_none() {
            issues.push(issue(
                FindingCode::UnknownKind,
                format!(
                    "edge '{}'->'{}' has unknown kind '{}'",
                    edge.src, edge.tgt, edge.kind
                ),
                edge.provenance.clone(),
                profile,
                None,
            ));
        }

        if tgt_slot.is_none() {
            let severity = if edge_kind.is_some_and(|kind| kind.cross_source) {
                Severity::Hint
            } else {
                severity_for(FindingCode::Vacancy, profile)
            };
            issues.push(Issue::new(
                severity,
                FindingCode::Vacancy.as_str(),
                vacancy_target_message(&edge.src, &edge.tgt, &edge.kind),
                edge.provenance.clone(),
                Some(edge.src.clone()),
            ));
        }

        if src_slot.is_none() {
            issues.push(issue(
                FindingCode::Vacancy,
                format!(
                    "edge '{}'->'{}' (kind '{}'): source '{}' does not exist",
                    edge.src, edge.tgt, edge.kind, edge.src
                ),
                edge.provenance.clone(),
                profile,
                Some(edge.src.clone()),
            ));
        }

        let Some(edge_kind) = edge_kind else { continue };
        let (Some(src_slot), Some(tgt_slot)) = (src_slot, tgt_slot) else {
            continue;
        };
        if unknown_kind[src_slot] || unknown_kind[tgt_slot] {
            continue;
        }
        let (src_node, tgt_node) = (graph.node_at(src_slot), graph.node_at(tgt_slot));

        let (src_kind, tgt_kind) = (&src_node.kind, &tgt_node.kind);
        if !edge_kind.admits(src_kind, tgt_kind) {
            issues.push(issue(
                FindingCode::EdgeConstraint,
                format!(
                    "edge '{}'->'{}' (kind '{}'): [{src_kind}, {tgt_kind}] not in allowed pairs",
                    edge.src, edge.tgt, edge.kind
                ),
                edge.provenance.clone(),
                profile,
                Some(edge.src.clone()),
            ));
        }

        let ends = [
            ("source", &src_node.attrs),
            ("target", &tgt_node.attrs),
            ("edge", &edge.attrs),
        ];
        let rules = edge_where
            .get(edge.kind.as_str())
            .map_or(&[][..], Vec::as_slice);
        for &(entry, rule) in rules {
            if !ends
                .iter()
                .zip(edge_blocks(&rule.when))
                .all(|((_, attrs), conds)| eval_conditions(conds, attrs, true))
            {
                continue;
            }
            let failure = [(&rule.expect, true), (&rule.reject, false)]
                .into_iter()
                .flat_map(|(block, expect_all)| {
                    ends.iter()
                        .zip(edge_blocks(block))
                        .map(move |((end, attrs), conds)| (end, attrs, conds, expect_all))
                })
                .find_map(|(end, attrs, conds, expect_all)| {
                    first_failing_condition(conds, attrs, expect_all)
                        .map(|(attr, op_desc)| (end, attr, op_desc))
                });
            if let Some((end, attr, op_desc)) = failure {
                let severity = rule
                    .severity
                    .unwrap_or_else(|| severity_for(FindingCode::EdgeWhere, profile));
                let message = render_message(
                    entry.message.as_ref(),
                    format!(
                        "edge '{}'->'{}' (kind '{}'): '{end}.{attr}' {op_desc}",
                        edge.src, edge.tgt, edge.kind
                    ),
                    FindingCode::EdgeWhere,
                    severity,
                    Some(src_node),
                );
                issues.push(Issue::new(
                    severity,
                    FindingCode::EdgeWhere.as_str(),
                    message,
                    edge.provenance.clone(),
                    Some(edge.src.clone()),
                ));
            }
        }
    }

    for entry in profile.validation_plan().entries() {
        let ValidationSpec::EdgeCount(index) = entry.spec else {
            continue;
        };
        let rule = &profile.edge_count_configs()[index];
        check_edge_count(
            rule,
            entry.message.as_ref(),
            graph,
            profile,
            &endpoints,
            &unknown_kind,
            &mut issues,
        );
    }

    let plan = profile.validation_plan();
    for entry in plan.entries() {
        let spec = match &entry.spec {
            ValidationSpec::Coverage(spec) => spec,
            ValidationSpec::Faulty(faults) if entry.code == FindingCode::Coverage.as_str() => {
                report_faulty(faults, profile, &mut issues);
                continue;
            }
            _ => continue,
        };
        let (target, edge_name) = (spec.target_kind.as_str(), spec.edge_kind.as_str());

        let ctx = coverage_context(graph, profile, target, edge_name, &spec.source_where);
        let severity = entry_severity(entry, FindingCode::Coverage, profile);

        for node in graph.iter_nodes() {
            if node.kind == target
                && eval_conditions(&spec.where_, &node.attrs, true)
                && !ctx.covered.contains(node.id.as_str())
            {
                let mut finding = issue(
                    FindingCode::Coverage,
                    format!(
                        "node '{}' (kind '{target}') has no incoming '{edge_name}' edge",
                        node.id
                    ),
                    node.provenance.clone(),
                    profile,
                    Some(node.id.clone()),
                );
                finding.severity = severity;
                finding.message = render_message(
                    entry.message.as_ref(),
                    finding.message,
                    FindingCode::Coverage,
                    severity,
                    Some(node),
                );
                finding.state = Some(ctx.state().to_string());
                issues.push(finding);
            }
        }

        ctx.emit_unknown(edge_name, target, "", profile, &mut issues);
    }

    for entry in plan.entries() {
        let spec = match &entry.spec {
            ValidationSpec::CoverageDeep(spec) => spec,
            ValidationSpec::Faulty(faults) if entry.code == FindingCode::CoverageDeep.as_str() => {
                report_faulty(faults, profile, &mut issues);
                continue;
            }
            _ => continue,
        };
        let (target, via, evidence) = (
            spec.target_kind.as_str(),
            spec.via.as_str(),
            spec.evidence.as_str(),
        );
        let where_conditions = &spec.where_;

        let targets: BTreeSet<&str> = graph
            .iter_nodes()
            .filter(|n| n.kind == target)
            .map(|n| n.id.as_str())
            .collect();

        let ctx = coverage_context(graph, profile, target, evidence, &spec.source_where);

        // Children keyed by parent. `via` edge targets are the parents. Only
        // target-kind endpoints join the rollup: a foreign-kind or dangling
        // source has no evidence semantics here and contributes no child.
        // When via == evidence the original single-pass gave evidence priority;
        // preserve that by skipping evidence edges here.
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for edge in graph.iter_edges() {
            if edge.kind == via
                && edge.kind != evidence
                && targets.contains(edge.src.as_str())
                && targets.contains(edge.tgt.as_str())
            {
                children
                    .entry(edge.tgt.as_str())
                    .or_default()
                    .push(edge.src.as_str());
            }
        }

        // Least fixed point: seed with directly evidenced targets, then notify
        // each parent when one of its children becomes covered. Each node enters
        // the queue once and each `via` edge is visited once. The recursive
        // reading alone has two solutions on a cycle; seeding picks the least
        // one, so an evidence-free cycle stays uncovered while an anchored one
        // propagates coverage out. Childless targets never enter.
        let mut covered: HashSet<&str> = targets
            .iter()
            .copied()
            .filter(|t| ctx.covered.contains(*t))
            .collect();
        let mut parents_of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut remaining: HashMap<&str, usize> = HashMap::new();
        for (&parent, kids) in &children {
            remaining.insert(parent, kids.len());
            for &kid in kids {
                parents_of.entry(kid).or_default().push(parent);
            }
        }
        let mut queue: VecDeque<&str> = covered.iter().copied().collect();
        while let Some(child) = queue.pop_front() {
            for &parent in parents_of.get(child).into_iter().flatten() {
                let count = remaining.get_mut(parent).expect("parent has children");
                *count -= 1;
                if *count == 0 && covered.insert(parent) {
                    queue.push_back(parent);
                }
            }
        }

        let uncovered_set: HashSet<&str> = targets
            .iter()
            .copied()
            .filter(|t| !covered.contains(*t))
            .collect();
        let via_kinds: &[&str] = if via != evidence { &[via] } else { &[] };
        let via_index = AdjacencyIndex::build_filtered(graph, via_kinds);
        let condensation = crate::query::components(&via_index, &uncovered_set);
        let mut component_of: HashMap<&str, usize> = HashMap::new();
        let mut cyclic_components: Vec<&[String]> = Vec::new();
        for comp in &condensation.components {
            if comp.cyclic {
                let idx = cyclic_components.len();
                for member in &comp.members {
                    component_of.insert(member.as_str(), idx);
                }
                cyclic_components.push(&comp.members);
            }
        }

        for node in graph.iter_nodes() {
            if node.kind != target
                || !eval_conditions(where_conditions, &node.attrs, true)
                || covered.contains(node.id.as_str())
            {
                continue;
            }
            let id = node.id.as_str();
            // The immediate cause, so a cascade of ancestor findings stays
            // navigable leaf-ward instead of flooding indistinguishable rows.
            let detail = if let Some(component) = component_of.get(id) {
                let members = &cyclic_components[*component];
                format!("in an uncovered '{via}' cycle: {}", members.join(", "))
            } else if let Some(kids) = children.get(id) {
                let mut bad: Vec<&str> = kids
                    .iter()
                    .copied()
                    .filter(|k| !covered.contains(*k))
                    .collect();
                bad.sort_unstable();
                bad.dedup();
                let list = bad
                    .iter()
                    .map(|k| format!("'{k}'"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("uncovered children: {list}")
            } else {
                "no evidence and no children".to_string()
            };
            let mut finding = issue(
                FindingCode::CoverageDeep,
                format!("node '{id}' (kind '{target}') is not deep-covered: {detail}"),
                node.provenance.clone(),
                profile,
                Some(node.id.clone()),
            );
            finding.severity = entry_severity(entry, FindingCode::CoverageDeep, profile);
            finding.message = render_message(
                entry.message.as_ref(),
                finding.message,
                FindingCode::CoverageDeep,
                finding.severity,
                Some(node),
            );
            finding.state = Some(ctx.state().to_string());
            issues.push(finding);
        }

        ctx.emit_unknown(evidence, target, "deep ", profile, &mut issues);
    }

    for entry in profile.validation_plan().entries() {
        let ValidationSpec::Constraint(index) = entry.spec else {
            continue;
        };
        let constraint = &profile.constraint_configs()[index];
        let severity = constraint
            .severity
            .unwrap_or_else(|| severity_for(FindingCode::Constraint, profile));
        for node in graph.iter_nodes() {
            if node.kind != constraint.kind {
                continue;
            }
            if !eval_conditions(&constraint.when, &node.attrs, true) {
                continue;
            }
            let expect_fail = first_failing_condition(&constraint.expect, &node.attrs, true);
            let failure = expect_fail
                .or_else(|| first_failing_condition(&constraint.reject, &node.attrs, false));
            if let Some((attr, op_desc)) = failure {
                let message = render_message(
                    entry.message.as_ref(),
                    format!("constraint failed: '{attr}' {op_desc}"),
                    FindingCode::Constraint,
                    severity,
                    Some(node),
                );
                issues.push(Issue::new(
                    severity,
                    FindingCode::Constraint.as_str(),
                    message,
                    node.provenance.clone(),
                    Some(node.id.clone()),
                ));
            }
        }
    }

    issues.extend(apply_overrides(graph.adapter_issues(), profile));

    resolve_axes(&mut issues, graph, profile);
    issues
}

/// The reporting order: location, then code, then identity. Stable, so equal keys
/// keep collection order.
pub(crate) fn sort_issues(issues: &mut [Issue]) {
    issues.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
}

/// Report a coverage entry the loader compiled with faults, one `CONFIG_ERROR`
/// per fault.
///
/// Only the profile `fuse` composes reaches here: a standard profile's coverage
/// entries were refused when it loaded (profile-schema spec, coverage entries
/// name declared kinds), while the composed profile reports the same faults as
/// findings.
fn report_faulty(faults: &[String], profile: &Profile, issues: &mut Vec<Issue>) {
    issues.extend(faults.iter().map(|message| {
        issue(
            FindingCode::ConfigError,
            message.clone(),
            Origin::Profile,
            profile,
            None,
        )
    }));
}

/// Render a value as written, in compact JSON, cut to a readable length.
///
/// A type finding names what the register holds, not only its type: `got int 3`
/// says which value to fix where `got int` leaves the user to find it.
fn as_written(value: &Value) -> String {
    const LIMIT: usize = 60;
    let text = serde_json::to_string(value).expect("a JSON value serializes");
    if text.chars().count() <= LIMIT {
        return text;
    }
    let cut: String = text.chars().take(LIMIT - 1).collect();
    format!("{cut}…")
}
