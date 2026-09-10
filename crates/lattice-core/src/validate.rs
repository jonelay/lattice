//! Checking a graph against its profile: ID syntax, kinds, attrs, edges, coverage.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::document::type_name;
use crate::graph::LatticeGraph;
use crate::profile::{ConditionOp, Profile, ValidationConfig};
use crate::types::{Issue, Provenance, Severity};

#[derive(Clone, Copy)]
pub(crate) enum FindingCode {
    AttrEnum,
    AttrListItems,
    AttrRequired,
    AttrType,
    Constraint,
    PathwayUnresolved,
    ConfigError,
    Coverage,
    CoverageDeep,
    CoverageUnknown,
    Vacancy,
    EdgeConstraint,
    IdFormat,
    OrphanNode,
    SuggestedEdge,
    SuggestionUnresolved,
    UnknownKind,
}

impl FindingCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AttrEnum => "ATTR_ENUM",
            Self::AttrListItems => "ATTR_LIST_ITEMS",
            Self::AttrRequired => "ATTR_REQUIRED",
            Self::AttrType => "ATTR_TYPE",
            Self::Constraint => "CONSTRAINT",
            Self::PathwayUnresolved => "PATHWAY_UNRESOLVED",
            Self::ConfigError => "CONFIG_ERROR",
            Self::Coverage => "COVERAGE",
            Self::CoverageDeep => "COVERAGE_DEEP",
            Self::CoverageUnknown => "COVERAGE_UNKNOWN",
            Self::Vacancy => "VACANCY",
            Self::EdgeConstraint => "EDGE_CONSTRAINT",
            Self::IdFormat => "ID_FORMAT",
            Self::OrphanNode => "ORPHAN_NODE",
            Self::SuggestedEdge => "SUGGESTED_EDGE",
            Self::SuggestionUnresolved => "SUGGESTION_UNRESOLVED",
            Self::UnknownKind => "UNKNOWN_KIND",
        }
    }

    fn from_str(code: &str) -> Option<Self> {
        match code {
            "ATTR_ENUM" => Some(Self::AttrEnum),
            "ATTR_LIST_ITEMS" => Some(Self::AttrListItems),
            "ATTR_REQUIRED" => Some(Self::AttrRequired),
            "ATTR_TYPE" => Some(Self::AttrType),
            "CONSTRAINT" => Some(Self::Constraint),
            "PATHWAY_UNRESOLVED" => Some(Self::PathwayUnresolved),
            "CONFIG_ERROR" => Some(Self::ConfigError),
            "COVERAGE" => Some(Self::Coverage),
            "COVERAGE_DEEP" => Some(Self::CoverageDeep),
            "COVERAGE_UNKNOWN" => Some(Self::CoverageUnknown),
            "VACANCY" => Some(Self::Vacancy),
            "EDGE_CONSTRAINT" => Some(Self::EdgeConstraint),
            "ID_FORMAT" => Some(Self::IdFormat),
            "ORPHAN_NODE" => Some(Self::OrphanNode),
            "SUGGESTED_EDGE" => Some(Self::SuggestedEdge),
            "SUGGESTION_UNRESOLVED" => Some(Self::SuggestionUnresolved),
            "UNKNOWN_KIND" => Some(Self::UnknownKind),
            _ => None,
        }
    }
}

/// The shipped severity of each core finding code, before profile overrides.
pub(crate) fn default_severity(code: FindingCode) -> Severity {
    match code {
        FindingCode::IdFormat
        | FindingCode::UnknownKind
        | FindingCode::EdgeConstraint
        | FindingCode::Vacancy
        | FindingCode::AttrRequired
        | FindingCode::AttrType
        | FindingCode::AttrEnum
        | FindingCode::AttrListItems
        | FindingCode::ConfigError => Severity::Error,
        // The overlay codes. Registered here so a profile trying to promote one
        // is reported as a CONFIG_ERROR by the pre-pass in `validate`, which is
        // the only place that sees the override whether or not a document ran.
        FindingCode::CoverageUnknown
        | FindingCode::SuggestedEdge
        | FindingCode::SuggestionUnresolved => Severity::Hint,
        // Explicitly warning, not via the fallthrough: hint is never promotable,
        // so a hint default here would foreclose gating on deep coverage for
        // every profile permanently (coverage-query spec).
        FindingCode::Constraint
        | FindingCode::PathwayUnresolved
        | FindingCode::Coverage
        | FindingCode::CoverageDeep
        | FindingCode::OrphanNode => Severity::Warning,
    }
}

fn severity_for(code: FindingCode, profile: &Profile) -> Severity {
    let default = default_severity(code);
    match profile.validation_overrides().get(code.as_str()).copied() {
        // Nothing promotes from hint: an override trying is reported as a
        // CONFIG_ERROR where validate collects them, and changes nothing here.
        Some(o) if default == Severity::Hint && o != Severity::Hint => default,
        Some(o) => o,
        None => default,
    }
}

fn issue(
    code: FindingCode,
    message: String,
    provenance: Provenance,
    profile: &Profile,
    node_id: Option<String>,
) -> Issue {
    Issue::new(
        severity_for(code, profile),
        code.as_str(),
        message,
        provenance,
        node_id,
    )
}

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

fn check_type(value: &Value, expected: &str) -> bool {
    match expected {
        "string" | "enum" => value.is_string(),
        "date" => value.as_str().is_some_and(is_iso_date),
        "int" => value.is_i64() || value.is_u64(),
        "bool" => value.is_boolean(),
        "list" => value.is_array(),
        _ => true,
    }
}

fn expected_type_name(expected: &str) -> &str {
    if expected == "date" {
        "date (YYYY-MM-DD)"
    } else {
        expected
    }
}

/// Apply the profile's severity overrides to issues emitted outside core.
///
/// Guards on the severity an issue *arrives* at rather than on its code's
/// shipped default: core has no default for a code it does not implement, so
/// an externally emitted hint would otherwise be promoted with nothing said.
fn apply_overrides(issues: &[Issue], profile: &Profile) -> Vec<Issue> {
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
            Provenance::new("<profile>", 0),
            profile,
            None,
        )
    }));
    out
}

/// Demote findings whose node is not yet due on its bound ordering pathway.
///
/// Runs after collection and before `--strict`, so a demoted finding is already
/// `info` when promotion looks at it and survives. Starts from each issue's
/// effective severity rather than recomputing it, or an adapter code with no
/// shipped default would silently drop to the warning fallback.
fn resolve_axes(issues: Vec<Issue>, graph: &LatticeGraph, profile: &Profile) -> Vec<Issue> {
    if profile.pathway_bindings().is_empty() {
        return issues;
    }

    // One finding per binding, not per issue: the mismatch is a property of the
    // pairing, and repeating it per finding would bury the findings it reports
    // about. Keyed on the code too, so two codes bound to one missing pathway each
    // say so — a single finding could name only one of them.
    let mut unresolved: BTreeMap<(String, String), Issue> = BTreeMap::new();
    let mut resolved: Vec<Issue> = Vec::with_capacity(issues.len());

    for issue in issues {
        let Some(binding) = profile.pathway_bindings().get(&issue.code) else {
            resolved.push(issue);
            continue;
        };

        let Some(pathway) = graph.pathway(&binding.pathway) else {
            unresolved
                .entry((issue.code.clone(), binding.pathway.clone()))
                .or_insert_with(|| {
                    Issue::new(
                        severity_for(FindingCode::PathwayUnresolved, profile),
                        FindingCode::PathwayUnresolved.as_str(),
                        format!(
                            "profile binds '{}' to pathway '{}', which the target does not declare",
                            issue.code, binding.pathway
                        ),
                        Provenance::new("<profile>", 0),
                        None,
                    )
                });
            resolved.push(issue);
            continue;
        };

        let Some(node_id) = issue.node_id.as_deref() else {
            resolved.push(issue);
            continue;
        };
        let Some(node) = graph.node(node_id) else {
            resolved.push(issue);
            continue;
        };
        let Some(Value::String(position)) = node.attrs.get(&binding.position_attr) else {
            resolved.push(issue);
            continue;
        };

        if pathway.is_member(position) && !pathway.is_after(position) {
            resolved.push(issue);
            continue;
        }

        let mut demoted = issue;
        demoted.severity = Severity::Info;
        resolved.push(demoted);
    }

    resolved.extend(unresolved.into_values());
    resolved
}

/// Adapter issues at the severities the profile and its pathways decide.
#[must_use]
pub fn resolve_adapter_issues(graph: &LatticeGraph, profile: &Profile) -> Vec<Issue> {
    resolve_axes(
        apply_overrides(graph.adapter_issues(), profile),
        graph,
        profile,
    )
}

/// A config key's value: `Ok(Some)` for a string, `Ok(None)` when the key is
/// absent, `Err(type name)` for a declared value of any other type.
///
/// Wrong type and absence are distinct on purpose — reading a mistyped value
/// as "missing" (or stringifying it into a kind check) would report the wrong
/// fault, and a declared config must never fail in silence.
pub(crate) fn config_str(
    config: &ValidationConfig,
    key: &str,
) -> Result<Option<String>, &'static str> {
    match config.get(key) {
        None => Ok(None),
        Some(serde_norway::Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(crate::profile::name(other)),
    }
}

/// Which declared-kind table a config key must name into.
enum KindSpace {
    Node,
    Edge,
}

/// Read a validation entry's kind-naming keys, reporting every fault.
///
/// Each key is typed and checked against the declared kinds independently of
/// its siblings — one missing or mistyped key never masks another's fault
/// (validation spec, config typing). Returns one slot per requested key,
/// `None` where the key was missing, mistyped, or named an undeclared kind.
fn read_kind_keys(
    code: &str,
    config: &ValidationConfig,
    keys: &[(&'static str, KindSpace)],
    profile: &Profile,
    issues: &mut Vec<Issue>,
) -> Vec<Option<String>> {
    let config_error = |message: String, issues: &mut Vec<Issue>| {
        issues.push(issue(
            FindingCode::ConfigError,
            message,
            Provenance::new("<profile>", 0),
            profile,
            None,
        ));
    };
    let mut missing = Vec::new();
    let mut out = Vec::new();
    for (key, space) in keys {
        let value = match config_str(config, key) {
            Ok(Some(value)) => Some(value),
            Ok(None) => {
                missing.push(*key);
                None
            }
            Err(got) => {
                config_error(
                    format!("{code} config: '{key}' must be a string, got {got}"),
                    issues,
                );
                None
            }
        };
        out.push(value.filter(|value| {
            let (declared, noun) = match space {
                KindSpace::Node => (profile.node_kinds().contains_key(value), "node"),
                KindSpace::Edge => (profile.edge_kinds().contains_key(value), "edge"),
            };
            if !declared {
                config_error(
                    format!("{code} config: {key} '{value}' not in profile {noun} kinds"),
                    issues,
                );
            }
            declared
        }));
    }
    if !missing.is_empty() {
        config_error(
            format!(
                "{code} config missing required keys: {}",
                missing.join(", ")
            ),
            issues,
        );
    }
    out
}

fn compare_ints(left: &serde_json::Number, right: &serde_json::Number) -> Option<Ordering> {
    let integer = |number: &serde_json::Number| {
        number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from))
    };
    Some(integer(left)?.cmp(&integer(right)?))
}

fn compare_values(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        (Value::Number(left), Value::Number(right)) => compare_ints(left, right),
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
        ConditionOp::Not(expected) => attrs.get(&cond.attr) != Some(expected),
        ConditionOp::In(values) => attrs.get(&cond.attr).is_some_and(|v| values.contains(v)),
        ConditionOp::Lt(expected) => attrs
            .get(&cond.attr)
            .and_then(|value| compare_values(value, expected))
            .is_some_and(Ordering::is_lt),
        ConditionOp::Gt(expected) => attrs
            .get(&cond.attr)
            .and_then(|value| compare_values(value, expected))
            .is_some_and(Ordering::is_gt),
        ConditionOp::Lte(expected) => attrs
            .get(&cond.attr)
            .and_then(|value| compare_values(value, expected))
            .is_some_and(Ordering::is_le),
        ConditionOp::Gte(expected) => attrs
            .get(&cond.attr)
            .and_then(|value| compare_values(value, expected))
            .is_some_and(Ordering::is_ge),
        ConditionOp::Matches(regex) => attrs
            .get(&cond.attr)
            .and_then(|v| v.as_str())
            .is_some_and(|s| regex.is_match(s)),
    }
}

pub(crate) fn eval_conditions(
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
        ConditionOp::Not(v) => format!("expected not {v}"),
        ConditionOp::In(_) => "expected in [...]".to_string(),
        ConditionOp::Lt(v) => format!("expected lt {v}"),
        ConditionOp::Gt(v) => format!("expected gt {v}"),
        ConditionOp::Lte(v) => format!("expected lte {v}"),
        ConditionOp::Gte(v) => format!("expected gte {v}"),
        ConditionOp::Matches(_) => "expected matches pattern".to_string(),
        ConditionOp::Present(b) => format!("expected present={b}"),
    }
}

fn first_failing_condition(
    conditions: &[crate::profile::Condition],
    attrs: &serde_json::Map<String, Value>,
    expect_all: bool,
) -> Option<(String, String)> {
    for cond in conditions {
        let passed = eval_condition(cond, attrs);
        if expect_all && !passed {
            return Some((cond.attr.clone(), condition_op_desc(cond)));
        }
        if !expect_all && passed {
            return Some((cond.attr.clone(), condition_op_desc(cond)));
        }
    }
    None
}

/// Check a graph against its profile and return every issue found.
///
/// Covers ID syntax, unknown kinds, attribute schemas, edge endpoint pairs,
/// dangling references and the profile's configured validations. Collects rather
/// than stopping, so one malformed node never hides the rest; `strict` promotes
/// warnings to errors after collection.
#[must_use]
pub fn validate(graph: &LatticeGraph, profile: &Profile, strict: bool) -> Vec<Issue> {
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
                Provenance::new("<profile>", 0),
                profile,
                None,
            ));
        }
    }

    // Both endpoints of every edge count as connected, whether or not the
    // register declared them. An edge is evidence the node is referred to, which
    // is the question ORPHAN_NODE asks; whether the endpoint resolves is
    // VACANCY's question, and answering it here would report one fault twice.
    let mut connected: HashSet<&str> = HashSet::new();
    for edge in graph.iter_edges() {
        connected.insert(&edge.src);
        connected.insert(&edge.tgt);
    }

    let mut unknown_kind_nodes: HashSet<&str> = HashSet::new();

    for node in graph.iter_nodes() {
        let Some(node_kind) = profile.node_kinds().get(&node.kind) else {
            issues.push(issue(
                FindingCode::UnknownKind,
                format!("node '{}' has unknown kind '{}'", node.id, node.kind),
                node.provenance.clone(),
                profile,
                Some(node.id.clone()),
            ));
            unknown_kind_nodes.insert(&node.id);
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

            if !check_type(value, &schema.kind) {
                issues.push(issue(
                    FindingCode::AttrType,
                    format!(
                        "node '{}': attr '{attr_name}' expected type '{}', got {}",
                        node.id,
                        expected_type_name(&schema.kind),
                        type_name(value)
                    ),
                    node.provenance.clone(),
                    profile,
                    Some(node.id.clone()),
                ));
                continue;
            }

            if schema.kind == "enum"
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
                        node.provenance.clone(),
                        profile,
                        Some(node.id.clone()),
                    ));
                }
            }

            if schema.kind == "list"
                && let (Some(items_type), Some(array)) = (&schema.items, value.as_array())
            {
                for (index, item) in array.iter().enumerate() {
                    if !check_type(item, items_type) {
                        issues.push(issue(
                            FindingCode::AttrListItems,
                            format!(
                                "node '{}': attr '{attr_name}' element {index} \
                                     expected type '{}', got {}",
                                node.id,
                                expected_type_name(items_type),
                                type_name(item)
                            ),
                            node.provenance.clone(),
                            profile,
                            Some(node.id.clone()),
                        ));
                    }
                }
            }
        }

        if !connected.contains(node.id.as_str()) && !node_kind.orphan_ok {
            issues.push(issue(
                FindingCode::OrphanNode,
                format!("node '{}' has no edges", node.id),
                node.provenance.clone(),
                profile,
                Some(node.id.clone()),
            ));
        }
    }

    for edge in graph.iter_edges() {
        // Resolve each endpoint once. The checks below ask three separate
        // questions about the same two nodes, and re-hashing the IDs for each is
        // the per-edge cost that grows with the register.
        let src_node = graph.node(&edge.src);
        let tgt_node = graph.node(&edge.tgt);

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

        if tgt_node.is_none() {
            let severity = if edge_kind.is_some_and(|kind| kind.cross_source) {
                Severity::Hint
            } else {
                severity_for(FindingCode::Vacancy, profile)
            };
            issues.push(Issue::new(
                severity,
                FindingCode::Vacancy.as_str(),
                format!(
                    "edge '{}'->'{}' (kind '{}'): target '{}' does not exist",
                    edge.src, edge.tgt, edge.kind, edge.tgt
                ),
                edge.provenance.clone(),
                Some(edge.src.clone()),
            ));
        }

        if src_node.is_none() {
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
        let (Some(src_node), Some(tgt_node)) = (src_node, tgt_node) else {
            continue;
        };
        if unknown_kind_nodes.contains(edge.src.as_str())
            || unknown_kind_nodes.contains(edge.tgt.as_str())
        {
            continue;
        }

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
    }

    let no_configs = Vec::new();
    let coverage_configs = profile
        .validation_configs()
        .get("COVERAGE")
        .unwrap_or(&no_configs);
    for config in coverage_configs {
        let where_conditions =
            crate::profile::parse_condition_block(config.get("where"), "COVERAGE", "where")
                .expect("profile loading validated COVERAGE where conditions");
        let mut keys = read_kind_keys(
            "COVERAGE",
            config,
            &[
                ("target_kind", KindSpace::Node),
                ("edge_kind", KindSpace::Edge),
            ],
            profile,
            &mut issues,
        );
        let edge_kind_name = keys.pop().unwrap();
        let target_kind = keys.pop().unwrap();
        let (Some(target), Some(edge_name)) = (target_kind, edge_kind_name) else {
            continue;
        };

        // Unlike `connected` above, coverage counts only edges whose endpoints the
        // register actually declared: an edge from a source that does not exist is
        // not evidence that the target is verified.
        let mut covered: HashSet<&str> = HashSet::new();
        // Attribution asks less than coverage does: an outgoing edge of this kind
        // shows the author attributed the node, even when its target dangles —
        // the dangling reference is its own finding, not an unmarked source.
        let mut attributed: HashSet<&str> = HashSet::new();
        for edge in graph.iter_edges() {
            if edge.kind != edge_name {
                continue;
            }
            attributed.insert(edge.src.as_str());
            if graph.has_node(&edge.tgt) && graph.has_node(&edge.src) {
                covered.insert(edge.tgt.as_str());
            }
        }

        // The unattributed population: nodes of a kind this edge kind admits as
        // a source toward the config's target kind, with no outgoing edge of it.
        // While any exist, "no incoming edge" cannot distinguish a gap from a
        // missing attribution, so the per-node state is unknown.
        let edge_kind = &profile.edge_kinds()[&edge_name];
        let source_kinds: BTreeSet<&str> = edge_kind
            .allowed
            .iter()
            .filter(|(_, t)| *t == target)
            .map(|(s, _)| s.as_str())
            .collect();
        let population = graph
            .iter_nodes()
            .filter(|n| source_kinds.contains(n.kind.as_str()))
            .count();
        let unattributed = graph
            .iter_nodes()
            .filter(|n| {
                source_kinds.contains(n.kind.as_str()) && !attributed.contains(n.id.as_str())
            })
            .count();
        let state = if unattributed == 0 {
            "unverified"
        } else {
            "unknown"
        };

        for node in graph.iter_nodes() {
            if node.kind == target
                && eval_conditions(&where_conditions, &node.attrs, true)
                && !covered.contains(node.id.as_str())
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
                finding.state = Some(state.to_string());
                issues.push(finding);
            }
        }

        if unattributed > 0 {
            let kinds = source_kinds
                .iter()
                .map(|k| format!("'{k}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let noun = if source_kinds.len() == 1 {
                "kind"
            } else {
                "kinds"
            };
            // The base population rides along: "287 carry no edge" over an
            // unstated 1939 reads as an unwired layer, not a 14.8% gap.
            let pct = unattributed as f64 * 100.0 / population as f64;
            issues.push(issue(
                FindingCode::CoverageUnknown,
                format!(
                    "{unattributed} of {population} node(s) of {noun} {kinds} ({pct:.1}%) \
                     carry no outgoing '{edge_name}' edge; coverage state for '{target}' \
                     is unknown"
                ),
                Provenance::new("<profile>", 0),
                profile,
                None,
            ));
        }
    }

    let deep_configs = profile
        .validation_configs()
        .get("COVERAGE_DEEP")
        .unwrap_or(&no_configs);
    for config in deep_configs {
        let where_conditions =
            crate::profile::parse_condition_block(config.get("where"), "COVERAGE_DEEP", "where")
                .expect("profile loading validated COVERAGE_DEEP where conditions");
        let mut keys = read_kind_keys(
            "COVERAGE_DEEP",
            config,
            &[
                ("target_kind", KindSpace::Node),
                ("via", KindSpace::Edge),
                ("evidence", KindSpace::Edge),
            ],
            profile,
            &mut issues,
        );
        let evidence = keys.pop().unwrap();
        let via = keys.pop().unwrap();
        let target_kind = keys.pop().unwrap();
        let (Some(target), Some(via), Some(evidence)) = (target_kind, via, evidence) else {
            continue;
        };

        let targets: BTreeSet<&str> = graph
            .iter_nodes()
            .filter(|n| n.kind == target)
            .map(|n| n.id.as_str())
            .collect();

        // Same evidence rules as COVERAGE: a dangling edge is not evidence,
        // but its source still counts as attributed.
        let mut evidenced: HashSet<&str> = HashSet::new();
        let mut attributed: HashSet<&str> = HashSet::new();
        // Children keyed by parent — `via` edge targets are the parents. Only
        // target-kind endpoints join the rollup: a foreign-kind or dangling
        // source has no evidence semantics here and contributes no child.
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for edge in graph.iter_edges() {
            if edge.kind == evidence {
                attributed.insert(edge.src.as_str());
                if graph.has_node(&edge.src) && graph.has_node(&edge.tgt) {
                    evidenced.insert(edge.tgt.as_str());
                }
            } else if edge.kind == via
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
            .filter(|t| evidenced.contains(*t))
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

        // Find strongly connected components once with iterative Kosaraju walks.
        // A cyclic uncovered node names every member of its component.
        let uncovered: Vec<&str> = targets
            .iter()
            .copied()
            .filter(|t| !covered.contains(*t))
            .collect();
        let uncovered_set: HashSet<&str> = uncovered.iter().copied().collect();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut finish_order = Vec::with_capacity(uncovered.len());
        for &start in &uncovered {
            if !seen.insert(start) {
                continue;
            }
            let mut stack = vec![(start, 0)];
            while let Some((node, next)) = stack.last_mut() {
                let neighbours = parents_of.get(node).map(Vec::as_slice).unwrap_or(&[]);
                if *next < neighbours.len() {
                    let neighbour = neighbours[*next];
                    *next += 1;
                    if uncovered_set.contains(neighbour) && seen.insert(neighbour) {
                        stack.push((neighbour, 0));
                    }
                } else {
                    finish_order.push(*node);
                    stack.pop();
                }
            }
        }

        seen.clear();
        let mut cyclic_components: Vec<Vec<&str>> = Vec::new();
        let mut component_of: HashMap<&str, usize> = HashMap::new();
        for &start in finish_order.iter().rev() {
            if !seen.insert(start) {
                continue;
            }
            let mut members = Vec::new();
            let mut stack = vec![start];
            while let Some(node) = stack.pop() {
                members.push(node);
                for &child in children.get(node).into_iter().flatten() {
                    if uncovered_set.contains(child) && seen.insert(child) {
                        stack.push(child);
                    }
                }
            }
            let self_loop = members.len() == 1
                && parents_of
                    .get(start)
                    .is_some_and(|parents| parents.contains(&start));
            if members.len() > 1 || self_loop {
                members.sort_unstable();
                let component = cyclic_components.len();
                for &member in &members {
                    component_of.insert(member, component);
                }
                cyclic_components.push(members);
            }
        }

        let evidence_kind = &profile.edge_kinds()[&evidence];
        let source_kinds: BTreeSet<&str> = evidence_kind
            .allowed
            .iter()
            .filter(|(_, t)| *t == target)
            .map(|(s, _)| s.as_str())
            .collect();
        let population = graph
            .iter_nodes()
            .filter(|n| source_kinds.contains(n.kind.as_str()))
            .count();
        let unattributed = graph
            .iter_nodes()
            .filter(|n| {
                source_kinds.contains(n.kind.as_str()) && !attributed.contains(n.id.as_str())
            })
            .count();
        let state = if unattributed == 0 {
            "unverified"
        } else {
            "unknown"
        };

        for node in graph.iter_nodes() {
            if node.kind != target
                || !eval_conditions(&where_conditions, &node.attrs, true)
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
            finding.state = Some(state.to_string());
            issues.push(finding);
        }

        if unattributed > 0 {
            let kinds = source_kinds
                .iter()
                .map(|k| format!("'{k}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let noun = if source_kinds.len() == 1 {
                "kind"
            } else {
                "kinds"
            };
            let pct = unattributed as f64 * 100.0 / population as f64;
            issues.push(issue(
                FindingCode::CoverageUnknown,
                format!(
                    "{unattributed} of {population} node(s) of {noun} {kinds} ({pct:.1}%) \
                     carry no outgoing '{evidence}' edge; deep coverage state for \
                     '{target}' is unknown"
                ),
                Provenance::new("<profile>", 0),
                profile,
                None,
            ));
        }
    }

    for constraint in profile.constraint_configs() {
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
            let reject_fail = first_failing_condition(&constraint.reject, &node.attrs, false);
            let failure = expect_fail.or(reject_fail);
            if let Some((attr, op_desc)) = failure {
                let message = constraint
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("constraint failed: '{attr}' {op_desc}"));
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

    let mut issues = resolve_axes(issues, graph, profile);

    if strict {
        for issue in &mut issues {
            if issue.severity == Severity::Warning {
                issue.severity = Severity::Error;
            }
        }
    }

    sort_issues(&mut issues);
    issues
}

/// The reporting order: location, then code, then identity. Stable, so equal keys
/// keep collection order.
pub(crate) fn sort_issues(issues: &mut [Issue]) {
    issues.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
}

#[cfg(test)]
mod default_severity_tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::{FindingCode, default_severity, validate};
    use crate::graph::{EdgeSpec, LatticeGraph};
    use crate::profile::load_profile;
    use crate::types::{Provenance, Severity};

    #[test]
    fn warning_finding_codes_have_explicit_defaults() {
        assert_eq!(default_severity(FindingCode::OrphanNode), Severity::Warning);
        assert_eq!(default_severity(FindingCode::Coverage), Severity::Warning);
        assert_eq!(
            default_severity(FindingCode::PathwayUnresolved),
            Severity::Warning
        );
    }

    fn dangling_severity(cross_source: bool) -> Severity {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "lattice-cross-source-{}-{}.yaml",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let flag = if cross_source {
            "    cross_source: true\n"
        } else {
            ""
        };
        std::fs::write(
            &path,
            format!(
                "name: t\nprofile_version: \"1.0.0\"\n\
                 node_kinds:\n  req:\n    id_pattern: \"^REQ-[0-9]+$\"\n\
                 edge_kinds:\n  derives:\n    allowed: [[req, req]]\n{flag}"
            ),
        )
        .expect("profile fixture writes");
        let profile = load_profile(&path).expect("profile fixture loads");

        let mut graph = LatticeGraph::new();
        graph
            .add_node(
                "REQ-1",
                "req",
                Default::default(),
                Provenance::new("requirements.md", 1),
            )
            .expect("source node is unique");
        graph.add_edge(
            EdgeSpec {
                src: "REQ-1".into(),
                tgt: "REQ-2".into(),
                kind: "derives".into(),
                attrs: Default::default(),
            },
            Provenance::new("requirements.md", 1),
        );

        validate(&graph, &profile, false)
            .into_iter()
            .find(|finding| finding.code == "VACANCY")
            .expect("dangling edge produces a finding")
            .severity
    }

    #[test]
    fn cross_source_dangling_target_is_a_hint() {
        assert_eq!(dangling_severity(true), Severity::Hint);
    }

    #[test]
    fn ordinary_dangling_target_remains_an_error() {
        assert_eq!(dangling_severity(false), Severity::Error);
    }
}
