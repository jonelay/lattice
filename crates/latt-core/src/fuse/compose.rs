//! Assembling the fused graph from source traces and validating it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{Map, Value, json};

use crate::graph::{EdgeSpec, LatticeGraph};
use crate::profile::{Kinds, NodeKind, Profile, load_profile, load_profile_value_with};
use crate::scope::{EmptyOf, KindTally};
use crate::types::{
    FuseEdge, FuseFinding, FuseNode, FuseReport, FuseStage, Issue, Note, Origin, PathwayEntry,
    Severity,
};
use crate::validate::{
    FindingCode, SuppressionIndex, collect, default_severity, issue, promote, resolve_severity,
    sort_issues, unused_message, unused_overrides, unused_suppressions, vacancy_target_message,
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
                FuseStage::SourceProfile,
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
    let mut raw_sources: Vec<usize> = Vec::new();
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
                raw_sources.push(report.nodes.len());
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
            stage: None,
        });
    }
    // A source's own trace run cannot see other sources, so an edge fuse
    // judges — resolved, or declared and unresolved — may still carry that
    // source's VACANCY; it is withdrawn below.
    let mut withdrawn: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ((edge, raw), source_at) in report.edges.iter_mut().zip(raw_targets).zip(raw_sources) {
        let raw_src = raw_id(&report.nodes[source_at].id);
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
                .insert(vacancy_target_message(raw_src, &raw, &edge.kind));
            edge.tgt = target.id.clone();
            edge.target_kind = Some(target.kind.clone());
            edge.target_source = Some(target.provenance.source.clone());
        } else if declaration.is_some() {
            let (code, message, default) = if first.is_none() {
                withdrawn
                    .entry(edge.provenance.source.clone())
                    .or_default()
                    .insert(vacancy_target_message(raw_src, &raw, &edge.kind));
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
                stage: None,
            });
        }
    }
    // Read before anything is withdrawn: the notes each source attached, which
    // reach the fuse's own findings on the same code and node, and the nodes
    // whose malformed ID a source already reported. A qualified ID names its
    // source, so it alone keys both.
    let mut carried_notes = HashMap::new();
    let mut reported_id_format = BTreeSet::new();
    for finding in report.findings[..merged]
        .iter()
        .filter(|finding| finding.source.is_some())
    {
        let Some(id) = &finding.issue.node_id else {
            continue;
        };
        if let Some(note) = &finding.issue.note {
            carried_notes
                .entry((finding.issue.code.clone(), id.clone()))
                .or_insert_with(|| note.clone());
        }
        if finding.issue.code == FindingCode::IdFormat.as_str() {
            reported_id_format.insert(id.clone());
        }
    }
    // The composed graph owns the orphan question: every source UNREFERENCED
    // and UNTRACED goes, and the composed run's own finding for the same node
    // keeps the suppression the source decided. It owns EMPTY_KIND too: a kind
    // is empty only when no source populated it, and a source's edges that
    // dangle here are still edges of their kind.
    let mut carried_suppression = BTreeSet::new();
    // The withdrawn orphan findings, as `(code, raw ID)`, of each source whose
    // profile suppresses an orphan code: what its `SUPPRESS_UNUSED` was judged
    // against, kept so the composed run can judge the entry again.
    let mut withdrawn_orphans: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    let suppresses_orphans = |source: &str| {
        source_profiles
            .get(source)
            .is_some_and(|p: &Profile| p.suppressions().iter().any(|e| is_orphan_code(&e.code)))
    };
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
            if suppresses_orphans(source) {
                withdrawn_orphans
                    .entry(source.to_string())
                    .or_default()
                    .push((
                        code.to_string(),
                        finding
                            .issue
                            .node_id
                            .as_deref()
                            .map(raw_id)
                            .map(str::to_string),
                    ));
            }
            return false;
        }
        if code == FindingCode::EmptyKind.as_str() {
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
            withdrawn_orphans: &withdrawn_orphans,
            carried_notes: &carried_notes,
            reported_id_format: &reported_id_format,
        },
        merged,
        strict,
    )?;
    Ok(report)
}

/// True when a source's `SUPPRESS` entries would suppress `issue` in the
/// source's own run, where the node ID carries no `source:` qualifier.
fn source_suppresses(suppressions: &SuppressionIndex<'_>, issue: &Issue) -> bool {
    let raw = issue.node_id.as_deref().map(raw_id);
    suppressions
        .decide(&issue.code, raw)
        .unwrap_or(issue.suppressed)
}

/// Return a composed ID without its `source:` qualifier.
fn raw_id(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, raw)| raw)
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
    /// Each source's withdrawn orphan findings as `(code, raw ID)`, for the
    /// sources whose profile suppresses an orphan code.
    withdrawn_orphans: &'a HashMap<String, Vec<(String, Option<String>)>>,
    /// `(code, composed node ID)` → the note a source attached to its finding.
    carried_notes: &'a HashMap<(String, String), Note>,
    /// Composed node IDs whose `ID_FORMAT` a source already reported.
    reported_id_format: &'a BTreeSet<String>,
}

fn compose_profile(document: &Value) -> Result<Profile, FuseError> {
    let norway = serde_norway::to_value(document)
        .map_err(|e| FuseError::FuseProfile(format!("fuse profile conversion: {e}")))?;
    Ok(load_profile_value_with(norway, "fuse", Kinds::Deferred)?)
}

/// One `EMPTY_KIND` per declared kind with no instance in the whole fused
/// population, dangling edges included.
///
/// Node kinds are every source's declared kinds, qualified; edge kinds are
/// shared by name across the sources and the fuse profile. An edge kind's
/// finding takes the fuse profile's severity and suppression alone, since its
/// sources may disagree. A node kind's follows the orphan rule: the fuse
/// profile's override if it declares one, else its source's, and suppressed
/// where its source would suppress it.
fn empty_kinds(
    report: &FuseReport,
    profile: &FuseProfile,
    composition: &Composition<'_>,
    standard: &Profile,
) -> Vec<Issue> {
    let mut edge_kinds: BTreeSet<String> = profile.edges.keys().cloned().collect();
    for source_profile in composition.source_profiles.values() {
        edge_kinds.extend(source_profile.edge_kinds().keys().cloned());
    }
    let tally = KindTally::of_kinds(
        composition.kind_patterns.keys().cloned(),
        edge_kinds,
        report.nodes.iter().map(|node| node.kind.as_str()),
        report.edges.iter().map(|edge| edge.kind.as_str()),
    );
    let code = FindingCode::EmptyKind;
    let fuse_override = profile
        .validations
        .iter()
        .any(|(name, config)| name == code.as_str() && config.contains_key("severity"));
    tally
        .empty_kinds()
        .map(|(kind, side)| {
            let mut found = issue(code, side.message(kind), Origin::Profile, standard, None);
            // Source names are colon-free, so the first colon ends the qualifier.
            let source_profile = (side == EmptyOf::Node)
                .then(|| kind.split_once(':'))
                .flatten()
                .and_then(|(source, _)| composition.source_profiles.get(source));
            if let Some(source_profile) = source_profile {
                if !fuse_override {
                    let override_ = source_profile.validation_overrides().get(code.as_str());
                    found.severity = resolve_severity(default_severity(code), override_.copied());
                }
                if source_suppresses(&SuppressionIndex::new(source_profile), &found) {
                    found.suppressed = true;
                }
            }
            found
        })
        .collect()
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
    for node in &mut report.nodes {
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
            std::mem::take(&mut node.attrs),
            node.provenance.location.clone(),
        )?;
    }
    for edge in &mut report.edges {
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
                    attrs: std::mem::take(&mut edge.attrs),
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
                stage: None,
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
    // Return the attrs taken above, in insertion order; the edge zip skips the
    // edges the add loop skipped (no `target_kind`).
    let (graph_nodes, graph_edges) = graph.into_nodes_and_edges();
    for (node, taken) in report.nodes.iter_mut().zip(graph_nodes) {
        node.attrs = taken.attrs;
    }
    for (edge, taken) in report
        .edges
        .iter_mut()
        .filter(|edge| edge.target_kind.is_some())
        .zip(graph_edges)
    {
        edge.attrs = taken.attrs;
    }
    // The composed graph holds resolved edges and observed kinds only; the
    // empty kinds are counted below over everything fuse read.
    composed.retain(|issue| issue.code != FindingCode::EmptyKind.as_str());
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
    composed.extend(empty_kinds(report, profile, composition, &standard));
    for node in &report.nodes {
        if composition.reported_id_format.contains(&node.id) {
            continue;
        }
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
            stage: None,
        });
    }
    // A note a source attached reaches every finding of the fuse's own with its
    // code and node, the ones that replace withdrawn source findings included.
    // Fuse's own judgements across sources take none: a duplicate ID's node is
    // raw and may spell another source's qualified ID, and an ambiguity is a
    // question no one source's run asked.
    for finding in &mut report.findings[merged..] {
        if finding.issue.note.is_some()
            || finding.source.is_none()
            || finding.issue.code == CROSS_SOURCE_DUPLICATE_ID
            || finding.issue.code == AMBIGUOUS_CROSS_REF
        {
            continue;
        }
        if let Some(id) = &finding.issue.node_id
            && let Some(note) = composition
                .carried_notes
                .get(&(finding.issue.code.clone(), id.clone()))
        {
            finding.issue.note = Some(note.clone());
        }
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
            stage: None,
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
            stage: None,
        }));
    promote_report(report, strict);
    // The fuse profile's SUPPRESS only adds: a fuse-owned finding arrives
    // suppressed only where its source decided so, and an entry that does not
    // match it (any `node_ids` entry, for a node-less EMPTY_KIND) must not undo
    // that decision.
    let suppressions = SuppressionIndex::new(&standard);
    for finding in &mut report.findings[merged..] {
        if suppressions.decide(&finding.issue.code, finding.issue.node_id.as_deref()) == Some(true)
        {
            finding.issue.suppressed = true;
        }
    }
    rejudge_source_unused(report, composition, merged, strict);
    Ok(())
}

/// Judge again each source orphan-code `SUPPRESS` entry against the composed
/// run's orphan findings on that source's nodes.
///
/// The source judged the entry against its own orphan findings, which
/// [`assemble`] withdrew; the composed graph answers the orphan question
/// instead. An entry unused in the source run may match here, so its source
/// `SUPPRESS_UNUSED` goes or names only the `node_ids` still unmatched. An entry
/// the source run used may match nothing here, as when an edge from another
/// source reaches its node, so the source gets a `SUPPRESS_UNUSED` for it, with
/// the severity and suppression its own profile gives that code.
fn rejudge_source_unused(
    report: &mut FuseReport,
    composition: &Composition<'_>,
    merged: usize,
    strict: bool,
) {
    let mut names: Vec<&String> = composition.source_profiles.keys().collect();
    names.sort();
    let mut used = Vec::new();
    let mut added = Vec::new();
    let (sources, composed) = report.findings.split_at_mut(merged);
    for source in names {
        let profile = &composition.source_profiles[source];
        let withdrawn = composition
            .withdrawn_orphans
            .get(source.as_str())
            .map_or(&[][..], Vec::as_slice);
        let before = || {
            withdrawn
                .iter()
                .map(|(code, id)| (code.as_str(), id.as_deref()))
        };
        let after = || {
            composed
                .iter()
                .filter(|f| {
                    f.source.as_deref() == Some(source.as_str()) && is_orphan_code(&f.issue.code)
                })
                .map(|f| {
                    (
                        f.issue.code.as_str(),
                        f.issue.node_id.as_deref().map(raw_id),
                    )
                })
        };
        for entry in profile
            .suppressions()
            .iter()
            .filter(|entry| is_orphan_code(&entry.code))
        {
            let judged = unused_message(entry, after());
            let Some(was) = unused_message(entry, before()) else {
                if let Some(message) = judged {
                    let mut found = issue(
                        FindingCode::SuppressUnused,
                        message,
                        Origin::Profile,
                        profile,
                        None,
                    );
                    promote(std::iter::once(&mut found), strict);
                    found.message = rendered_unused(found.message, found.severity, profile);
                    found.suppressed = source_suppresses(&SuppressionIndex::new(profile), &found);
                    added.push(FuseFinding {
                        issue: found,
                        source: Some(source.clone()),
                        locations: Vec::new(),
                        stage: None,
                    });
                }
                continue;
            };
            let Some(index) = sources.iter().position(|f| {
                f.issue.code == FindingCode::SuppressUnused.as_str()
                    && f.source.as_deref() == Some(source.as_str())
                    && f.issue.message == rendered_unused(was.clone(), f.issue.severity, profile)
            }) else {
                continue;
            };
            match judged {
                Some(message) => {
                    let severity = sources[index].issue.severity;
                    sources[index].issue.message = rendered_unused(message, severity, profile);
                }
                None => used.push(index),
            }
        }
    }
    used.sort_unstable();
    used.dedup();
    let at = merged - used.len();
    for index in used.into_iter().rev() {
        report.findings.remove(index);
    }
    report.findings.splice(at..at, added);
}

/// Render `message` through the source profile's `SUPPRESS_UNUSED` template at
/// `severity`, as the source run rendered its own finding.
///
/// Reconciliation finds a source's finding by its text and rewrites it, so it
/// must use the text the source printed: the bare message misses every source
/// whose profile declares a template.
fn rendered_unused(message: String, severity: Severity, profile: &Profile) -> String {
    let mut probe = issue(
        FindingCode::SuppressUnused,
        message,
        Origin::Profile,
        profile,
        None,
    );
    probe.severity = severity;
    crate::validate::apply_messages(std::iter::once(&mut probe), profile, &LatticeGraph::new());
    probe.message
}
