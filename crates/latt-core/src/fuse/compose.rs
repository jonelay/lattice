//! Assembling the fused graph from source traces and validating it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Map, Value, json};

use crate::graph::{EdgeSpec, LatticeGraph};
use crate::profile::{Kinds, NodeKind, Profile, load_profile, load_profile_value_with};
use crate::types::{
    FuseEdge, FuseFinding, FuseNode, FuseReport, Issue, Origin, PathwayEntry, Severity,
};
use crate::validate::{
    FindingCode, SuppressionIndex, apply_suppressions, collect, default_severity, issue,
    resolve_severity, sort_issues, unused_overrides, unused_suppressions, vacancy_target_message,
};

use super::fuse_profile::FuseProfile;
use super::trace::{SourceTrace, qualify};
use super::{
    AMBIGUOUS_CROSS_REF, CROSS_SOURCE_DUPLICATE_ID, FuseError, Manifest, empty_report,
    failed_report, promote_report, source_failure,
};

/// Refuse a fuse profile whose edge endpoint names a source the manifest lacks, or a
/// kind its source profile does not declare.
///
/// Such a pair matches nothing, and composition would give its kind an empty
/// declaration that accepts any ID, so a misspelt `reqs:requrement` would read as
/// a clean join. The fault is in the declaration, not the register, so it is a
/// setup error (exit 2) naming every such endpoint and what does exist. A source
/// whose profile did not load is left to its own failure.
pub(super) fn check_endpoints(
    manifest: &Manifest,
    profile: &FuseProfile,
    source_profiles: &HashMap<String, Profile>,
) -> Result<(), FuseError> {
    let sources: Vec<&str> = manifest.sources.iter().map(|s| s.name.as_str()).collect();
    let mut faults = Vec::new();
    for (edge_kind, declaration) in &profile.edges {
        for endpoint in declaration.allowed.iter().flat_map(|(s, t)| [s, t]) {
            let Some((source, kind)) = endpoint.split_once(':') else {
                continue;
            };
            let named = format!("fuse profile edge kind '{edge_kind}' names '{endpoint}'");
            if !sources.contains(&source) {
                faults.push(format!(
                    "{named}, but the manifest has no source '{source}' (sources: {})",
                    sources.join(", ")
                ));
            } else if let Some(source_profile) = source_profiles.get(source)
                && !source_profile.node_kinds().contains_key(kind)
            {
                let kinds: Vec<&str> = source_profile
                    .node_kinds()
                    .keys()
                    .map(String::as_str)
                    .collect();
                faults.push(format!(
                    "{named}, but source '{source}' declares no kind '{kind}' (kinds: {})",
                    kinds.join(", ")
                ));
            }
        }
    }
    if faults.is_empty() {
        Ok(())
    } else {
        Err(FuseError::FuseProfile(faults.join("; ")))
    }
}

/// Compose already-parsed traces, one per manifest source in manifest order.
///
/// Split from [`fuse`](super::fuse) so a caller that already holds the traces can compose
/// them without spawning the binary. The manifest must come from
/// [`load_manifest`](super::load_manifest), which is what guarantees unique, colon-free source names.
///
/// Each source's profile is still read from disk, for its ID patterns. One that
/// cannot be loaded is a `SOURCE_FAILURE`, as a failed trace run is in [`fuse`](super::fuse):
/// the report could not run and carries the other sources' own findings, since
/// composing without the patterns would check that source's IDs against nothing.
///
/// # Errors
///
/// [`FuseError::Manifest`] when `traces` does not hold one trace per manifest
/// source; the composition errors [`FuseError::Duplicate`] and
/// [`FuseError::Pathway`]; and [`FuseError::FuseProfile`] or
/// [`FuseError::Profile`] when the composed profile cannot be loaded.
pub fn assemble(
    manifest: &Manifest,
    profile: &FuseProfile,
    traces: Vec<SourceTrace>,
    strict: bool,
) -> Result<FuseReport, FuseError> {
    if traces.len() != manifest.sources.len() {
        return Err(FuseError::Manifest(
            "expected one trace per manifest source".into(),
        ));
    }
    let mut kind_patterns = HashMap::new();
    let mut source_profiles = HashMap::new();
    let mut failures = Vec::new();
    for source in &manifest.sources {
        match load_profile(&source.profile) {
            Ok(source_profile) => {
                kind_patterns.extend(
                    source_profile
                        .node_kinds()
                        .iter()
                        .map(|(kind, node_kind)| (qualify(&source.name, kind), node_kind.clone())),
                );
                source_profiles.insert(source.name.clone(), source_profile);
            }
            Err(e) => failures.push(source_failure(
                &source.name,
                format!(
                    "could not load source profile '{}': {e}",
                    source.profile.display()
                ),
            )),
        }
    }
    if !failures.is_empty() {
        let runs = manifest
            .sources
            .iter()
            .zip(traces)
            .map(|(source, trace)| {
                let failed = failures
                    .iter()
                    .any(|f| f.source.as_deref() == Some(source.name.as_str()));
                (!failed).then_some(trace)
            })
            .collect();
        return Ok(failed_report(manifest, runs, failures, strict));
    }
    check_endpoints(manifest, profile, &source_profiles)?;
    let mut report = empty_report(manifest);
    let mut occurrences: HashMap<String, Vec<usize>> = HashMap::new();
    let mut raw_targets = Vec::new();
    let mut raw_sources = Vec::new();
    for (source, SourceTrace(trace)) in manifest.sources.iter().zip(traces) {
        for node in trace.entries {
            let id = qualify(&source.name, &node.id);
            let kind = qualify(&source.name, &node.kind);
            for edge in node.edges {
                report.edges.push(FuseEdge {
                    src: id.clone(),
                    tgt: qualify(&source.name, &edge.tgt),
                    kind: edge.kind,
                    attrs: edge.attrs,
                    provenance: edge.provenance.attributed(&source.name),
                    source_kind: kind.clone(),
                    target_kind: None,
                    target_source: None,
                });
                raw_targets.push(edge.tgt);
                raw_sources.push(node.id.clone());
            }
            report.findings.extend(
                node.findings
                    .into_iter()
                    .map(|f| f.attributed(&source.name, Some(&node.id))),
            );
            occurrences
                .entry(node.id)
                .or_default()
                .push(report.nodes.len());
            report.nodes.push(FuseNode {
                id,
                kind,
                attrs: node.attrs,
                provenance: node.provenance.attributed(&source.name),
            });
        }
        report.findings.extend(
            trace
                .unattachable_findings
                .into_iter()
                .map(|f| f.attributed(&source.name, None)),
        );
        report
            .pathways
            .extend(trace.pathways.into_iter().map(|p| PathwayEntry {
                name: qualify(&source.name, &p.name),
                order: p.order,
                current: p.current,
            }));
    }
    // Everything from here on is the command's own: the fuse profile's SUPPRESS
    // entries reach these findings and no source's.
    let mut merged = report.findings.len();
    // Reported in the order the IDs were first met: a node's first index rises
    // with insertion, so sorting on it recovers that order without a second
    // list of the IDs.
    let mut duplicates: Vec<(&str, &[usize])> = occurrences
        .iter()
        .filter(|(_, indices)| indices.len() >= 2)
        .map(|(raw, indices)| (raw.as_str(), indices.as_slice()))
        .collect();
    duplicates.sort_unstable_by_key(|(_, indices)| indices[0]);
    for (raw, indices) in duplicates {
        let locations: Vec<_> = indices
            .iter()
            .map(|i| report.nodes[*i].provenance.clone())
            .collect();
        let sources = locations
            .iter()
            .map(|p| p.source.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        report.findings.push(FuseFinding {
            issue: Issue::new(
                profile.severity(CROSS_SOURCE_DUPLICATE_ID, Severity::Error),
                CROSS_SOURCE_DUPLICATE_ID,
                format!("node ID '{raw}' occurs in sources {sources}"),
                locations[0].location.clone(),
                Some(raw.to_string()),
            ),
            source: None,
            locations,
        });
    }
    // A source's own trace run cannot see other sources, so an edge fuse
    // judges — resolved, or declared and unresolved — may still carry that
    // source's VACANCY; it is withdrawn below.
    let mut withdrawn: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ((edge, raw), raw_src) in report.edges.iter_mut().zip(raw_targets).zip(raw_sources) {
        let declaration = profile.edges.get(&edge.kind);
        // Almost every edge has zero or one candidate: the first is kept and
        // the rest are counted, with no list built per edge; only a finding
        // walks the candidates again, for their locations.
        let candidates = || {
            occurrences
                .get(&raw)
                .into_iter()
                .flatten()
                .map(|i| &report.nodes[*i])
                .filter(|node| match declaration {
                    Some(d) => d
                        .allowed
                        .iter()
                        .any(|(s, t)| s == &edge.source_kind && t == &node.kind),
                    None => node.provenance.source == edge.provenance.source,
                })
        };
        let mut walk = candidates();
        let first = walk.next();
        let others = walk.count();
        if let (Some(target), 0) = (first, others) {
            withdrawn
                .entry(edge.provenance.source.clone())
                .or_default()
                .insert(vacancy_target_message(&raw_src, &raw, &edge.kind));
            edge.tgt = target.id.clone();
            edge.target_kind = Some(target.kind.clone());
            edge.target_source = Some(target.provenance.source.clone());
        } else if declaration.is_some() {
            let (code, message, default) = if first.is_none() {
                withdrawn
                    .entry(edge.provenance.source.clone())
                    .or_default()
                    .insert(vacancy_target_message(&raw_src, &raw, &edge.kind));
                (
                    "VACANCY",
                    format!(
                        "edge '{}'->'{raw}' (kind '{}'): target '{raw}' does not exist in an allowed source kind",
                        edge.src, edge.kind
                    ),
                    default_severity(FindingCode::Vacancy),
                )
            } else {
                (
                    AMBIGUOUS_CROSS_REF,
                    format!(
                        "edge '{}'->'{raw}' (kind '{}') matches {} allowed targets",
                        edge.src,
                        edge.kind,
                        others + 1
                    ),
                    Severity::Error,
                )
            };
            report.findings.push(FuseFinding {
                issue: Issue::new(
                    profile.severity(code, default),
                    code,
                    message,
                    edge.provenance.location.clone(),
                    Some(edge.src.clone()),
                ),
                source: Some(edge.provenance.source.clone()),
                locations: candidates().map(|n| n.provenance.clone()).collect(),
            });
        }
    }
    // The composed graph owns the orphan question: every source UNREFERENCED
    // and UNTRACED goes, and the composed run's own finding for the same node
    // keeps the suppression the source decided.
    let mut carried_suppression = BTreeSet::new();
    let own = report.findings.split_off(merged);
    report.findings.retain(|finding| {
        let Some(source) = finding.source.as_deref() else {
            return true;
        };
        let code = finding.issue.code.as_str();
        if is_orphan_code(code) {
            if finding.issue.suppressed
                && let Some(id) = &finding.issue.node_id
            {
                carried_suppression.insert((code.to_string(), id.clone()));
            }
            return false;
        }
        !(code == "VACANCY"
            && withdrawn
                .get(source)
                .is_some_and(|messages| messages.contains(&finding.issue.message)))
    });
    merged = report.findings.len();
    report.findings.extend(own);
    validate_composed(
        &mut report,
        profile,
        &Composition {
            kind_patterns: &kind_patterns,
            source_profiles: &source_profiles,
            carried_suppression: &carried_suppression,
        },
        merged,
        strict,
    )?;
    Ok(report)
}

/// True when a source's `SUPPRESS` entries would suppress `issue` in the
/// source's own run, where the node ID carries no `source:` qualifier.
fn source_suppresses(suppressions: &SuppressionIndex<'_>, issue: &Issue) -> bool {
    let raw = issue
        .node_id
        .as_deref()
        .map(|id| id.split_once(':').map_or(id, |(_, raw)| raw));
    suppressions
        .decide(&issue.code, raw)
        .unwrap_or(issue.suppressed)
}

/// True when the composed run, not a source, owns findings of this code.
fn is_orphan_code(code: &str) -> bool {
    code == FindingCode::Unreferenced.as_str() || code == FindingCode::Untraced.as_str()
}

/// What the composed validation pass carries over from the source profiles and
/// the withdrawn source findings.
struct Composition<'a> {
    /// Each composed kind's source `NodeKind`: its ID pattern and `orphan_ok`.
    kind_patterns: &'a HashMap<String, NodeKind>,
    /// Each source's profile: its severity overrides, for an orphan finding
    /// the fuse profile does not override itself, and its `SUPPRESS` entries,
    /// which reach the orphan findings its own run never reported.
    source_profiles: &'a HashMap<String, Profile>,
    /// `(code, composed node ID)` of every withdrawn orphan finding the source
    /// had suppressed.
    carried_suppression: &'a BTreeSet<(String, String)>,
}

fn compose_profile(document: &Value) -> Result<Profile, FuseError> {
    let norway = serde_norway::to_value(document)
        .map_err(|e| FuseError::FuseProfile(format!("fuse profile conversion: {e}")))?;
    Ok(load_profile_value_with(norway, "fuse", Kinds::Deferred)?)
}

/// Run the standard validators over the composed graph, then the tail of the
/// finding pipeline once over every fuse-collected finding (`merged..`):
/// stale-suppression detection, `--strict`, suppression. Source findings are
/// promoted alongside but keep the suppression their own profile decided.
///
/// The orphan validators run here with each source kind's own `orphan_ok`, so
/// an edge resolved across sources counts on its target; the source runs'
/// orphan findings were withdrawn in [`assemble`]. A composed orphan finding
/// takes the fuse profile's severity for its code when one is declared, else
/// its source profile's, and stays suppressed when the source's was.
fn validate_composed(
    report: &mut FuseReport,
    profile: &FuseProfile,
    composition: &Composition<'_>,
    merged: usize,
    strict: bool,
) -> Result<(), FuseError> {
    let kind_patterns = composition.kind_patterns;
    let mut graph = LatticeGraph::new();
    let mut nodes = Map::new();
    let mut edges: BTreeMap<String, Vec<(String, String)>> = profile
        .edges
        .iter()
        .map(|(name, edge)| (name.clone(), edge.allowed.clone()))
        .collect();
    // The orphan checks ask which directions an edge could supply, so every
    // source's own pairs count, qualified, beside the fuse profile's. Observed
    // edges alone would leave a kind that received none unchecked, which is
    // exactly the node an orphan finding is for. Every composed edge's pair is
    // added below, so the extra pairs admit nothing `EDGE_CONSTRAINT` would see.
    let mut sources: Vec<_> = composition.source_profiles.iter().collect();
    sources.sort_unstable_by_key(|(source, _)| *source);
    for (source, source_profile) in sources {
        for (name, edge_kind) in source_profile.edge_kinds() {
            let pairs = edges.entry(name.clone()).or_default();
            for (src, tgt) in &edge_kind.allowed {
                let pair = (qualify(source, src), qualify(source, tgt));
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
        }
    }
    for node in &report.nodes {
        if !nodes.contains_key(&node.kind) {
            // The composed kind accepts any ID: assembly wrote the qualifier,
            // and splicing the source pattern in behind it is not a regex
            // transformation (a nested anchor or an escaped `$` changes
            // meaning). The raw ID is checked below against the source's own
            // compiled pattern. `orphan_ok` is the source kind's: the composed
            // run owns the orphan question.
            let orphan_ok = kind_patterns
                .get(&node.kind)
                .is_none_or(|kind| kind.orphan_ok);
            nodes.insert(
                node.kind.clone(),
                json!({"id_pattern": ".*", "orphan_ok": orphan_ok}),
            );
        }
        graph.add_node(
            &node.id,
            &node.kind,
            node.attrs.clone(),
            node.provenance.location.clone(),
        )?;
    }
    for edge in &report.edges {
        if let Some(target) = &edge.target_kind {
            if !edges.contains_key(&edge.kind) {
                edges.insert(edge.kind.clone(), Vec::new());
            }
            let Some(pairs) = edges.get_mut(&edge.kind) else {
                continue;
            };
            if !pairs
                .iter()
                .any(|(s, t)| s == &edge.source_kind && t == target)
            {
                pairs.push((edge.source_kind.clone(), target.clone()));
            }
            graph.add_edge(
                EdgeSpec {
                    src: edge.src.clone(),
                    tgt: edge.tgt.clone(),
                    kind: edge.kind.clone(),
                    attrs: edge.attrs.clone(),
                },
                edge.provenance.location.clone(),
            );
        }
    }
    for pairs in edges.values() {
        for (src, tgt) in pairs {
            for kind in [src, tgt] {
                nodes
                    .entry(kind.clone())
                    .or_insert(json!({"id_pattern": ".*", "orphan_ok": true}));
            }
        }
    }
    for pathway in &report.pathways {
        graph.set_pathway(&pathway.name, pathway.order.clone(), &pathway.current)?;
    }
    // COVERAGE entries whose edge_kind is undeclared in the fuse profile get a
    // CONFIG_ERROR here rather than flowing into the standard validator, where the
    // auto-added observed kinds would mask the misconfiguration.
    let mut validations = Vec::new();
    for (code, config) in &profile.validations {
        if code == "COVERAGE"
            && let Some(ek) = config.get("edge_kind").and_then(Value::as_str)
            && !profile.edges.contains_key(ek)
        {
            report.findings.push(FuseFinding {
                issue: Issue::new(
                    default_severity(FindingCode::ConfigError),
                    "CONFIG_ERROR",
                    format!("COVERAGE: edge_kind '{ek}' is not declared in fuse edge_kinds"),
                    Origin::FuseProfile,
                    None,
                ),
                source: None,
                locations: Vec::new(),
            });
            continue;
        }
        validations.push(json!({code: config}));
    }
    let edges: Map<_, _> = edges
        .into_iter()
        .map(|(name, pairs)| (name, json!({"allowed": pairs})))
        .collect();
    let document = json!({
        "name": "fuse",
        "profile_version": "1.0.0",
        "node_kinds": nodes,
        "edge_kinds": edges,
        "pathways": report.pathways.iter().map(|p| &p.name).collect::<Vec<_>>(),
        "validations": validations,
    });
    let standard = compose_profile(&document)?;
    let mut composed = collect(&graph, &standard);
    let source_of: HashMap<&str, &str> = report
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.provenance.source.as_str()))
        .collect();
    // An unresolved edge is not in the composed graph (its VACANCY is already
    // reported), yet the source's own run counted it: it leaves its node, and
    // it reaches the same-source node its raw target names when there is one.
    // The orphan validators never consult allowed pairs; neither does this.
    let mut leaves = BTreeSet::new();
    let mut reaches = BTreeSet::new();
    for edge in report
        .edges
        .iter()
        .filter(|edge| edge.target_kind.is_none())
    {
        leaves.insert(edge.src.as_str());
        if source_of.contains_key(edge.tgt.as_str()) {
            reaches.insert(edge.tgt.as_str());
        }
    }
    composed.retain(|issue| {
        let Some(id) = issue.node_id.as_deref() else {
            return true;
        };
        !((issue.code == FindingCode::Untraced.as_str() && leaves.contains(id))
            || (issue.code == FindingCode::Unreferenced.as_str() && reaches.contains(id)))
    });
    let fuse_overrides: BTreeSet<&str> = profile
        .validations
        .iter()
        .filter(|(_, config)| config.contains_key("severity"))
        .map(|(code, _)| code.as_str())
        .collect();
    let source_suppressions: HashMap<&str, SuppressionIndex<'_>> = composition
        .source_profiles
        .iter()
        .map(|(source, profile)| (source.as_str(), SuppressionIndex::new(profile)))
        .collect();
    for issue in composed.iter_mut() {
        let code = if issue.code == FindingCode::Unreferenced.as_str() {
            FindingCode::Unreferenced
        } else if issue.code == FindingCode::Untraced.as_str() {
            FindingCode::Untraced
        } else {
            continue;
        };
        let Some(id) = issue.node_id.as_deref() else {
            continue;
        };
        let source_profile = source_of
            .get(id)
            .and_then(|source| composition.source_profiles.get(*source));
        if !fuse_overrides.contains(issue.code.as_str()) {
            let override_ = source_profile
                .and_then(|profile| profile.validation_overrides().get(&issue.code))
                .copied();
            issue.severity = resolve_severity(default_severity(code), override_);
        }
        if composition
            .carried_suppression
            .contains(&(issue.code.clone(), id.to_string()))
            || source_of
                .get(id)
                .and_then(|source| source_suppressions.get(*source))
                .is_some_and(|suppressions| source_suppresses(suppressions, issue))
        {
            issue.suppressed = true;
        }
    }
    for node in &report.nodes {
        let Some(node_kind) = kind_patterns.get(&node.kind) else {
            continue;
        };
        let raw = node
            .id
            .split_once(':')
            .map_or(node.id.as_str(), |(_, raw)| raw);
        if !node_kind.id_matches(raw) {
            composed.push(issue(
                FindingCode::IdFormat,
                format!(
                    "node '{}' does not match pattern '{}' for kind '{}'",
                    node.id, node_kind.id_pattern_source, node.kind
                ),
                node.provenance.location.clone(),
                &standard,
                Some(node.id.clone()),
            ));
        }
    }
    sort_issues(&mut composed);
    for issue in composed {
        let source = issue
            .node_id
            .as_deref()
            .and_then(|id| source_of.get(id))
            .map(|source| (*source).to_string());
        report.findings.push(FuseFinding {
            issue,
            source,
            locations: Vec::new(),
        });
    }
    // Overrides before suppressions, as `validate::finish` orders them.
    let unused = unused_overrides(
        report.findings[merged..].iter().map(|f| &f.issue),
        &standard,
        &Origin::FuseProfile,
    );
    report
        .findings
        .extend(unused.into_iter().map(|issue| FuseFinding {
            issue,
            source: None,
            locations: Vec::new(),
        }));
    let unused = unused_suppressions(
        report.findings[merged..].iter().map(|f| &f.issue),
        &standard,
        &Origin::FuseProfile,
    );
    report
        .findings
        .extend(unused.into_iter().map(|issue| FuseFinding {
            issue,
            source: None,
            locations: Vec::new(),
        }));
    promote_report(report, strict);
    apply_suppressions(
        report.findings[merged..].iter_mut().map(|f| &mut f.issue),
        &standard,
    );
    Ok(())
}
