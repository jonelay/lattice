//! Checking a graph against its profile: ID syntax, kinds, attrs, edges, coverage.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use serde_json::Value;

use crate::document::type_name;
use crate::graph::LatticeGraph;
use crate::profile::{Profile, ValidationConfig};
use crate::types::{Issue, Provenance, Severity};

#[derive(Clone, Copy)]
pub(crate) enum FindingCode {
    AttrEnum,
    AttrListItems,
    AttrRequired,
    AttrType,
    AxisUnresolved,
    ConfigError,
    Coverage,
    CoverageDeep,
    CoverageUnknown,
    DanglingRef,
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
            Self::AxisUnresolved => "AXIS_UNRESOLVED",
            Self::ConfigError => "CONFIG_ERROR",
            Self::Coverage => "COVERAGE",
            Self::CoverageDeep => "COVERAGE_DEEP",
            Self::CoverageUnknown => "COVERAGE_UNKNOWN",
            Self::DanglingRef => "DANGLING_REF",
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
            "AXIS_UNRESOLVED" => Some(Self::AxisUnresolved),
            "CONFIG_ERROR" => Some(Self::ConfigError),
            "COVERAGE" => Some(Self::Coverage),
            "COVERAGE_DEEP" => Some(Self::CoverageDeep),
            "COVERAGE_UNKNOWN" => Some(Self::CoverageUnknown),
            "DANGLING_REF" => Some(Self::DanglingRef),
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
        | FindingCode::DanglingRef
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
        FindingCode::AxisUnresolved
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

fn check_type(value: &Value, expected: &str) -> bool {
    match expected {
        "string" | "enum" => value.is_string(),
        "int" => value.is_i64() || value.is_u64(),
        "bool" => value.is_boolean(),
        "list" => value.is_array(),
        _ => true,
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

/// Demote findings whose node is not yet due on its bound ordering axis.
///
/// Runs after collection and before `--strict`, so a demoted finding is already
/// `info` when promotion looks at it and survives. Starts from each issue's
/// effective severity rather than recomputing it, or an adapter code with no
/// shipped default would silently drop to the warning fallback.
fn resolve_axes(issues: Vec<Issue>, graph: &LatticeGraph, profile: &Profile) -> Vec<Issue> {
    if profile.axis_bindings().is_empty() {
        return issues;
    }

    // One finding per binding, not per issue: the mismatch is a property of the
    // pairing, and repeating it per finding would bury the findings it reports
    // about. Keyed on the code too, so two codes bound to one missing axis each
    // say so — a single finding could name only one of them.
    let mut unresolved: BTreeMap<(String, String), Issue> = BTreeMap::new();
    let mut resolved: Vec<Issue> = Vec::with_capacity(issues.len());

    for issue in issues {
        let Some(binding) = profile.axis_bindings().get(&issue.code) else {
            resolved.push(issue);
            continue;
        };

        let Some(axis) = graph.axis(&binding.axis) else {
            unresolved
                .entry((issue.code.clone(), binding.axis.clone()))
                .or_insert_with(|| {
                    Issue::new(
                        severity_for(FindingCode::AxisUnresolved, profile),
                        FindingCode::AxisUnresolved.as_str(),
                        format!(
                            "profile binds '{}' to axis '{}', which the target does not declare",
                            issue.code, binding.axis
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

        if axis.is_member(position) && !axis.is_after(position) {
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

/// Adapter issues at the severities the profile and its axes decide.
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
    // DANGLING_REF's question, and answering it here would report one fault twice.
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
                        schema.kind,
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
                                     expected type '{items_type}', got {}",
                                node.id,
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
            issues.push(issue(
                FindingCode::DanglingRef,
                format!(
                    "edge '{}'->'{}' (kind '{}'): target '{}' does not exist",
                    edge.src, edge.tgt, edge.kind, edge.tgt
                ),
                edge.provenance.clone(),
                profile,
                Some(edge.src.clone()),
            ));
        }

        if src_node.is_none() {
            issues.push(issue(
                FindingCode::DanglingRef,
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
            if node.kind == target && !covered.contains(node.id.as_str()) {
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
            if node.kind != target || covered.contains(node.id.as_str()) {
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
    use super::{FindingCode, default_severity};
    use crate::types::Severity;

    #[test]
    fn warning_finding_codes_have_explicit_defaults() {
        assert_eq!(default_severity(FindingCode::OrphanNode), Severity::Warning);
        assert_eq!(default_severity(FindingCode::Coverage), Severity::Warning);
        assert_eq!(
            default_severity(FindingCode::AxisUnresolved),
            Severity::Warning
        );
    }
}
