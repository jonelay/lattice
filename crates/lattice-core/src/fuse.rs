//! Compose source traces with explicit, source-qualified cross-register joins.

use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::graph::{EdgeSpec, LatticeGraph};
use crate::profile::{Profile, load_profile, load_profile_value};
use crate::types::{
    FuseEdge, FuseFinding, FuseNode, FuseProvenance, FuseReport, Issue, PathwayEntry, Provenance,
    Severity,
};
use crate::validate::{FindingCode, default_severity, validate};

#[derive(Debug, Deserialize)]
pub struct Source {
    pub name: String,
    pub profile: PathBuf,
    pub adapter: PathBuf,
    pub target: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub manifest_version: String,
    pub name: String,
    pub version: String,
    pub fuse_profile: PathBuf,
    pub sources: Vec<Source>,
}

fn load_yaml(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_norway::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn nonempty(value: &str, where_: &str) -> Result<(), String> {
    if value.is_empty() {
        Err(format!("{where_}: expected a non-empty string"))
    } else {
        Ok(())
    }
}

fn relative(base: &Path, path: &mut PathBuf, key: &str) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!("'{key}' must be a non-empty relative path"));
    }
    *path = base.join(&*path);
    Ok(())
}

pub fn load_manifest(path: &Path) -> Result<Manifest, String> {
    let path =
        std::fs::canonicalize(path).map_err(|e| format!("manifest '{}': {e}", path.display()))?;
    let raw = load_yaml(&path)?;
    let mut manifest: Manifest =
        serde_json::from_value(raw).map_err(|e| format!("manifest '{}': {e}", path.display()))?;
    for (key, value) in [
        ("manifest_version", &manifest.manifest_version),
        ("name", &manifest.name),
        ("version", &manifest.version),
    ] {
        nonempty(value, key)?;
    }
    if manifest.sources.is_empty() {
        return Err("'sources' must be a non-empty list".into());
    }
    let base = path.parent().expect("canonical manifest has parent");
    relative(base, &mut manifest.fuse_profile, "fuse_profile")?;
    let mut names = BTreeSet::new();
    for source in &mut manifest.sources {
        nonempty(&source.name, "source name")?;
        if source.name.contains(':') {
            return Err("source names must not contain ':'".into());
        }
        if !names.insert(source.name.clone()) {
            return Err(format!("duplicate source name '{}'", source.name));
        }
        relative(base, &mut source.profile, "profile")?;
        relative(base, &mut source.adapter, "adapter")?;
        relative(base, &mut source.target, "target")?;
    }
    Ok(manifest)
}

#[derive(Debug, Deserialize)]
struct EdgeDeclaration {
    allowed: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct FuseProfile {
    edges: BTreeMap<String, EdgeDeclaration>,
    validations: Vec<(String, Map<String, Value>)>,
}

fn qualified_kind(kind: &str) -> Result<(), String> {
    match kind.split_once(':') {
        Some((source, kind)) if !source.is_empty() && !kind.is_empty() && !kind.contains(':') => {
            Ok(())
        }
        _ => Err(format!(
            "expected a source-qualified kind 'source:kind', got '{kind}'"
        )),
    }
}

pub fn load_fuse_profile(path: &Path) -> Result<FuseProfile, String> {
    let raw = load_yaml(path)?;
    let top = raw
        .as_object()
        .ok_or("fuse profile must be a YAML mapping")?;
    const KNOWN: &[&str] = &[
        "name",
        "profile_version",
        "edge_kinds",
        "node_kinds",
        "validations",
    ];
    for key in top.keys() {
        if !KNOWN.contains(&key.as_str()) {
            return Err(format!("fuse profile: unknown key '{key}'"));
        }
    }
    if top.contains_key("node_kinds") {
        return Err(
            "fuse profile must not declare 'node_kinds'; node kinds belong to source profiles"
                .into(),
        );
    }
    let edges: BTreeMap<String, EdgeDeclaration> = serde_json::from_value(
        top.get("edge_kinds")
            .ok_or("fuse profile requires 'edge_kinds'")?
            .clone(),
    )
    .map_err(|e| format!("fuse profile edge_kinds: {e}"))?;
    for (name, edge) in &edges {
        nonempty(name, "edge kind")?;
        for (src, tgt) in &edge.allowed {
            qualified_kind(src)?;
            qualified_kind(tgt)?;
        }
    }
    let mut validations = Vec::new();
    if let Some(raw) = top.get("validations").filter(|v| !v.is_null()) {
        for entry in raw.as_array().ok_or("'validations' must be a list")? {
            let entry = entry
                .as_object()
                .filter(|e| e.len() == 1)
                .ok_or("expected one validation code mapping")?;
            let (code, config) = entry.iter().next().expect("one entry");
            let config = config
                .as_object()
                .ok_or("validation config must be a mapping")?;
            if let Some(severity) = config.get("severity").filter(|v| !v.is_null()) {
                severity
                    .as_str()
                    .and_then(Severity::parse)
                    .ok_or("invalid validation severity")?;
            }
            if code == "COVERAGE" {
                const COVERAGE_KEYS: &[&str] = &["severity", "target_kind", "edge_kind", "where"];
                for key in ["target_kind", "edge_kind"] {
                    nonempty(
                        config
                            .get(key)
                            .and_then(Value::as_str)
                            .ok_or_else(|| format!("COVERAGE requires '{key}'"))?,
                        key,
                    )?;
                }
                qualified_kind(config["target_kind"].as_str().expect("checked string"))?;
                for key in config.keys() {
                    if !COVERAGE_KEYS.contains(&key.as_str()) {
                        return Err(format!("COVERAGE: unknown config key '{key}'"));
                    }
                }
                if let Some(where_val) = config.get("where").filter(|v| !v.is_null()) {
                    let norway_val = serde_norway::to_value(where_val)
                        .map_err(|e| format!("COVERAGE where: {e}"))?;
                    crate::profile::parse_condition_block(Some(&norway_val), "COVERAGE", "where")
                        .map_err(|e| format!("fuse profile: {e}"))?;
                }
            }
            let mut config = config.clone();
            config.retain(|_, v| !v.is_null());
            validations.push((code.clone(), config));
        }
    }
    Ok(FuseProfile { edges, validations })
}

impl FuseProfile {
    fn severity(&self, code: &str, default: Severity) -> Severity {
        self.validations
            .iter()
            .filter(|(c, _)| c == code)
            .filter_map(|(_, config)| {
                config
                    .get("severity")
                    .and_then(Value::as_str)
                    .and_then(Severity::parse)
            })
            .next_back()
            .map(|s| {
                if default == Severity::Hint {
                    default
                } else {
                    s
                }
            })
            .unwrap_or(default)
    }
}

#[derive(Debug, Deserialize)]
struct TraceLocation {
    file: String,
    line: i64,
}
impl TraceLocation {
    fn attributed(self, source: &str) -> FuseProvenance {
        FuseProvenance {
            source: source.into(),
            location: Provenance::new(self.file, self.line),
        }
    }
}
#[derive(Debug, Deserialize)]
struct TraceFinding {
    code: String,
    severity: String,
    message: String,
    file: String,
    line: i64,
    node_id: Option<String>,
    state: Option<String>,
}
impl TraceFinding {
    fn attributed(self, source: &str, node: Option<&str>) -> FuseFinding {
        let mut issue = Issue::new(
            Severity::parse(&self.severity).expect("validated severity"),
            self.code,
            self.message,
            Provenance::new(self.file, self.line),
            self.node_id
                .as_deref()
                .or(node)
                .map(|id| qualify(source, id)),
        );
        issue.state = self.state;
        FuseFinding {
            issue,
            source: Some(source.into()),
            locations: Vec::new(),
        }
    }
}
#[derive(Debug, Deserialize)]
struct TraceEdge {
    tgt: String,
    kind: String,
    #[serde(default)]
    attrs: Map<String, Value>,
    provenance: TraceLocation,
}
#[derive(Debug, Deserialize)]
struct TraceNode {
    id: String,
    kind: String,
    #[serde(default)]
    attrs: Map<String, Value>,
    provenance: TraceLocation,
    #[serde(default)]
    edges: Vec<TraceEdge>,
    #[serde(default)]
    findings: Vec<TraceFinding>,
}
#[derive(Debug, Deserialize)]
struct TracePathway {
    name: String,
    order: Vec<String>,
    current: String,
}
#[derive(Debug, Deserialize)]
pub struct SourceTrace {
    header: BTreeMap<String, String>,
    entries: Vec<TraceNode>,
    #[serde(default)]
    unattachable_findings: Vec<TraceFinding>,
    #[serde(default)]
    pathways: Vec<TracePathway>,
}

pub fn parse_trace(text: &str, source: &str) -> Result<SourceTrace, String> {
    let check = || -> Result<SourceTrace, String> {
        let trace: SourceTrace = serde_json::from_str(text).map_err(|e| e.to_string())?;
        match trace.header.get("trace_version").map(String::as_str) {
            Some("1" | "2") => (),
            _ => return Err("unsupported or missing trace_version".into()),
        }
        let mut ids = BTreeSet::new();
        for node in &trace.entries {
            nonempty(&node.id, "node id")?;
            nonempty(&node.kind, "node kind")?;
            if !ids.insert(&node.id) {
                return Err(format!("repeated trace entry '{}'", node.id));
            }
            for edge in &node.edges {
                nonempty(&edge.tgt, "edge target")?;
                nonempty(&edge.kind, "edge kind")?;
            }
        }
        for finding in trace
            .entries
            .iter()
            .flat_map(|n| &n.findings)
            .chain(&trace.unattachable_findings)
        {
            if Severity::parse(&finding.severity).is_none() {
                return Err(format!("invalid finding severity '{}'", finding.severity));
            }
        }
        let mut graph = LatticeGraph::new();
        for pathway in &trace.pathways {
            nonempty(&pathway.name, "pathway name")?;
            if graph.pathway(&pathway.name).is_some() {
                return Err(format!("repeated pathway '{}'", pathway.name));
            }
            graph
                .set_pathway(&pathway.name, pathway.order.clone(), &pathway.current)
                .map_err(|e| e.to_string())?;
        }
        Ok(trace)
    };
    check().map_err(|e| format!("source '{source}' emitted invalid trace JSON: {e}"))
}

fn qualify(source: &str, raw: &str) -> String {
    format!("{source}:{raw}")
}
fn strip_anchors(pattern: &str) -> String {
    let s = pattern
        .strip_prefix(r"\A")
        .or_else(|| pattern.strip_prefix('^'))
        .unwrap_or(pattern);
    let s = s
        .strip_suffix(r"\z")
        .or_else(|| s.strip_suffix('$'))
        .unwrap_or(s);
    s.to_owned()
}
fn empty_report(manifest: &Manifest) -> FuseReport {
    FuseReport {
        header: BTreeMap::from([
            ("name".into(), manifest.name.clone()),
            ("version".into(), manifest.version.clone()),
            ("manifest_version".into(), manifest.manifest_version.clone()),
        ]),
        could_run: true,
        ..Default::default()
    }
}
fn promote(report: &mut FuseReport, strict: bool) {
    if strict {
        for finding in &mut report.findings {
            if finding.issue.severity == Severity::Warning {
                finding.issue.severity = Severity::Error;
            }
        }
    }
}
fn source_findings(trace: SourceTrace, source: &str) -> Vec<FuseFinding> {
    trace
        .entries
        .into_iter()
        .flat_map(|node| {
            node.findings
                .into_iter()
                .map(move |f| f.attributed(source, Some(&node.id)))
        })
        .chain(
            trace
                .unattachable_findings
                .into_iter()
                .map(|f| f.attributed(source, None)),
        )
        .collect()
}

/// Run the same lattice executable, accepting trace exit 1 as usable evidence.
/// Source failures are collected across every source, but never yield a partial graph.
pub fn fuse(path: &Path, binary: &Path, strict: bool) -> Result<FuseReport, String> {
    let manifest = load_manifest(path)?;
    let profile = load_fuse_profile(&manifest.fuse_profile)?;
    let mut runs = Vec::new();
    let mut failures = Vec::new();
    for source in &manifest.sources {
        let result = Command::new(binary)
            .args(["trace", "--format", "json", "--profile"])
            .arg(&source.profile)
            .arg("--adapter")
            .arg(&source.adapter)
            .arg("--target")
            .arg(&source.target)
            .output()
            .map_err(|e| e.to_string())
            .and_then(|output| {
                if !matches!(output.status.code(), Some(0 | 1)) {
                    return Err(format!(
                        "trace exited {}: {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                let text = std::str::from_utf8(&output.stdout).map_err(|e| e.to_string())?;
                parse_trace(text, &source.name)
            });
        match result {
            Ok(trace) => runs.push(Some(trace)),
            Err(message) => {
                failures.push(FuseFinding {
                    issue: Issue::new(
                        Severity::Error,
                        "SOURCE_FAILURE",
                        message,
                        Provenance::new("<source>", 0),
                        None,
                    ),
                    source: Some(source.name.clone()),
                    locations: Vec::new(),
                });
                runs.push(None);
            }
        }
    }
    if !failures.is_empty() {
        let mut report = empty_report(&manifest);
        report.could_run = false;
        for (source, trace) in manifest.sources.iter().zip(runs) {
            if let Some(trace) = trace {
                report.findings.extend(source_findings(trace, &source.name));
            }
        }
        report.findings.extend(failures);
        promote(&mut report, strict);
        return Ok(report);
    }
    assemble(
        &manifest,
        &profile,
        runs.into_iter().flatten().collect(),
        strict,
    )
}

pub fn assemble(
    manifest: &Manifest,
    profile: &FuseProfile,
    traces: Vec<SourceTrace>,
    strict: bool,
) -> Result<FuseReport, String> {
    if traces.len() != manifest.sources.len() {
        return Err("expected one trace per manifest source".into());
    }
    let mut kind_patterns = HashMap::new();
    let mut profile_warnings = Vec::new();
    for source in &manifest.sources {
        match load_profile(&source.profile) {
            Ok(source_profile) => {
                kind_patterns.extend(source_profile.node_kinds().iter().map(
                    |(kind, node_kind)| {
                        (
                            qualify(&source.name, kind),
                            strip_anchors(&node_kind.id_pattern_source),
                        )
                    },
                ));
            }
            Err(e) => {
                profile_warnings.push(FuseFinding {
                    issue: Issue::new(
                        Severity::Warning,
                        "CONFIG_ERROR",
                        format!(
                            "could not load source profile '{}': {e} — composed IDs for source '{}' will not be validated against their declared pattern",
                            source.profile.display(),
                            source.name,
                        ),
                        Provenance::new("<fuse-profile>", 0),
                        None,
                    ),
                    source: Some(source.name.clone()),
                    locations: Vec::new(),
                });
            }
        }
    }
    let mut report = empty_report(manifest);
    report.findings.extend(profile_warnings);
    let mut occurrences: HashMap<String, Vec<usize>> = HashMap::new();
    let mut raw_order = Vec::new();
    let mut raw_targets = Vec::new();
    for (source, trace) in manifest.sources.iter().zip(traces) {
        for node in trace.entries {
            let id = qualify(&source.name, &node.id);
            let kind = qualify(&source.name, &node.kind);
            let locations = occurrences.entry(node.id.clone()).or_insert_with(|| {
                raw_order.push(node.id.clone());
                Vec::new()
            });
            locations.push(report.nodes.len());
            for edge in node.edges {
                raw_targets.push(edge.tgt.clone());
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
            }
            report.findings.extend(
                node.findings
                    .into_iter()
                    .map(|f| f.attributed(&source.name, Some(&node.id))),
            );
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
    for raw in raw_order {
        let indices = &occurrences[&raw];
        if indices.len() < 2 {
            continue;
        }
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
                profile.severity("CROSS_SOURCE_DUPLICATE_ID", Severity::Error),
                "CROSS_SOURCE_DUPLICATE_ID",
                format!("node ID '{raw}' occurs in sources {sources}"),
                locations[0].location.clone(),
                Some(raw),
            ),
            source: None,
            locations,
        });
    }
    for (edge, raw) in report.edges.iter_mut().zip(raw_targets) {
        let declaration = profile.edges.get(&edge.kind);
        let matches: Vec<_> = occurrences
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
            .collect();
        if let [target] = matches.as_slice() {
            edge.tgt = target.id.clone();
            edge.target_kind = Some(target.kind.clone());
            edge.target_source = Some(target.provenance.source.clone());
        } else if declaration.is_some() {
            let (code, message, default) = if matches.is_empty() {
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
                    "AMBIGUOUS_CROSS_REF",
                    format!(
                        "edge '{}'->'{raw}' (kind '{}') matches {} allowed targets",
                        edge.src,
                        edge.kind,
                        matches.len()
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
                locations: matches.iter().map(|n| n.provenance.clone()).collect(),
            });
        }
    }
    validate_composed(&mut report, profile, &kind_patterns)?;
    promote(&mut report, strict);
    Ok(report)
}

fn compose_profile(document: &Value) -> Result<Profile, String> {
    let norway =
        serde_norway::to_value(document).map_err(|e| format!("fuse profile conversion: {e}"))?;
    load_profile_value(norway, "fuse").map_err(|e| format!("fuse profile: {e}"))
}

fn validate_composed(
    report: &mut FuseReport,
    profile: &FuseProfile,
    kind_patterns: &HashMap<String, String>,
) -> Result<(), String> {
    let mut graph = LatticeGraph::new();
    let mut nodes = Map::new();
    let mut edges: BTreeMap<String, Vec<(String, String)>> = profile
        .edges
        .iter()
        .map(|(name, edge)| (name.clone(), edge.allowed.clone()))
        .collect();
    for node in &report.nodes {
        let id_pattern = kind_patterns
            .get(&node.kind)
            .and_then(|pattern| {
                node.kind
                    .split_once(':')
                    .map(|(source, _)| format!("{}:(?:{})", regex::escape(source), pattern))
            })
            .unwrap_or_else(|| ".*".into());
        nodes.insert(
            node.kind.clone(),
            json!({"id_pattern": id_pattern, "orphan_ok": true}),
        );
        graph
            .add_node(
                &node.id,
                &node.kind,
                node.attrs.clone(),
                node.provenance.location.clone(),
            )
            .map_err(|e| e.to_string())?;
    }
    for edge in &report.edges {
        if let Some(target) = &edge.target_kind {
            let pairs = edges.entry(edge.kind.clone()).or_default();
            let pair = (edge.source_kind.clone(), target.clone());
            if !pairs.contains(&pair) {
                pairs.push(pair);
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
        graph
            .set_pathway(&pathway.name, pathway.order.clone(), &pathway.current)
            .map_err(|e| e.to_string())?;
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
                    Provenance::new("<fuse-profile>", 0),
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
    for issue in validate(&graph, &standard, false) {
        let source = issue
            .node_id
            .as_ref()
            .and_then(|id| report.nodes.iter().find(|n| &n.id == id))
            .map(|n| n.provenance.source.clone());
        report.findings.push(FuseFinding {
            issue,
            source,
            locations: Vec::new(),
        });
    }
    Ok(())
}
