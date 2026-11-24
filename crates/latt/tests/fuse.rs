use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use latt_core::fuse::{assemble, load_fuse_profile, load_manifest, parse_trace};
use latt_core::output::{Format, output_result};
use latt_core::types::Origin;
use serde_json::{Value, json};

#[cfg(unix)]
#[path = "common/stub.rs"]
mod stub;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "lattice-fuse-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn manifest(dir: &Scratch) -> PathBuf {
    dir.write("fuse.yaml", "manifest_version: '1.0.0'\nname: sample\nversion: '2.0.0'\nfuse_profile: profile.yaml\nsources:\n  - {name: z, profile: z.yaml, adapter: z, target: .}\n  - {name: a, profile: a.yaml, adapter: a, target: .}\n")
}
fn entry(id: &str, edges: Value) -> Value {
    json!({"id": id, "kind": "item", "attrs": {"title": id}, "provenance": {"file": "input", "line": 3}, "edges": edges, "findings": []})
}
fn edge(target: &str) -> Value {
    json!({"tgt": target, "kind": "links", "attrs": {"weight": 2}, "provenance": {"file": "edges", "line": 7}})
}
fn trace(entries: Value) -> Value {
    json!({"header": {"trace_version": "2"}, "entries": entries, "unattachable_findings": [], "pathways": [{"name": "phase", "order": ["first", "last"], "current": "first"}]})
}
fn report(
    dir: &Scratch,
    z: Value,
    a: Value,
    profile: &str,
    strict: bool,
) -> latt_core::types::FuseReport {
    for name in ["z.yaml", "a.yaml"] {
        if !dir.0.join(name).exists() {
            dir.write(name, SOURCE_PROFILE);
        }
    }
    let manifest = load_manifest(&manifest(dir)).unwrap();
    let profile = load_fuse_profile(&dir.write("profile.yaml", profile)).unwrap();
    assemble(
        &manifest,
        &profile,
        vec![
            parse_trace(&z.to_string(), "z").unwrap(),
            parse_trace(&a.to_string(), "a").unwrap(),
        ],
        strict,
    )
    .unwrap()
}
/// A source profile that loads and constrains no composed ID, for a test
/// about something else.
const SOURCE_PROFILE: &str = "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}}\nedge_kinds: {}\n";
const PROFILE: &str = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links, severity: hint}\n";

// Requirement: program-composition/Read a fuse manifest
// Requirement: versioning/Wire version axes
#[test]
fn manifest_order_paths_and_invalid_inputs() {
    let dir = Scratch::new();
    let path = manifest(&dir);
    let loaded = load_manifest(&path).unwrap();
    assert_eq!(
        loaded
            .sources
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    assert_eq!(loaded.sources[0].adapter, dir.0.join("z"));
    let original = fs::read_to_string(&path).unwrap();
    // A compiled-in adapter name passes through; a path spelling of it is a file.
    let named = dir.write(
        "named.yaml",
        &original
            .replace("adapter: z", "adapter: md")
            .replace("adapter: a", "adapter: ./md"),
    );
    let named = load_manifest(&named).unwrap();
    #[cfg(feature = "native-adapters")]
    assert_eq!(named.sources[0].adapter, PathBuf::from("md"));
    #[cfg(not(feature = "native-adapters"))]
    assert_eq!(named.sources[0].adapter, dir.0.join("md"));
    assert_eq!(named.sources[1].adapter, dir.0.join("./md"));
    let minor = dir.write(
        "minor.yaml",
        &original.replace("manifest_version: '1.0.0'", "manifest_version: '1.4.2'"),
    );
    assert_eq!(load_manifest(&minor).unwrap().manifest_version, "1.4.2");
    for (invalid, reason) in [
        (
            original.replace("name: a", "name: z"),
            "duplicate source name 'z'",
        ),
        (
            original.replace("name: z", "name: z:q"),
            "source names must not contain ':'",
        ),
        (
            original.replace("profile: z.yaml", "profile: /absolute"),
            "'profile' must be a non-empty relative path",
        ),
        (
            original.replace("version: '2.0.0'", "version: 2"),
            "invalid type: integer `2`, expected a string",
        ),
        (
            original.replace("fuse_profile:", "program_profile:"),
            "unknown field `program_profile`",
        ),
        ("[]".into(), "invalid length 0"),
        (
            original.replace("manifest_version: '1.0.0'", "manifest_version: '2.0.0'"),
            "unsupported manifest version 2.0.0 (supported major: 1)",
        ),
        (
            original.replace("manifest_version: '1.0.0'", "manifest_version: '1'"),
            "manifest_version '1' is not a valid version",
        ),
        (
            original.replace("fuse_profile:", "notes: x\nfuse_profile:"),
            "unknown field `notes`",
        ),
        (
            original.replace("target: .}", "target: ., native: md}"),
            "unknown field `native`",
        ),
    ] {
        fs::write(&path, invalid).unwrap();
        let error = load_manifest(&path).unwrap_err().to_string();
        assert!(error.contains(reason), "{reason}: {error}");
    }
}

// Requirement: program-composition/Source-qualified merge with colon separator
// Requirement: program-composition/Pathway preservation
// Requirement: program-composition/Tri-format output
#[test]
fn assembly_qualifies_ids_kinds_edges_pathways_and_preserves_attributes() {
    let dir = Scratch::new();
    let report = report(
        &dir,
        trace(json!([entry("Z", json!([edge("A")]))])),
        trace(json!([entry("A", json!([]))])),
        PROFILE,
        false,
    );
    assert_eq!(report.exit_code(), 0);
    let json: Value = serde_json::from_str(&output_result(&report, Format::Json)).unwrap();
    assert_eq!(json["header"]["name"], "sample");
    assert_eq!(json["header"]["fuse_version"], "1");
    assert_eq!(json["nodes"][0]["id"], "z:Z");
    assert_eq!(json["nodes"][1]["kind"], "a:item");
    assert_eq!(json["edges"][0]["src"], "z:Z");
    assert_eq!(json["edges"][0]["tgt"], "a:A");
    assert_eq!(json["edges"][0]["attrs"]["weight"], 2);
    assert_eq!(json["edges"][0]["provenance"]["source"], "z");
    assert_eq!(json["pathways"][0]["name"], "z:phase");
    assert_eq!(json["pathways"][1]["name"], "a:phase");
    assert_eq!(json["pathways"][0]["order"], json!(["first", "last"]));
    for format in [Format::Plain, Format::Rich] {
        let text = output_result(&report, format);
        assert!(text.contains("z:Z"));
        assert!(text.contains("a:A"));
        assert!(text.contains("z:phase"));
    }
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn composed_ids_are_validated_against_source_profile_patterns() {
    let dir = Scratch::new();
    let source_profile = "name: local\nprofile_version: '1.0.0'\nnode_kinds:\n  item: {id_pattern: '^[A-Z]+-[0-9]+$', orphan_ok: true}\nedge_kinds: {}\n";
    dir.write("z.yaml", source_profile);
    dir.write("a.yaml", source_profile);
    let report = report(
        &dir,
        trace(json!([entry("malformed", json!([]))])),
        trace(json!([entry("REQ-001", json!([]))])),
        "edge_kinds: {}",
        false,
    );

    let id_format_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.issue.code == "ID_FORMAT")
        .collect();
    assert_eq!(id_format_findings.len(), 1);
    assert_eq!(
        id_format_findings[0].issue.node_id.as_deref(),
        Some("z:malformed")
    );
    assert_eq!(id_format_findings[0].source.as_deref(), Some("z"));
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.issue.node_id.as_deref() != Some("a:REQ-001"))
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn a_source_pattern_keeps_its_meaning_under_composition() {
    // A pattern valid for the source must stay valid for the composed ID:
    // nested anchors and an escaped `$` are not prefix-splicable.
    for (pattern, id) in [("(?:^A$)", "A"), (r"A\$", "A$"), (r"\AB\z", "B")] {
        let dir = Scratch::new();
        let source_profile = format!(
            "name: local\nprofile_version: '1.0.0'\nnode_kinds:\n  item: {{id_pattern: '{pattern}', orphan_ok: true}}\nedge_kinds: {{}}\n"
        );
        dir.write("z.yaml", &source_profile);
        dir.write("a.yaml", &source_profile);
        let report = report(
            &dir,
            trace(json!([entry(id, json!([]))])),
            trace(json!([entry("nope", json!([]))])),
            "edge_kinds: {}",
            false,
        );
        assert!(report.could_run, "{pattern}: {:?}", report.findings);
        let ids: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.issue.code == "ID_FORMAT")
            .map(|f| f.issue.node_id.clone().unwrap())
            .collect();
        assert_eq!(ids, ["a:nope"], "{pattern}: {:?}", report.findings);
    }
}

// Requirement: program-composition/Cross-source edge resolution via allowed pairings
// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn duplicates_ambiguity_dangling_coverage_and_strict() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item], [z:item, z:item]]\nvalidations:\n  - CROSS_SOURCE_DUPLICATE_ID: {severity: warning}\n  - AMBIGUOUS_CROSS_REF: {severity: hint}\n  - VACANCY: {severity: warning}\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links, severity: hint}\n";
    let z = trace(json!([entry("X", json!([edge("X"), edge("absent")]))]));
    let a = trace(json!([entry("X", json!([]))]));
    let normal = report(&dir, z.clone(), a.clone(), profile, false);
    assert_eq!(normal.exit_code(), 0);
    let value: Value = serde_json::from_str(&output_result(&normal, Format::Json)).unwrap();
    let findings = value["findings"].as_array().unwrap();
    for code in [
        "CROSS_SOURCE_DUPLICATE_ID",
        "AMBIGUOUS_CROSS_REF",
        "VACANCY",
        "COVERAGE",
    ] {
        assert!(findings.iter().any(|f| f["code"] == code), "{code}");
    }
    let duplicate = findings
        .iter()
        .find(|f| f["code"] == "CROSS_SOURCE_DUPLICATE_ID")
        .unwrap();
    assert_eq!(duplicate["sources"], json!(["z", "a"]));
    assert_eq!(duplicate["locations"].as_array().unwrap().len(), 2);
    let strict = report(&dir, z, a, profile, true);
    assert_eq!(strict.exit_code(), 1);
    assert!(
        strict
            .findings
            .iter()
            .filter(|f| f.issue.code == "COVERAGE" || f.issue.code == "AMBIGUOUS_CROSS_REF")
            .all(|f| f.issue.severity == latt_core::types::Severity::Hint)
    );
    let coverage = normal
        .findings
        .iter()
        .find(|f| f.issue.code == "COVERAGE")
        .unwrap();
    assert_eq!(
        coverage.issue.state.as_deref(),
        Some("unknown"),
        "unresolved edges leave coverage state unknown"
    );
    assert!(
        normal
            .findings
            .iter()
            .any(|f| f.issue.code == "COVERAGE_UNKNOWN"),
        "unattributed sources produce COVERAGE_UNKNOWN"
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn source_findings_retain_attribution_and_are_promoted_after_collection() {
    let dir = Scratch::new();
    let sp = "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}}\nedge_kinds: {}\n";
    dir.write("z.yaml", sp);
    dir.write("a.yaml", sp);
    let mut z = trace(json!([entry("Z", json!([]))]));
    let finding = json!({"code": "LOCAL", "severity": "warning", "message": "check", "file": "x", "line": 1, "state": "unknown"});
    z["entries"][0]["findings"] = json!([finding]);
    z["unattachable_findings"] = json!([finding]);
    let report = report(&dir, z, trace(json!([])), "edge_kinds: {}", true);
    assert_eq!(report.exit_code(), 1);
    // Source `a` declares `item` and has none: the fuse's own finding.
    let messages: Vec<&str> = report
        .findings
        .iter()
        .map(|f| f.issue.message.as_str())
        .collect();
    assert_eq!(
        messages,
        ["check", "check", "node kind 'a:item' has no node"]
    );
    assert_eq!(report.findings[0].source.as_deref(), Some("z"));
    assert_eq!(report.findings[0].issue.node_id.as_deref(), Some("z:Z"));
    assert_eq!(report.findings[1].issue.node_id, None);
}

// Requirement: program-composition/A source profile that cannot load is a source failure
#[test]
fn source_profile_that_cannot_load_is_a_source_failure() {
    let dir = Scratch::new();
    dir.write("a.yaml", SOURCE_PROFILE);
    let manifest = load_manifest(&manifest(&dir)).unwrap();
    let profile = load_fuse_profile(&dir.write("profile.yaml", "edge_kinds: {}")).unwrap();
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["unattachable_findings"] = json!([{"code": "LOCAL", "severity": "warning", "message": "z's own", "file": "x", "line": 1}]);
    let mut a = trace(json!([entry("A", json!([]))]));
    a["unattachable_findings"] = json!([{"code": "LOCAL", "severity": "warning", "message": "a's own", "file": "x", "line": 1}]);
    let report = assemble(
        &manifest,
        &profile,
        vec![
            parse_trace(&z.to_string(), "z").unwrap(),
            parse_trace(&a.to_string(), "a").unwrap(),
        ],
        false,
    )
    .unwrap();

    assert!(!report.could_run);
    assert_eq!(report.exit_code(), 2);
    assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
    let failure = report
        .findings
        .iter()
        .find(|f| f.issue.code == "SOURCE_FAILURE")
        .expect("a SOURCE_FAILURE");
    assert_eq!(failure.source.as_deref(), Some("z"));
    assert_eq!(
        failure.stage,
        Some(latt_core::types::FuseStage::SourceProfile)
    );
    let json: Value = serde_json::from_str(&output_result(&report, Format::Json)).unwrap();
    let staged: Vec<_> = json["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["code"].clone(), f.get("stage").cloned()))
        .collect();
    assert!(
        staged.contains(&(json!("SOURCE_FAILURE"), Some(json!("source_profile"))))
            && staged.contains(&(json!("LOCAL"), None)),
        "{staged:?}"
    );
    assert_eq!(failure.issue.severity, latt_core::types::Severity::Error);
    assert!(
        failure.issue.message.contains("z.yaml"),
        "{}",
        failure.issue.message
    );
    // The failed source contributes nothing; its trace is not composed.
    assert_eq!(report.findings.len(), 2, "{:?}", report.findings);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.issue.code == "LOCAL" && f.source.as_deref() == Some("a"))
    );
}

// Requirement: program-composition/Tri-format output
#[test]
fn fuse_errors_past_loading_are_staged_by_their_cause() {
    let dir = Scratch::new();
    for name in ["z.yaml", "a.yaml"] {
        dir.write(name, SOURCE_PROFILE);
    }
    let path = manifest(&dir);
    let manifest = load_manifest(&path).unwrap();
    let profile = load_fuse_profile(&dir.write(
        "profile.yaml",
        "edge_kinds: {}\nvalidations:\n  - VACANCY: {bogus: 1}\n",
    ))
    .unwrap();
    let z = trace(json!([entry("Z", json!([]))]));
    let a = trace(json!([entry("A", json!([]))]));
    let error = assemble(
        &manifest,
        &profile,
        vec![
            parse_trace(&z.to_string(), "z").unwrap(),
            parse_trace(&a.to_string(), "a").unwrap(),
        ],
        false,
    )
    .unwrap_err();
    use latt_core::types::FuseStage;
    // A fuse profile entry only the composed profile's loader refuses.
    assert!(
        matches!(error, latt_core::fuse::FuseError::Profile(_)),
        "{error:?}"
    );
    assert_eq!(error.stage(&path), FuseStage::FuseProfile);
    // A trace that cannot be read: `source_output`, as `fuse` stages it.
    let unreadable = parse_trace("not json", "z").unwrap_err();
    assert_eq!(unreadable.stage(&path), FuseStage::SourceOutput);
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn coverage_with_undeclared_edge_kind_produces_config_error() {
    let dir = Scratch::new();
    let profile = "edge_kinds: {}\nvalidations:\n  - COVERAGE:\n      target_kind: z:typo\n      edge_kind: link\n      severity: hint\n";
    let z = trace(json!([entry("Z", json!([]))]));
    let a = trace(json!([entry("A", json!([]))]));
    let result = report(&dir, z, a, profile, false);
    assert_eq!(result.exit_code(), 1);
    assert!(result.findings.iter().all(|f| f.issue.code != "COVERAGE"));
    assert!(
        result
            .findings
            .iter()
            .any(|f| f.issue.code == "CONFIG_ERROR")
    );
}

// Requirement: validation/Built-in validators
#[test]
fn one_bad_fuse_coverage_entry_does_not_suppress_the_next() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n\
                   \x20 - COVERAGE: {target_kind: 'z:typo', edge_kind: links}\n\
                   \x20 - COVERAGE: {target_kind: 'a:item', edge_kind: links}\n";
    let z = trace(json!([entry("Z", json!([]))]));
    let a = trace(json!([entry("A", json!([]))]));
    let result = report(&dir, z, a, profile, false);
    assert_eq!(result.exit_code(), 1);
    let config_error = result
        .findings
        .iter()
        .find(|f| f.issue.code == "CONFIG_ERROR")
        .expect("a CONFIG_ERROR");
    assert!(
        config_error.issue.message.contains("z:typo"),
        "{}",
        config_error.issue.message
    );
    assert!(
        result
            .findings
            .iter()
            .any(|f| f.issue.code == "COVERAGE" && f.issue.node_id.as_deref() == Some("a:A")),
        "{:?}",
        result.findings
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn coverage_undeclared_edge_kind_observed_in_edges_produces_config_error() {
    let dir = Scratch::new();
    let profile = "edge_kinds: {}\nvalidations:\n  - COVERAGE: {target_kind: 'z:item', edge_kind: links, severity: hint}\n";
    let mut z_entry = entry("Z", json!([]));
    z_entry["edges"] =
        json!([{"tgt": "Y", "kind": "links", "attrs": {}, "provenance": {"file": "e", "line": 1}}]);
    let z = trace(json!([z_entry, entry("Y", json!([]))]));
    let a = trace(json!([]));
    let result = report(&dir, z, a, profile, false);
    assert!(
        result
            .findings
            .iter()
            .any(|f| f.issue.code == "CONFIG_ERROR"),
        "undeclared edge_kind observed in resolved edges must produce CONFIG_ERROR"
    );
    assert!(
        result.findings.iter().all(|f| f.issue.code != "COVERAGE"),
        "excluded entry must not produce COVERAGE"
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn coverage_repeated_entries_keep_their_own_severity() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links, severity: error}\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links, severity: hint}\n";
    let z = trace(json!([entry("Z", json!([]))]));
    let a = trace(json!([entry("A", json!([]))]));
    let result = report(&dir, z, a, profile, false);
    assert_eq!(result.exit_code(), 1);
    let mut severities: Vec<String> = result
        .findings
        .iter()
        .filter(|f| f.issue.code == "COVERAGE")
        .map(|f| format!("{:?}", f.issue.severity))
        .collect();
    severities.sort();
    assert_eq!(
        severities,
        ["Error", "Hint"],
        "one finding per entry, each at its own severity"
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn coverage_unknown_state_and_hint_through_fuse() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE: {target_kind: 'a:item', edge_kind: links}\n";
    let z = trace(json!([entry("Z", json!([]))]));
    let a = trace(json!([entry("A", json!([]))]));
    let result = report(&dir, z, a, profile, false);
    let coverage = result
        .findings
        .iter()
        .find(|f| f.issue.code == "COVERAGE")
        .expect("COVERAGE finding");
    assert_eq!(coverage.issue.state.as_deref(), Some("unknown"));
    assert!(
        result
            .findings
            .iter()
            .any(|f| f.issue.code == "COVERAGE_UNKNOWN")
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn coverage_where_filters_target_nodes() {
    let dir = Scratch::new();
    let profile_no_where = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE:\n      target_kind: 'a:item'\n      edge_kind: links\n";
    let profile_with_where = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE:\n      target_kind: 'a:item'\n      edge_kind: links\n      where: {title: {eq: match}}\n";
    let z = trace(json!([entry("Z", json!([]))]));
    let mut a_entry = entry("A", json!([]));
    a_entry["attrs"]["title"] = json!("other");
    let a = trace(json!([a_entry]));
    let without = report(&dir, z.clone(), a.clone(), profile_no_where, false);
    assert!(
        without.findings.iter().any(|f| f.issue.code == "COVERAGE"),
        "without where, COVERAGE fires"
    );
    let with = report(&dir, z, a, profile_with_where, false);
    assert!(
        with.findings.iter().all(|f| f.issue.code != "COVERAGE"),
        "where filter should suppress COVERAGE for non-matching node"
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn coverage_where_on_the_source_side_excludes_evidence() {
    let dir = Scratch::new();
    let base = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE:\n      target_kind: 'a:item'\n      edge_kind: links\n";
    let excluding = format!("{base}      source_where: {{title: {{ne: Z}}}}\n");
    let z = trace(json!([entry("Z", json!([edge("A")]))]));
    let a = trace(json!([entry("A", json!([]))]));
    let covered = report(&dir, z.clone(), a.clone(), base, false);
    assert!(covered.findings.iter().all(|f| f.issue.code != "COVERAGE"));
    let excluded = report(&dir, z, a, &excluding, false);
    assert!(
        excluded.findings.iter().any(|f| f.issue.code == "COVERAGE"),
        "an edge from a source source_where excludes is not coverage"
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn coverage_where_null_is_treated_as_absent() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE:\n      target_kind: 'a:item'\n      edge_kind: links\n      where: null\n";
    let z = trace(json!([entry("Z", json!([]))]));
    let a = trace(json!([entry("A", json!([]))]));
    let result = report(&dir, z, a, profile, false);
    assert!(
        result.findings.iter().any(|f| f.issue.code == "COVERAGE"),
        "where: null should behave like absent where"
    );
}

// Requirement: program-composition/Read a fuse profile
// Requirement: program-composition/Run each source through latt trace
#[test]
fn rejects_bad_profiles_and_traces() {
    let dir = Scratch::new();
    for (invalid, reason) in [
        (
            "node_kinds: {}\nedge_kinds: {}",
            "must not declare 'node_kinds'",
        ),
        ("edge_kinds: []", "edge_kinds: invalid type: sequence"),
        (
            "edge_kinds: {links: {allowed: [[z/item, a/item]]}}",
            "expected a source-qualified kind 'source:kind', got 'z/item'",
        ),
        (
            "edge_kinds: {}\nvalidations: [{COVERAGE: {target_kind: 'a:item'}}]",
            "COVERAGE requires 'edge_kind'",
        ),
        (
            "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE:\n      target_kind: 'a:item'\n      edge_kind: links\n      where: {title: {badop: 1}}\n",
            "unknown operator 'badop'",
        ),
        (
            "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  - COVERAGE:\n      target_kind: 'a:item'\n      edge_kind: links\n      source_where: {title: {badop: 1}}\n",
            "unknown operator 'badop'",
        ),
        (
            "edge_kinds: {}\nvalidations: [{VACANCY: {severity: fatal}}]",
            "invalid severity 'fatal'",
        ),
        (
            "edge_kinds: {}\nvalidations: [{DANGLING_REF: {severity: error}}]",
            "retired in 0.6.0; write VACANCY instead",
        ),
        (
            "edge_kinds: {}\nvalidations: [{VACANCY: {message: 'see {message}'}}]",
            "'message' is not supported in a fuse profile",
        ),
        (
            "edge_kinds: {}\nmisspelled: true",
            "unknown key 'misspelled'",
        ),
    ] {
        let error = load_fuse_profile(&dir.write("profile.yaml", invalid))
            .unwrap_err()
            .to_string();
        assert!(error.contains(reason), "{invalid}: {error}");
    }
    for (invalid, reason) in [
        ("not json".to_owned(), "invalid trace JSON: expected ident"),
        ("{}".to_owned(), "missing field `header`"),
        (
            trace(json!([{"id": "A"}])).to_string(),
            "missing field `kind`",
        ),
        (
            trace(json!([entry("A", json!([{"tgt": "B"}]))])).to_string(),
            "missing field `kind`",
        ),
    ] {
        let error = parse_trace(&invalid, "z").unwrap_err().to_string();
        assert!(
            error.starts_with("source 'z' emitted") && error.contains(reason),
            "{reason}: {error}"
        );
    }
}

fn cli(path: &Path, format: &str) -> std::process::Output {
    // The fixture's shims run `latt adapter <name>`: point them at this build, not
    // at an installed `latt` on PATH.
    let debug = Path::new(env!("CARGO_BIN_EXE_latt")).parent().unwrap();
    Command::new(env!("CARGO_BIN_EXE_latt"))
        .args(["fuse", "--manifest"])
        .arg(path)
        .args(["--format", format])
        .env("PATH", "/usr/bin:/bin")
        .env("CARGO_TARGET_DIR", debug.parent().unwrap())
        .output()
        .unwrap()
}
// Requirement: builtin-adapter-dispatch/Native adapter names
#[cfg(feature = "native-adapters")]
#[test]
fn mini_fuse_native_names_match_the_adapter_programs() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mini-fuse");
    let native = cli(&fixture.join("fuse-native.yaml"), "json");
    let programs = cli(&fixture.join("fuse.yaml"), "json");
    assert_eq!(
        native.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&native.stdout)
    );
    let value: Value = serde_json::from_slice(&native.stdout).unwrap();
    assert!(!value["edges"].as_array().unwrap().is_empty());
    assert_eq!(native.stdout, programs.stdout);
}
// Requirement: program-composition/Fuse exit codes
#[test]
fn cli_bad_manifest_and_source_failure_are_exit_two_in_every_format() {
    let dir = Scratch::new();
    let path = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    for format in ["plain", "json", "rich"] {
        let output = cli(&path, format);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stdout).contains("SOURCE_FAILURE"));
        assert_eq!(cli(&dir.0.join("missing"), format).status.code(), Some(2));
    }
}
// Requirement: graph-service/Supported service shapes admit additions
// The binary builds this report through `FuseReport::failed`, not a literal.
#[test]
fn cli_fuse_that_cannot_start_reports_one_unsourced_fuse_error() {
    let dir = Scratch::new();
    let missing = dir.0.join("missing.yaml");
    let output = cli(&missing, "json");
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = format!(
        "manifest '{}': No such file or directory (os error 2)",
        missing.display()
    );
    assert_eq!(
        value,
        json!({"edges": [], "header": {"fuse_version": "1"}, "nodes": [], "pathways": [], "findings": [{
            "code": "FUSE_ERROR", "file": missing.display().to_string(), "line": 0,
            "message": message, "node_id": null, "severity": "error", "stage": "manifest"}]})
    );
}

// Requirement: program-composition/Tri-format output
#[test]
fn json_failure_findings_carry_stage() {
    let dir = Scratch::new();
    let stages = |path: &Path| -> Vec<(String, String)> {
        let output = cli(path, "json");
        assert_eq!(output.status.code(), Some(2));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["code"].as_str().unwrap().to_owned(),
                    f["stage"].as_str().unwrap_or("<none>").to_owned(),
                )
            })
            .collect()
    };
    let one = |code: &str, stage: &str| vec![(code.to_owned(), stage.to_owned())];
    assert_eq!(
        stages(&dir.0.join("missing.yaml")),
        one("FUSE_ERROR", "manifest")
    );
    let manifest_text = |profile: &str| {
        format!(
            "manifest_version: '1.0.0'\nname: sample\nversion: '2.0.0'\nfuse_profile: {profile}\nsources:\n  - {{name: z, profile: z.yaml, adapter: z, target: .}}\n"
        )
    };
    let absent = dir.write("absent.fuse.yaml", &manifest_text("absent.yaml"));
    assert_eq!(stages(&absent), one("FUSE_ERROR", "fuse_profile"));
    dir.write(
        "endpoint.yaml",
        "edge_kinds:\n  links:\n    allowed:\n      - ['ghost:item', 'z:item']\n",
    );
    let endpoint = dir.write("endpoint.fuse.yaml", &manifest_text("endpoint.yaml"));
    assert_eq!(stages(&endpoint), one("FUSE_ERROR", "fuse_profile"));
    dir.write("profile.yaml", "edge_kinds: {}");
    let failing = dir.write("failing.fuse.yaml", &manifest_text("profile.yaml"));
    assert_eq!(stages(&failing), one("SOURCE_FAILURE", "source_run"));
    // Unparseable YAML is the manifest's or the fuse profile's by which file.
    let bad_manifest = dir.write("bad.fuse.yaml", "sources: [\n");
    assert_eq!(stages(&bad_manifest), one("FUSE_ERROR", "manifest"));
    dir.write("bad-profile.yaml", "edge_kinds: [\n");
    let bad_profile = dir.write("bad-profile.fuse.yaml", &manifest_text("bad-profile.yaml"));
    assert_eq!(stages(&bad_profile), one("FUSE_ERROR", "fuse_profile"));
}

/// A manifest with two sources whose adapters are the stubs `z` and `a` in
/// `dir`, both writing an empty interface document after running `before`.
#[cfg(unix)]
fn stub_sources(dir: &Scratch, profile: &str, before: &str) -> PathBuf {
    let path = manifest(dir);
    dir.write("profile.yaml", profile);
    dir.write("z.yaml", SOURCE_PROFILE);
    dir.write("a.yaml", SOURCE_PROFILE);
    let document = json!({"interface_version": "1.2", "nodes": [], "edges": [], "pathways": [], "findings": []});
    let script = format!("#!/bin/sh\n{before}\ncat <<'PAYLOAD'\n{document}\nPAYLOAD\n");
    executable(dir, "z", &script);
    executable(dir, "a", &script);
    path
}

// Requirement: program-composition/Tri-format output
#[cfg(unix)]
#[test]
fn json_stage_names_a_composed_profile_refusal_and_a_source_profile_load() {
    let staged = |path: &Path| -> Vec<(Value, Value, Value)> {
        let output = cli(path, "json");
        assert_eq!(output.status.code(), Some(2));
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["code"].clone(), f["stage"].clone(), f["source"].clone()))
            .collect()
    };
    // Each source profile loads alone; only the composed profile refuses the
    // fuse profile's entry.
    let dir = Scratch::new();
    let refused = stub_sources(
        &dir,
        "edge_kinds: {}\nvalidations:\n  - VACANCY: {bogus: 1}\n",
        "",
    );
    assert_eq!(
        staged(&refused),
        vec![(json!("FUSE_ERROR"), json!("fuse_profile"), Value::Null)]
    );
    let plain = cli(&refused, "plain");
    let stderr = String::from_utf8_lossy(&plain.stderr);
    assert!(
        stderr.starts_with("fuse: fuse_profile failed: the composed fuse profile was refused")
            && stderr.ends_with(&format!(". Next: {FIX_THE_FILE}\n")),
        "{stderr}"
    );
    // The trace run read `z.yaml`; the adapter then breaks it, so only fuse's
    // own load of that profile fails.
    let dir = Scratch::new();
    let corrupt = format!(
        "echo 'node_kinds: [' > '{}'",
        dir.0.join("z.yaml").display()
    );
    let broken = stub_sources(&dir, "edge_kinds: {}", &corrupt);
    assert_eq!(
        staged(&broken),
        vec![
            (json!("EMPTY_KIND"), Value::Null, json!("a")),
            (json!("SOURCE_FAILURE"), json!("source_profile"), json!("z"))
        ]
    );
}

// Requirement: program-composition/Tri-format output
// The fixture's shims run `latt adapter <name>`, which a build without native
// adapters does not have.
#[cfg(feature = "native-adapters")]
#[test]
fn json_non_failure_findings_have_no_stage() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/mini-fuse/fuse-native.yaml");
    let output = cli(&path, "json");
    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let findings = value["findings"].as_array().unwrap();
    assert!(!findings.is_empty());
    assert!(findings.iter().all(|f| f.get("stage").is_none()), "{value}");
}

/// The `fuse:` lines on stderr.
fn stderr_causes(output: &std::process::Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| line.starts_with("fuse: "))
        .map(str::to_owned)
        .collect()
}

/// The one object a `json` run writes on stderr, as `validate` shapes findings.
fn stderr_object(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stderr).unwrap_or_else(|e| {
        panic!(
            "stderr is not one JSON value ({e}): {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

// Requirement: program-composition/Fuse exit codes
#[test]
fn fuse_source_failure_names_the_source_on_stderr() {
    let dir = Scratch::new();
    let path = dir.write("fuse.yaml", "manifest_version: '1.0.0'\nname: sample\nversion: '2.0.0'\nfuse_profile: profile.yaml\nsources:\n  - {name: specs, profile: specs.yaml, adapter: z, target: .}\n");
    dir.write("profile.yaml", "edge_kinds: {}");
    for format in ["plain", "rich"] {
        let output = cli(&path, format);
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(stderr.lines().count(), 1, "{format}: {stderr}");
        let line = stderr.lines().next().unwrap();
        assert!(
            line.starts_with("fuse: source 'specs' failed at source_run: trace exited")
                && line.ends_with(&format!(". Next: {RUN_TRACE_ALONE}")),
            "{format}: {line}"
        );
    }
    let output = cli(&path, "json");
    assert_eq!(output.status.code(), Some(2));
    let findings = stderr_object(&output)["findings"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0]["code"], "SOURCE_FAILURE");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(
        message.starts_with("source 'specs' failed at source_run: trace exited"),
        "{message}"
    );
    assert!(findings[0].get("stage").is_none(), "{findings:?}");
}

/// The `Next:` action for a source that failed to run or to be read.
const RUN_TRACE_ALONE: &str = "run `latt trace` with that source's profile, adapter and target from the manifest and read its stderr";
/// The `Next:` action for a manifest, fuse profile or composition failure.
const FIX_THE_FILE: &str = "fix the file or entry the message names";

// Requirement: program-composition/Fuse exit codes
#[test]
fn a_control_character_in_a_source_name_stays_off_the_stderr_line() {
    let dir = Scratch::new();
    let path = dir.write("fuse.yaml", "manifest_version: '1.0.0'\nname: sample\nversion: '2.0.0'\nfuse_profile: profile.yaml\nsources:\n  - {name: \"evil\\nFORGED\\e[31m\", profile: specs.yaml, adapter: z, target: .}\n");
    dir.write("profile.yaml", "edge_kinds: {}");
    let control_free = |bytes: &[u8], format: &str| {
        let text = String::from_utf8_lossy(bytes);
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{format}: {text:?}"
        );
    };
    for format in ["plain", "rich"] {
        let output = cli(&path, format);
        assert_eq!(output.status.code(), Some(2), "{format}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(stderr.lines().count(), 1, "{format}: {stderr:?}");
        assert!(
            stderr.starts_with(r"fuse: source 'evil\nFORGED\u{1b}[31m' failed at source_run: "),
            "{format}: {stderr:?}"
        );
        control_free(&output.stderr, format);
    }
    let output = cli(&path, "json");
    assert_eq!(output.status.code(), Some(2));
    control_free(&output.stderr, "json");
    let findings = stderr_object(&output)["findings"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(findings.len(), 1, "{findings:?}");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(!message.chars().any(char::is_control), "{message:?}");
    assert!(
        message.starts_with(r"source 'evil\nFORGED\u{1b}[31m' failed at source_run: "),
        "{message:?}"
    );
}

// Requirement: program-composition/Fuse exit codes
#[test]
fn fuse_cannot_start_prints_one_stderr_cause() {
    let dir = Scratch::new();
    let no_profile = dir.write("fuse.yaml", "manifest_version: '1.0.0'\nname: sample\nversion: '2.0.0'\nfuse_profile: absent.yaml\nsources:\n  - {name: z, profile: z.yaml, adapter: z, target: .}\n");
    for (path, stage) in [
        (dir.0.join("missing.yaml"), "manifest"),
        (no_profile, "fuse_profile"),
    ] {
        let json_run = cli(&path, "json");
        assert_eq!(json_run.status.code(), Some(2));
        let stdout: Value = serde_json::from_slice(&json_run.stdout).unwrap();
        let message = stdout["findings"][0]["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(stdout["findings"][0]["code"], "FUSE_ERROR");
        let stderr = stderr_object(&json_run);
        let findings = stderr["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1, "{stderr}");
        assert_eq!(findings[0]["code"], "FUSE_ERROR");
        let cause = format!("{stage} failed: {message}. Next: {FIX_THE_FILE}");
        assert_eq!(findings[0]["message"], cause.as_str());
        for format in ["plain", "rich"] {
            let output = cli(&path, format);
            assert_eq!(output.status.code(), Some(2));
            assert_eq!(
                String::from_utf8_lossy(&output.stderr),
                format!("fuse: {cause}\n"),
                "{format}"
            );
        }
    }
}

// Requirement: program-composition/Fuse exit codes
#[test]
fn two_failing_sources_report_two_stderr_causes_once_each() {
    let dir = Scratch::new();
    let path = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    for format in ["plain", "rich"] {
        let causes = stderr_causes(&cli(&path, format));
        assert_eq!(causes.len(), 2, "{format}: {causes:?}");
        assert!(
            causes[0].starts_with("fuse: source 'z' failed at source_run: "),
            "{causes:?}"
        );
        assert!(
            causes[1].starts_with("fuse: source 'a' failed at source_run: "),
            "{causes:?}"
        );
    }
    let stderr = stderr_object(&cli(&path, "json"));
    // The validate shape orders findings by its own sort key, not manifest order.
    let mut messages: Vec<_> = stderr["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["message"].as_str().unwrap().to_owned())
        .collect();
    messages.sort();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(
        messages[0].starts_with("source 'a' failed at source_run: "),
        "{messages:?}"
    );
    assert!(
        messages[1].starts_with("source 'z' failed at source_run: "),
        "{messages:?}"
    );
}

// Requirement: program-composition/Fuse exit codes
// Golden stdout as rc.3 wrote it, json gaining only `stage`: the stderr cause
// adds nothing to stdout.
#[test]
fn fuse_stdout_is_unchanged_by_the_stderr_cause() {
    let dir = Scratch::new();
    let path = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    let root = dir.0.display();
    let failure = |name: &str| {
        format!(
            "trace exited 2: Error: Profile error: failed to read profile {root}/{name}.yaml: no such file; no profile found in '{root}'"
        )
    };
    let text = format!(
        "Fuse sample 2.0.0: 0 nodes, 0 edges, 0 pathways\nERROR SOURCE_FAILURE [z] <source>:0 {}\nERROR SOURCE_FAILURE [a] <source>:0 {}\n",
        failure("z"),
        failure("a")
    );
    for format in ["plain", "rich"] {
        assert_eq!(
            String::from_utf8_lossy(&cli(&path, format).stdout),
            text,
            "{format}"
        );
    }
    let finding = |name: &str| {
        json!({"code": "SOURCE_FAILURE", "file": "<source>", "line": 0, "message": failure(name),
            "node_id": null, "severity": "error", "source": name, "stage": "source_run"})
    };
    let value: Value = serde_json::from_slice(&cli(&path, "json").stdout).unwrap();
    assert_eq!(
        value,
        json!({"edges": [], "findings": [finding("z"), finding("a")],
            "header": {"fuse_version": "1", "manifest_version": "1.0.0", "name": "sample", "version": "2.0.0"},
            "nodes": [], "pathways": []})
    );
    let missing = dir.0.join("missing.yaml");
    let shown = missing.display();
    let text = format!(
        "Fuse <unknown> : 0 nodes, 0 edges, 0 pathways\nERROR FUSE_ERROR [] {shown}:0 manifest '{shown}': No such file or directory (os error 2)\n"
    );
    for format in ["plain", "rich"] {
        assert_eq!(
            String::from_utf8_lossy(&cli(&missing, format).stdout),
            text,
            "{format}"
        );
    }
}

// Requirement: output/Untrusted text in human formats
// json is the lossless channel: a source's stderr reaches `findings[].message`
// as it came, controls JSON-escaped, while plain and rich escape them visibly.
#[cfg(unix)]
#[test]
fn a_failing_source_stderr_keeps_its_bytes_in_fuse_json() {
    let dir = Scratch::new();
    let adapter = executable(
        &dir,
        "adapter",
        "#!/bin/sh\nprintf 'bad\\033]8;;x\\007\\n' >&2\nexit 1\n",
    );
    let path = dir.write(
        "fuse.yaml",
        "manifest_version: '1.0.0'\nname: sample\nversion: '2.0.0'\nfuse_profile: profile.yaml\nsources:\n  - {name: s, profile: s.yaml, adapter: adapter, target: .}\n",
    );
    dir.write("profile.yaml", "edge_kinds: {}");
    dir.write("s.yaml", SOURCE_PROFILE);
    let output = cli(&path, "json");
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let golden = format!(
        "{{\n  \"edges\": [],\n  \"findings\": [\n    {{\n      \"code\": \"SOURCE_FAILURE\",\n      \"file\": \"<source>\",\n      \"line\": 0,\n      \"message\": \"trace exited 2: Error: adapter '{}' exited 1: bad\\u001b]8;;x\\u0007\",\n      \"node_id\": null,\n      \"severity\": \"error\",\n      \"source\": \"s\",\n      \"stage\": \"source_run\"\n    }}\n  ],\n  \"header\": {{\n    \"fuse_version\": \"1\",\n    \"manifest_version\": \"1.0.0\",\n    \"name\": \"sample\",\n    \"version\": \"2.0.0\"\n  }},\n  \"nodes\": [],\n  \"pathways\": []\n}}\n",
        adapter.display()
    );
    assert_eq!(stdout, golden);
    for format in ["plain", "rich"] {
        let output = cli(&path, format);
        assert_eq!(output.status.code(), Some(2), "{format}");
        for bytes in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(
                !text.chars().any(|c| c.is_control() && c != '\n'),
                "{format}: {text:?}"
            );
            assert!(text.contains(r"bad\u{1b}]8;;x\u{7}"), "{format}: {text:?}");
        }
    }
    // Run directly, the same trace escapes its stderr in every format.
    for format in ["plain", "json"] {
        let output = Command::new(env!("CARGO_BIN_EXE_latt"))
            .current_dir(&dir.0)
            .args(["trace", "--profile", "s.yaml", "--adapter"])
            .arg(&adapter)
            .args(["--target", ".", "--format", format])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{format}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(r"bad\u{1b}]8;;x\u{7}") && !stderr.contains('\u{1b}'),
            "{format}: {stderr:?}"
        );
    }
}

// Requirement: program-composition/Fuse exit codes
// The fixture's shims run `latt adapter <name>`, which a build without native
// adapters does not have.
#[cfg(feature = "native-adapters")]
#[test]
fn a_clean_or_findings_only_fuse_run_writes_nothing_to_stderr() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/mini-fuse/fuse-native.yaml");
    for format in ["plain", "json", "rich"] {
        let output = cli(&path, format);
        assert_eq!(output.status.code(), Some(1), "{format}");
        assert!(stderr_causes(&output).is_empty(), "{format}");
        if format == "json" {
            assert!(
                output.stderr.is_empty(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
// Requirement: program-composition/Source-qualified merge with colon separator
// The fixture's shims run `latt adapter <name>`, which a build without native
// adapters does not have.
#[cfg(feature = "native-adapters")]
#[test]
fn mini_fuse_live_fixture_has_real_edges_findings_and_pathways() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mini-fuse/fuse.yaml");
    let output = cli(&path, "json");
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!value["edges"].as_array().unwrap().is_empty());
    assert!(
        value["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["kind"] == "markdown:requirement")
    );
    assert!(
        value["pathways"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"].as_str().unwrap().starts_with("toml:"))
    );
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn standard_constraints_run_on_the_composed_graph() {
    let dir = Scratch::new();
    let profile = "edge_kinds: {}\nvalidations:\n  - CONSTRAINT:\n      kind: 'a:item'\n      expect: {title: {eq: expected}}\n      severity: error\n";
    let report = report(
        &dir,
        trace(json!([])),
        trace(json!([entry("A", json!([]))])),
        profile,
        false,
    );
    assert_eq!(report.exit_code(), 1);
    let finding = report
        .findings
        .iter()
        .find(|f| f.issue.code == "CONSTRAINT")
        .unwrap();
    assert_eq!(finding.source.as_deref(), Some("a"));
    assert_eq!(finding.issue.node_id.as_deref(), Some("a:A"));
}

// Requirement: validation/EDGE_WHERE evaluation
// Requirement: validation/EDGE_COUNT evaluation
#[test]
fn edge_validators_run_on_the_composed_graph() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item]]\nvalidations:\n  \
                   - EDGE_WHERE:\n      edge_kind: links\n      reject:\n        edge: {weight: {eq: 2}}\n  \
                   - EDGE_COUNT:\n      edge_kind: links\n      node_kind: 'a:item'\n      direction: in\n      max: 0\n";
    let report = report(
        &dir,
        trace(json!([entry("Z", json!([edge("A")]))])),
        trace(json!([entry("A", json!([]))])),
        profile,
        false,
    );
    let codes: Vec<(&str, Option<&str>)> = report
        .findings
        .iter()
        .map(|f| (f.issue.code.as_str(), f.issue.node_id.as_deref()))
        .collect();
    assert!(codes.contains(&("EDGE_WHERE", Some("z:Z"))), "{codes:?}");
    assert!(codes.contains(&("EDGE_COUNT", Some("a:A"))), "{codes:?}");
}

// Requirement: program-composition/Source-qualified merge with colon separator
#[test]
fn local_edges_repeated_allowed_pairs_and_raw_colons_keep_identity() {
    let dir = Scratch::new();
    let profile = "edge_kinds:\n  links:\n    allowed: [[z:item, a:item], [z:item, a:item]]\n";
    let mut local = edge("Z:two");
    local["kind"] = "local".into();
    let report = report(
        &dir,
        trace(json!([
            entry("Z:one", json!([local, edge("A")])),
            entry("Z:two", json!([]))
        ])),
        trace(json!([entry("A", json!([]))])),
        profile,
        false,
    );
    assert_eq!(report.exit_code(), 0);
    assert_eq!(report.edges[0].tgt, "z:Z:two");
    assert_eq!(report.edges[1].tgt, "a:A");
}

// Requirement: program-composition/Cross-source edge resolution via allowed pairings
#[test]
fn a_source_vacancy_is_withdrawn_when_fuse_resolves_the_edge() {
    let dir = Scratch::new();
    let vacancy = |target: &str| {
        json!({"code": "VACANCY", "severity": "hint", "file": "edges", "line": 7, "node_id": "Z",
               "message": format!("edge 'Z'->'{target}' (kind 'links'): target '{target}' does not exist")})
    };
    let mut z = trace(json!([entry("Z", json!([edge("A"), edge("B")]))]));
    z["entries"][0]["findings"] = json!([vacancy("A"), vacancy("B")]);
    let report = report(
        &dir,
        z,
        trace(json!([entry("A", json!([]))])),
        PROFILE,
        false,
    );

    assert_eq!(report.edges[0].target_source.as_deref(), Some("a"));
    let vacancies: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.issue.code == "VACANCY")
        .map(|f| (f.issue.message.as_str(), f.source.as_deref()))
        .collect();
    // The resolved edge's source VACANCY is gone; the unresolved declared edge
    // is reported once, by fuse, not twice.
    assert_eq!(
        vacancies,
        [(
            "edge 'z:Z'->'B' (kind 'links'): target 'B' does not exist in an allowed source kind",
            Some("z")
        )]
    );
}

// Requirement: program-composition/Cross-source edge resolution via allowed pairings
#[test]
fn a_source_vacancy_for_an_undeclared_edge_kind_remains() {
    let dir = Scratch::new();
    let mut z = trace(json!([entry("Z", json!([edge("B")]))]));
    z["entries"][0]["findings"] = json!([{
        "code": "VACANCY", "severity": "warning", "file": "edges", "line": 7, "node_id": "Z",
        "message": "edge 'Z'->'B' (kind 'links'): target 'B' does not exist"
    }]);
    let report = report(&dir, z, trace(json!([])), "edge_kinds: {}", false);
    let vacancies: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.issue.code == "VACANCY")
        .map(|f| f.issue.message.as_str())
        .collect();
    assert_eq!(
        vacancies,
        ["edge 'Z'->'B' (kind 'links'): target 'B' does not exist"]
    );
}

/// A source profile whose `item` kind is not `orphan_ok`, so the composed run
/// owes it orphan findings; `severity` overrides `UNREFERENCED` when given.
fn orphan_profile(unreferenced: Option<&str>) -> String {
    let validations = unreferenced.map_or(String::new(), |s| {
        format!("validations:\n  - UNREFERENCED: {{severity: {s}}}\n")
    });
    format!(
        "name: local\nprofile_version: '1.0.0'\nnode_kinds: {{item: {{id_pattern: '.*'}}}}\nedge_kinds: {{links: {{allowed: [[item, item]]}}}}\n{validations}"
    )
}
fn orphan(code: &str, id: &str, suppressed: bool) -> Value {
    json!({"code": code, "severity": "warning", "file": "input", "line": 3, "node_id": id,
           "message": format!("node '{id}' has no {} edges", if code == "UNREFERENCED" {"incoming"} else {"outgoing"}),
           "suppressed": suppressed})
}
fn orphan_findings(
    report: &latt_core::types::FuseReport,
    code: &str,
) -> Vec<(String, String, String, bool)> {
    report
        .findings
        .iter()
        .filter(|f| f.issue.code == code)
        .map(|f| {
            (
                f.issue.node_id.clone().unwrap(),
                f.source.clone().unwrap(),
                f.issue.severity.to_string(),
                f.issue.suppressed,
            )
        })
        .collect()
}

// Requirement: program-composition/Standard validators on the composed graph
#[test]
fn orphan_findings_are_recomputed_on_the_composed_graph() {
    let dir = Scratch::new();
    dir.write("z.yaml", &orphan_profile(None));
    dir.write("a.yaml", &orphan_profile(Some("error")));
    // z lists A and B; T2's only edge resolves nowhere; T3's names T2, which
    // the fuse profile's pairing refuses although z's own run resolved it.
    // a's own run reported every node unreferenced and untraced, as it must.
    let z = trace(json!([
        entry("T1", json!([edge("A"), edge("B")])),
        entry("T2", json!([edge("missing")])),
        entry("T3", json!([edge("T2")])),
    ]));
    let mut a = trace(json!([
        entry("A", json!([])),
        entry("B", json!([])),
        entry("C", json!([]))
    ]));
    for (i, id) in ["A", "B", "C"].iter().enumerate() {
        a["entries"][i]["findings"] = json!([
            orphan("UNREFERENCED", id, false),
            orphan("UNTRACED", id, false)
        ]);
    }
    let composed = report(&dir, z, a, PROFILE, false);

    assert_eq!(
        composed
            .edges
            .iter()
            .filter(|e| e.target_kind.is_some())
            .count(),
        2
    );
    // Every UNREFERENCED/UNTRACED is the composed run's: none carries a's text.
    assert!(composed.findings.iter().all(|f| {
        !f.issue.message.ends_with("edges")
            || f.issue
                .node_id
                .as_deref()
                .is_some_and(|id| id.contains(':'))
    }));
    assert_eq!(
        composed
            .findings
            .iter()
            .filter(|f| f.issue.code == "VACANCY")
            .count(),
        2
    );
    let s = |v: &str| v.to_string();
    // T3's refused edge still reaches T2, as it did in z's own run.
    assert_eq!(
        orphan_findings(&composed, "UNREFERENCED"),
        [
            (s("a:C"), s("a"), s("error"), false),
            (s("z:T1"), s("z"), s("warning"), false),
            (s("z:T3"), s("z"), s("warning"), false),
        ]
    );
    // T2's and T3's unresolved edges still leave them, as in z's own run.
    assert_eq!(
        orphan_findings(&composed, "UNTRACED"),
        [
            (s("a:A"), s("a"), s("warning"), false),
            (s("a:B"), s("a"), s("warning"), false),
            (s("a:C"), s("a"), s("warning"), false),
        ]
    );

    // The fuse profile's own override wins over the source's.
    let dir = Scratch::new();
    dir.write("z.yaml", &orphan_profile(None));
    dir.write("a.yaml", &orphan_profile(Some("error")));
    let composed = report(
        &dir,
        trace(json!([entry("T1", json!([edge("A")]))])),
        trace(json!([entry("A", json!([])), entry("C", json!([]))])),
        &format!("{PROFILE}  - UNREFERENCED: {{severity: hint}}\n"),
        false,
    );
    assert_eq!(
        orphan_findings(&composed, "UNREFERENCED"),
        [
            (s("a:C"), s("a"), s("hint"), false),
            (s("z:T1"), s("z"), s("hint"), false),
        ]
    );
}

// Requirement: program-composition/Standard validators on the composed graph
// Requirement: validation/Orphan checks follow the allowed pairs
#[test]
fn composed_orphan_checks_follow_source_and_fuse_pairs_not_observed_edges() {
    let dir = Scratch::new();
    // z allows no pair, so only the fuse profile's [z:item, a:item] places
    // z:item, as a source. a allows [item, item], which no edge here uses:
    // a:item may still point somewhere, so a node that does not is untraced.
    dir.write(
        "z.yaml",
        "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*'}}\nedge_kinds: {}\n",
    );
    dir.write("a.yaml", &orphan_profile(None));
    let composed = report(
        &dir,
        trace(json!([
            entry("T1", json!([edge("A")])),
            entry("T4", json!([]))
        ])),
        trace(json!([entry("A", json!([])), entry("C", json!([]))])),
        PROFILE,
        false,
    );
    let ids = |code| {
        let mut ids: Vec<_> = orphan_findings(&composed, code)
            .into_iter()
            .map(|(id, ..)| id)
            .collect();
        ids.sort_unstable();
        ids
    };
    assert_eq!(ids("UNREFERENCED"), ["a:C"], "nothing may point at z:item");
    assert_eq!(ids("UNTRACED"), ["a:A", "a:C", "z:T4"]);
}

// Requirement: program-composition/Cross-source edge resolution via allowed pairings
#[test]
fn target_must_match_an_allowed_kind_even_if_the_raw_id_exists() {
    let dir = Scratch::new();
    // `a` declares `other`, so the pair is a real one that `A`, an `item`, fails.
    dir.write(
        "a.yaml",
        "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}, other: {id_pattern: '.*'}}\nedge_kinds: {}\n",
    );
    let report = report(
        &dir,
        trace(json!([entry("Z", json!([edge("A")]))])),
        trace(json!([entry("A", json!([]))])),
        "edge_kinds: {links: {allowed: [['z:item', 'a:other']]}}",
        false,
    );
    assert_eq!(report.exit_code(), 1);
    assert!(report.findings.iter().any(|f| f.issue.code == "VACANCY"));
    assert!(report.edges[0].target_kind.is_none());
}

#[cfg(unix)]
fn executable(dir: &Scratch, name: &str, text: &str) -> PathBuf {
    let path = dir.0.join(name);
    stub::write_stub(&path, text);
    path
}

// Requirement: program-composition/Run each source through latt trace
#[cfg(unix)]
#[test]
fn failed_trace_never_yields_a_partial_graph_and_keeps_healthy_findings() {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    let mut payload = trace(json!([entry("Z", json!([]))]));
    payload["unattachable_findings"] = json!([{"code": "LOCAL", "severity": "error", "message": "source finding", "file": "x", "line": 1}]);
    let binary = executable(
        &dir,
        "trace-stub",
        &format!(
            "#!/bin/sh\ncase \"$*\" in\n  *z.yaml*) cat <<'PAYLOAD'\n{payload}\nPAYLOAD\nexit 1;;\n  *) echo broken >&2; exit 2;;\nesac\n"
        ),
    );
    let report = latt_core::fuse::fuse(&manifest, &binary, true).unwrap();
    assert_eq!(report.exit_code(), 2);
    assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
    assert_eq!(report.findings[0].issue.code, "LOCAL");
    assert_eq!(report.findings[0].source.as_deref(), Some("z"));
    assert_eq!(report.findings[1].issue.code, "SOURCE_FAILURE");
    assert_eq!(report.findings[1].source.as_deref(), Some("a"));
}

// Requirement: program-composition/Run each source through latt trace
#[cfg(unix)]
#[test]
fn trace_rejection_through_fuse_produces_source_failure() {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    let mut unsupported = trace(json!([]));
    unsupported["header"]["trace_version"] = json!("99");
    let repeated = trace(json!([entry("Z", json!([])), entry("Z", json!([]))]));
    let mut invalid_severity = trace(json!([entry("Z", json!([]))]));
    invalid_severity["entries"][0]["findings"] =
        json!([{"code": "LOCAL", "severity": "fatal", "message": "check", "file": "x", "line": 1}]);
    for (payload, reason) in [
        (unsupported, "unsupported or missing trace_version"),
        (repeated, "repeated trace entry 'Z'"),
        (invalid_severity, "invalid finding severity 'fatal'"),
    ] {
        let binary = executable(
            &dir,
            "trace-stub",
            &format!("#!/bin/sh\ncat <<'PAYLOAD'\n{payload}\nPAYLOAD\nexit 0\n"),
        );
        let report = latt_core::fuse::fuse(&manifest, &binary, false).unwrap();
        assert_eq!(report.exit_code(), 2);
        assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
        assert_eq!(report.findings.len(), 2);
        for (finding, source) in report.findings.iter().zip(["z", "a"]) {
            assert_eq!(finding.issue.code, "SOURCE_FAILURE");
            assert_eq!(finding.issue.severity, latt_core::types::Severity::Error);
            assert_eq!(finding.source.as_deref(), Some(source));
            assert_eq!(
                finding.issue.message,
                format!("source '{source}' emitted invalid trace JSON: {reason}")
            );
        }
    }
}

// Requirement: program-composition/Run each source through latt trace
#[cfg(unix)]
#[test]
fn mixed_success_findings_preserve_manifest_order() {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    let source_profile = "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}}\nedge_kinds: {links: {allowed: [[item, item]]}}\npathways: [phase]\n";
    dir.write("z.yaml", source_profile);
    dir.write("a.yaml", source_profile);
    let document = json!({"interface_version": "1.2",
        "nodes": [
            {"id": "X", "kind": "item", "attrs": {}, "provenance": {"file": "x", "line": 1}},
            {"id": "Y", "kind": "item", "attrs": {}, "provenance": {"file": "x", "line": 2}}
        ],
        "edges": [{"src": "X", "tgt": "Y", "kind": "links", "provenance": {"file": "x", "line": 1}}],
        "pathways": [{"name": "phase", "order": ["first", "last"], "current": "first"}],
        "findings": [
        {"code": "PARSE_ERROR", "severity": "error", "message": "first", "provenance": {"file": "x", "line": 1}},
        {"code": "PARSE_ERROR", "severity": "error", "message": "second", "provenance": {"file": "x", "line": 2}}
    ]});
    let binary = Path::new(env!("CARGO_BIN_EXE_latt"));
    for (healthy, failed) in [("z", "a"), ("a", "z")] {
        executable(
            &dir,
            healthy,
            &format!("#!/bin/sh\ncat <<'PAYLOAD'\n{document}\nPAYLOAD\n"),
        );
        executable(&dir, failed, "#!/bin/sh\necho broken >&2\nexit 2\n");
        let report = latt_core::fuse::fuse(&manifest, binary, false).unwrap();
        assert_eq!(report.exit_code(), 2);
        assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
        assert_eq!(report.findings.len(), 3);
        for (finding, message) in report.findings[..2].iter().zip(["first", "second"]) {
            assert_eq!(finding.issue.code, "PARSE_ERROR");
            assert_eq!(finding.issue.message, message);
            assert_eq!(finding.issue.severity, latt_core::types::Severity::Error);
            assert_eq!(finding.source.as_deref(), Some(healthy));
        }
        assert_eq!(report.findings[2].issue.code, "SOURCE_FAILURE");
        assert_eq!(report.findings[2].source.as_deref(), Some(failed));
    }
}

// Requirement: program-composition/Run each source through latt trace
// Requirement: program-composition/Standard validators on the composed graph
#[cfg(unix)]
#[test]
fn mixed_success_with_strict_promotes_healthy_warnings() {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    let source_profile = "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}}\nedge_kinds: {}\n";
    dir.write("z.yaml", source_profile);
    dir.write("a.yaml", source_profile);
    let document = json!({"interface_version": "1.2", "nodes": [], "edges": [], "pathways": [], "findings": [
        {"code": "PARSE_ERROR", "severity": "warning", "message": "source warning", "provenance": {"file": "x", "line": 1}}
    ]});
    executable(
        &dir,
        "z",
        &format!("#!/bin/sh\ncat <<'PAYLOAD'\n{document}\nPAYLOAD\n"),
    );
    executable(&dir, "a", "#!/bin/sh\necho broken >&2\nexit 2\n");
    let binary = Path::new(env!("CARGO_BIN_EXE_latt"));
    for (strict, severity) in [
        (false, latt_core::types::Severity::Warning),
        (true, latt_core::types::Severity::Error),
    ] {
        let report = latt_core::fuse::fuse(&manifest, binary, strict).unwrap();
        assert_eq!(report.exit_code(), 2);
        assert!(report.nodes.is_empty() && report.edges.is_empty() && report.pathways.is_empty());
        // `z` declares a kind its empty register never uses; that `EMPTY_KIND`
        // is the source's own and not what this scenario is about.
        let findings: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.issue.code != "EMPTY_KIND")
            .collect();
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].issue.code, "PARSE_ERROR");
        assert_eq!(findings[0].issue.severity, severity);
        assert_eq!(findings[0].source.as_deref(), Some("z"));
        assert_eq!(findings[1].issue.code, "SOURCE_FAILURE");
        assert_eq!(
            findings[1].issue.severity,
            latt_core::types::Severity::Error
        );
        assert_eq!(findings[1].source.as_deref(), Some("a"));
    }
}

// Requirement: program-composition/Run each source through latt trace
// Requirement: program-composition/Fuse exit codes
#[cfg(unix)]
#[test]
fn malformed_trace_and_unexecutable_binary_are_exit_two() {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    for (output, status) in [
        ("not json".into(), 0),
        ("{}".into(), 1),
        (trace(json!([{"id": "broken"}])).to_string(), 0),
        (trace(json!([])).to_string(), 2),
    ] {
        let binary = executable(
            &dir,
            "trace-stub",
            &format!("#!/bin/sh\ncat <<'PAYLOAD'\n{output}\nPAYLOAD\nexit {status}\n"),
        );
        let report = latt_core::fuse::fuse(&manifest, &binary, false).unwrap();
        assert_eq!(report.exit_code(), 2);
        assert_eq!(report.findings.len(), 2);
        assert!(
            report
                .findings
                .iter()
                .all(|f| f.issue.code == "SOURCE_FAILURE")
        );
    }
    assert_eq!(
        latt_core::fuse::fuse(&manifest, &dir.0.join("absent"), false)
            .unwrap()
            .exit_code(),
        2
    );
}

// Requirement: program-composition/Fuse exit codes
// Requirement: program-composition/Tri-format output
#[cfg(unix)]
#[test]
fn cli_clean_warning_strict_and_source_errors_have_correct_exit_codes() {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    let source_profile = "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}}\nedge_kinds: {}\n";
    dir.write("z.yaml", source_profile);
    dir.write("a.yaml", source_profile);
    for severity in [None, Some("warning"), Some("error"), Some("hint")] {
        let findings: Vec<Value> = severity.into_iter().map(|s| json!({"code": "PARSE_ERROR", "severity": s, "message": "check", "provenance": {"file": "x", "line": 1}})).collect();
        let document = json!({"interface_version": "1.2", "nodes": [], "edges": [], "pathways": [], "findings": findings});
        let script = format!("#!/bin/sh\ncat <<'PAYLOAD'\n{document}\nPAYLOAD\n");
        executable(&dir, "z", &script);
        executable(&dir, "a", &script);
        for format in ["plain", "json", "rich"] {
            let result = cli(&manifest, format);
            let expected = if severity == Some("error") { 1 } else { 0 };
            assert_eq!(
                result.status.code(),
                Some(expected),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
            let strict = Command::new(env!("CARGO_BIN_EXE_latt"))
                .args(["fuse", "--manifest"])
                .arg(&manifest)
                .args(["--format", format, "--strict"])
                .output()
                .unwrap();
            let expected = if matches!(severity, Some("error" | "warning")) {
                1
            } else {
                0
            };
            assert_eq!(
                strict.status.code(),
                Some(expected),
                "{}",
                String::from_utf8_lossy(&strict.stdout)
            );
        }
    }
}

// Requirement: program-composition/Read a fuse profile
// Requirement: program-composition/Suppression in program composition

fn findings_json(report: &latt_core::types::FuseReport) -> Vec<Value> {
    let value: Value = serde_json::from_str(&output_result(report, Format::Json)).unwrap();
    value["findings"].as_array().unwrap().clone()
}

#[test]
fn fuse_profile_with_a_suppress_entry_loads_and_rejects_config_error_suppression() {
    let dir = Scratch::new();
    load_fuse_profile(&dir.write(
        "ok.yaml",
        "edge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: VACANCY}\n",
    ))
    .expect("a SUPPRESS entry is accepted on a fuse profile");
    let error = load_fuse_profile(&dir.write(
        "bad.yaml",
        "edge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: CONFIG_ERROR}\n",
    ))
    .expect_err("CONFIG_ERROR cannot be suppressed at the fuse level either")
    .to_string();
    assert!(error.contains("CONFIG_ERROR"), "{error}");
    let error = load_fuse_profile(&dir.write(
        "typo.yaml",
        "edge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: VACANCY, node_id: [x]}\n",
    ))
    .expect_err("unknown keys are rejected")
    .to_string();
    assert!(error.contains("node_id"), "{error}");
}

#[test]
fn suppress_source_level_suppression_survives_the_merge() {
    let dir = Scratch::new();
    dir.write("z.yaml", SOURCE_PROFILE);
    dir.write("a.yaml", SOURCE_PROFILE);
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["entries"][0]["findings"] = json!([{
        "code": "PARSE_ERROR", "severity": "error", "message": "m",
        "file": "x", "line": 1, "suppressed": true
    }]);
    let report = report(&dir, z, trace(json!([])), "edge_kinds: {}", false);
    assert_eq!(
        report.exit_code(),
        0,
        "a source-suppressed error never gates"
    );
    let findings = findings_json(&report);
    let parse = findings
        .iter()
        .find(|f| f["code"] == "PARSE_ERROR")
        .unwrap();
    assert_eq!(parse["suppressed"], true);
    assert_eq!(parse["source"], "z");
    assert!(
        !findings.iter().any(|f| f["code"] == "SUPPRESS_UNUSED"),
        "the fuse profile declared nothing, so nothing is stale"
    );
    let plain = output_result(&report, Format::Plain);
    assert!(!plain.contains("PARSE_ERROR"), "{plain}");
}

// Requirement: program-composition/Suppression in program composition
#[test]
fn suppress_a_source_suppressed_orphan_stays_suppressed_when_recomputed() {
    let dir = Scratch::new();
    dir.write("z.yaml", &orphan_profile(None));
    dir.write("a.yaml", SOURCE_PROFILE);
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["entries"][0]["findings"] = json!([
        orphan("UNTRACED", "Z", true),
        orphan("UNREFERENCED", "Z", false)
    ]);
    let report = report(&dir, z, trace(json!([])), "edge_kinds: {}", false);
    let s = |v: &str| v.to_string();
    assert_eq!(
        orphan_findings(&report, "UNTRACED"),
        [(s("z:Z"), s("z"), s("warning"), true)]
    );
    assert_eq!(
        orphan_findings(&report, "UNREFERENCED"),
        [(s("z:Z"), s("z"), s("warning"), false)]
    );
    assert_eq!(report.exit_code(), 0);
    let plain = output_result(&report, Format::Plain);
    assert!(!plain.contains("UNTRACED"), "{plain}");
    assert!(plain.contains("UNREFERENCED"), "{plain}");
}

/// Sources `z` and `a` where each `z` node's only edge names its own ID: the
/// `z` run resolves it to the node itself, so reports no orphan, and fuse
/// resolves it to the same-named `a` node, leaving the `z` node unreferenced.
/// `suppress` is the `z` profile's `SUPPRESS` entry; the fuse profile
/// suppresses the shared IDs' `CROSS_SOURCE_DUPLICATE_ID`, as Helix's does.
fn self_named_report(dir: &Scratch, ids: &[&str], suppress: &str) -> latt_core::types::FuseReport {
    dir.write(
        "z.yaml",
        &format!(
            "{}validations:\n  - SUPPRESS: {suppress}\n",
            orphan_profile(None)
        ),
    );
    let z = trace(ids.iter().map(|id| entry(id, json!([edge(id)]))).collect());
    let a = trace(ids.iter().map(|id| entry(id, json!([]))).collect());
    let profile = format!("{PROFILE}  - SUPPRESS: {{code: CROSS_SOURCE_DUPLICATE_ID}}\n");
    report(dir, z, a, &profile, false)
}

// Requirement: program-composition/Suppression in program composition
#[test]
fn suppress_a_source_entry_reaches_an_orphan_only_composition_made() {
    let dir = Scratch::new();
    let report = self_named_report(&dir, &["X"], "{code: UNREFERENCED}");
    let s = |v: &str| v.to_string();
    assert_eq!(
        orphan_findings(&report, "UNREFERENCED"),
        [(s("z:X"), s("z"), s("warning"), true)]
    );
    assert_eq!(report.exit_code(), 0);
}

// Requirement: program-composition/Suppression in program composition
#[test]
fn suppress_a_source_entry_with_node_ids_matches_raw_ids() {
    let dir = Scratch::new();
    let report = self_named_report(&dir, &["X", "Y"], "{code: UNREFERENCED, node_ids: [X]}");
    let s = |v: &str| v.to_string();
    assert_eq!(
        orphan_findings(&report, "UNREFERENCED"),
        [
            (s("z:X"), s("z"), s("warning"), true),
            (s("z:Y"), s("z"), s("warning"), false)
        ]
    );
}

/// [`self_named_report`] where the `z` run, which saw no orphan, also reported
/// its `SUPPRESS` entry unused, as a real run of that profile does.
fn self_named_report_with_unused(
    dir: &Scratch,
    ids: &[&str],
    suppress: &str,
    unused: &str,
) -> latt_core::types::FuseReport {
    dir.write(
        "z.yaml",
        &format!(
            "{}validations:\n  - SUPPRESS: {suppress}\n",
            orphan_profile(None)
        ),
    );
    let mut z = trace(ids.iter().map(|id| entry(id, json!([edge(id)]))).collect());
    z["entries"][0]["findings"] = json!([{
        "code": "SUPPRESS_UNUSED", "severity": "info", "file": "<profile>", "line": 0,
        "node_id": null, "message": unused
    }]);
    let a = trace(ids.iter().map(|id| entry(id, json!([]))).collect());
    let profile = format!("{PROFILE}  - SUPPRESS: {{code: CROSS_SOURCE_DUPLICATE_ID}}\n");
    report(dir, z, a, &profile, false)
}

fn unused_messages(report: &latt_core::types::FuseReport) -> Vec<(Option<String>, String)> {
    report
        .findings
        .iter()
        .filter(|f| f.issue.code == "SUPPRESS_UNUSED")
        .map(|f| (f.source.clone(), f.issue.message.clone()))
        .collect()
}

// Requirement: program-composition/Suppression in program composition
#[test]
fn suppress_a_source_entry_the_composed_graph_uses_is_not_reported_unused() {
    let dir = Scratch::new();
    let report = self_named_report_with_unused(
        &dir,
        &["X"],
        "{code: UNREFERENCED}",
        "SUPPRESS entry for 'UNREFERENCED' matched no finding: 'UNREFERENCED' is a built-in \
         code; no finding carried it this run",
    );
    assert_eq!(
        orphan_findings(&report, "UNREFERENCED"),
        [(
            "z:X".to_string(),
            "z".to_string(),
            "warning".to_string(),
            true
        )]
    );
    assert_eq!(unused_messages(&report), []);
}

// Requirement: program-composition/Suppression in program composition
#[test]
fn suppress_a_source_entry_names_only_the_node_ids_the_composed_graph_left_unmatched() {
    let dir = Scratch::new();
    let report = self_named_report_with_unused(
        &dir,
        &["X"],
        "{code: UNREFERENCED, node_ids: [X, W]}",
        "SUPPRESS entry for 'UNREFERENCED' matched no finding for node_ids [\"X\", \"W\"]: \
         'UNREFERENCED' is a built-in code; no finding carried it this run",
    );
    assert_eq!(
        unused_messages(&report),
        [(
            Some("z".to_string()),
            "SUPPRESS entry for 'UNREFERENCED' matched no finding for node_ids [\"W\"]: \
             'UNREFERENCED' is a built-in code; findings carried it this run, but none for \
             those node_ids"
                .to_string()
        )]
    );
}

// Requirement: program-composition/Suppression in program composition
#[test]
fn suppress_a_source_entry_the_composed_graph_leaves_unmatched_is_reported_unused() {
    let dir = Scratch::new();
    dir.write(
        "a.yaml",
        &format!(
            "{}validations:\n  - SUPPRESS: {{code: UNREFERENCED, node_ids: [A]}}\n",
            orphan_profile(None)
        ),
    );
    let z = trace(json!([entry("Z", json!([edge("A")]))]));
    let mut a = trace(json!([entry("A", json!([]))]));
    a["entries"][0]["findings"] = json!([orphan("UNREFERENCED", "A", true)]);
    let report = report(&dir, z, a, PROFILE, false);
    assert_eq!(orphan_findings(&report, "UNREFERENCED"), []);
    assert_eq!(
        unused_messages(&report),
        [(
            Some("a".to_string()),
            "SUPPRESS entry for 'UNREFERENCED' matched no finding for node_ids [\"A\"]: \
             'UNREFERENCED' is a built-in code; no finding carried it this run"
                .to_string()
        )]
    );
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn suppress_fuse_level_entry_hides_cross_source_vacancies() {
    let dir = Scratch::new();
    let profile = format!("{PROFILE}  - SUPPRESS: {{code: VACANCY}}\n");
    let report = report(
        &dir,
        trace(json!([entry("X", json!([edge("absent")]))])),
        trace(json!([entry("A", json!([]))])),
        &profile,
        false,
    );
    assert_eq!(report.exit_code(), 0);
    let findings = findings_json(&report);
    let vacancy = findings.iter().find(|f| f["code"] == "VACANCY").unwrap();
    assert_eq!(vacancy["suppressed"], true);
    assert_eq!(vacancy["severity"], "error");
    assert!(!findings.iter().any(|f| f["code"] == "SUPPRESS_UNUSED"));
    for format in [Format::Plain, Format::Rich] {
        let text = output_result(&report, format);
        assert!(!text.contains("VACANCY"), "{format}: {text}");
    }
}

#[test]
fn suppress_fuse_level_node_ids_match_composed_ids() {
    let dir = Scratch::new();
    let profile = format!("{PROFILE}  - SUPPRESS: {{code: VACANCY, node_ids: ['z:X']}}\n");
    let report = report(
        &dir,
        trace(json!([
            entry("X", json!([edge("absent")])),
            entry("Y", json!([edge("absent")]))
        ])),
        trace(json!([entry("A", json!([]))])),
        &profile,
        false,
    );
    assert_eq!(report.exit_code(), 1, "z:Y's vacancy still gates");
    let findings = findings_json(&report);
    let by_node = |id: &str| {
        findings
            .iter()
            .find(|f| f["code"] == "VACANCY" && f["node_id"] == id)
            .unwrap_or_else(|| panic!("no VACANCY for {id}"))
            .clone()
    };
    assert_eq!(by_node("z:X")["suppressed"], true);
    assert!(by_node("z:Y").get("suppressed").is_none());
    assert!(!findings.iter().any(|f| f["code"] == "SUPPRESS_UNUSED"));
}

#[test]
fn suppress_fuse_level_unmatched_entry_reports_against_the_fuse_profile() {
    let dir = Scratch::new();
    let profile = format!("{PROFILE}  - SUPPRESS: {{code: AMBIGUOUS_CROSS_REF}}\n");
    let report = report(
        &dir,
        trace(json!([entry("Z", json!([edge("A")]))])),
        trace(json!([entry("A", json!([]))])),
        &profile,
        false,
    );
    assert_eq!(report.exit_code(), 0);
    let unused = report
        .findings
        .iter()
        .find(|f| f.issue.code == "SUPPRESS_UNUSED")
        .expect("a stale fuse-level entry is reported");
    assert_eq!(unused.issue.severity, latt_core::types::Severity::Info);
    assert_eq!(unused.issue.origin, Origin::FuseProfile);
    assert_eq!(unused.issue.node_id, None);
    assert_eq!(unused.source, None);
    assert!(
        unused.issue.message.contains("AMBIGUOUS_CROSS_REF"),
        "{}",
        unused.issue.message
    );
}

#[test]
fn suppress_fuse_level_entry_does_not_reach_source_findings() {
    // A source's PARSE_ERROR is that source's to suppress: the fuse entry matches
    // nothing the command itself collected, so it is stale, and the source
    // finding keeps gating.
    let dir = Scratch::new();
    dir.write("z.yaml", SOURCE_PROFILE);
    dir.write("a.yaml", SOURCE_PROFILE);
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["entries"][0]["findings"] = json!([{
        "code": "PARSE_ERROR", "severity": "error", "message": "m", "file": "x", "line": 1
    }]);
    let report = report(
        &dir,
        z,
        trace(json!([])),
        "edge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: PARSE_ERROR}\n",
        false,
    );
    assert_eq!(report.exit_code(), 1);
    let findings = findings_json(&report);
    let parse = findings
        .iter()
        .find(|f| f["code"] == "PARSE_ERROR")
        .unwrap();
    assert!(parse.get("suppressed").is_none());
    assert!(findings.iter().any(|f| f["code"] == "SUPPRESS_UNUSED"));
}

#[test]
fn suppress_strict_promotes_before_fuse_level_suppression() {
    let dir = Scratch::new();
    dir.write("z.yaml", SOURCE_PROFILE);
    dir.write("a.yaml", SOURCE_PROFILE);
    let profile =
        format!("{PROFILE}  - VACANCY: {{severity: warning}}\n  - SUPPRESS: {{code: VACANCY}}\n");
    let report = report(
        &dir,
        trace(json!([entry("X", json!([edge("absent")]))])),
        trace(json!([entry("A", json!([]))])),
        &profile,
        true,
    );
    assert_eq!(report.exit_code(), 0);
    let vacancy = report
        .findings
        .iter()
        .find(|f| f.issue.code == "VACANCY")
        .unwrap();
    assert_eq!(vacancy.issue.severity, latt_core::types::Severity::Error);
    assert!(vacancy.issue.suppressed);
}

// Requirement: program-composition/Member findings keep their notes
#[test]
fn notes_on_source_findings_survive_composition() {
    let dir = Scratch::new();
    let sp = "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}}\nedge_kinds: {}\n";
    dir.write("z.yaml", sp);
    dir.write("a.yaml", sp);
    let note = json!({"text": "Lands with M2.", "until": {"pathway": "phase", "position": "last"}, "by": "jone", "at": "2026-10-01", "expired": true});
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["entries"][0]["findings"] = json!([{"code": "VACANCY", "severity": "error", "message": "m", "file": "x", "line": 1, "note": note}]);
    let report = report(&dir, z, trace(json!([])), "edge_kinds: {}", false);
    let finding = &report.findings[0];
    let carried = finding.issue.note.as_ref().expect("the note survives");
    assert_eq!(carried.text, "Lands with M2.");
    assert!(carried.expired);
    let json: Value = serde_json::from_str(&latt_core::output::output_result(
        &report,
        latt_core::output::Format::Json,
    ))
    .unwrap();
    assert_eq!(json["findings"][0]["note"], note);
}

#[test]
fn notes_a_malformed_note_in_a_source_trace_is_refused() {
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["entries"][0]["findings"] = json!([{"code": "VACANCY", "severity": "error", "message": "m", "file": "x", "line": 1, "note": {"text": "x", "until": "2026-02-30"}}]);
    let error = parse_trace(&z.to_string(), "z").unwrap_err().to_string();
    assert!(error.contains("2026-02-30"), "{error}");
}

// Requirement: program-composition/Read a fuse profile
#[test]
fn notes_a_fuse_profile_cannot_declare_annotations() {
    let dir = Scratch::new();
    let error =
        load_fuse_profile(&dir.write("profile.yaml", "edge_kinds: {}\nannotations: notes.yaml\n"))
            .unwrap_err()
            .to_string();
    assert!(
        error.contains("annotations") && error.contains("source profiles"),
        "{error}"
    );
}

// Requirement: program-composition/Fuse profile endpoints name declared kinds

/// An endpoint naming a source the manifest lacks, or a kind its source does not
/// declare, matches nothing: the run is refused, naming every such endpoint and
/// what exists, rather than composing an empty kind in silence.
#[test]
fn a_fuse_endpoint_naming_no_source_or_kind_refuses_the_run() {
    let dir = Scratch::new();
    for name in ["z.yaml", "a.yaml"] {
        dir.write(name, SOURCE_PROFILE);
    }
    let manifest = load_manifest(&manifest(&dir)).unwrap();
    let profile = load_fuse_profile(&dir.write(
        "profile.yaml",
        "edge_kinds:\n  links:\n    allowed: [[ghost:item, a:item], [z:itme, a:item], [z:item, a:item]]\n",
    ))
    .unwrap();
    let traces = || {
        vec![
            parse_trace(&trace(json!([entry("Z", json!([]))])).to_string(), "z").unwrap(),
            parse_trace(&trace(json!([entry("A", json!([]))])).to_string(), "a").unwrap(),
        ]
    };
    let error = assemble(&manifest, &profile, traces(), false).unwrap_err();
    assert_eq!(
        error.to_string(),
        "fuse profile edge kind 'links' names 'ghost:item', but the manifest has no source \
         'ghost' (sources: z, a); fuse profile edge kind 'links' names 'z:itme', but source \
         'z' declares no kind 'itme' (kinds: item)"
    );
}

// Requirement: program-composition/A source trace line outside 32 bits is a source failure
#[cfg(unix)]
#[test]
fn trace_line_outside_32_bits_fails_the_source_and_keeps_healthy_findings() {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    let mut healthy = trace(json!([entry("Z", json!([]))]));
    healthy["unattachable_findings"] = json!([{"code": "LOCAL", "severity": "warning", "message": "kept", "file": "x", "line": 1}]);
    let mut negative = trace(json!([entry("A", json!([]))]));
    negative["unattachable_findings"] =
        json!([{"code": "LOCAL", "severity": "warning", "message": "m", "file": "x", "line": -1}]);
    let mut too_large = trace(json!([entry("A", json!([]))]));
    too_large["entries"][0]["provenance"]["line"] = json!(4_294_967_296_u64);
    for (broken, line) in [(negative, "-1"), (too_large, "4294967296")] {
        let binary = executable(
            &dir,
            "trace-stub",
            &format!(
                "#!/bin/sh\ncase \"$*\" in\n  *z.yaml*) cat <<'PAYLOAD'\n{healthy}\nPAYLOAD\nexit 0;;\n  *) cat <<'PAYLOAD'\n{broken}\nPAYLOAD\nexit 0;;\nesac\n"
            ),
        );
        let report = latt_core::fuse::fuse(&manifest, &binary, false).unwrap();
        assert_eq!(report.exit_code(), 2, "line {line}");
        assert!(report.nodes.is_empty(), "line {line}");
        assert_eq!(report.findings[0].issue.code, "LOCAL", "line {line}");
        let failure = &report.findings[1];
        assert_eq!(failure.issue.code, "SOURCE_FAILURE", "line {line}");
        assert_eq!(failure.source.as_deref(), Some("a"));
        assert!(
            failure.issue.message.contains(line),
            "line {line}: {}",
            failure.issue.message
        );
    }
}

#[test]
fn trace_line_at_the_32_bit_maximum_is_kept() {
    let dir = Scratch::new();
    let mut z = trace(json!([entry("Z", json!([]))]));
    z["unattachable_findings"] = json!([{"code": "LOCAL", "severity": "warning", "message": "m", "file": "x", "line": 4_294_967_295_u64}]);
    let report = report(&dir, z, trace(json!([])), "edge_kinds: {}", false);
    let local = report
        .findings
        .iter()
        .find(|finding| finding.issue.code == "LOCAL")
        .expect("the source finding is composed");
    assert_eq!(
        local
            .issue
            .origin
            .location()
            .expect("a file origin")
            .line()
            .map(u32::from),
        Some(u32::MAX)
    );
}

/// Write source `name` over the native TOML adapter: `register` is its
/// `reg.toml`, `kinds` the rest of its profile, `notes` its annotations file
/// (none when empty).
#[cfg(feature = "native-adapters")]
fn toml_source(dir: &Scratch, name: &str, register: &str, kinds: &str, notes: &str) {
    fs::create_dir(dir.0.join(name)).unwrap();
    dir.write(&format!("{name}/reg.toml"), register);
    let annotations = if notes.is_empty() {
        String::new()
    } else {
        dir.write(&format!("{name}-notes.yaml"), notes);
        format!("annotations: {name}-notes.yaml\n")
    };
    dir.write(
        &format!("{name}.yaml"),
        &format!(
            "name: {name}\nprofile_version: '1.0.0'\n{annotations}adapter:\n  native: toml\n  paths: {{files: [reg.toml]}}\n  tables:\n    req: {{kind: req, id_key: id, edge_keys: {{controls: controls}}}}\n    haz: {{kind: haz, id_key: id}}\n{kinds}"
        ),
    );
}

/// Run `latt <args>` in `dir`; return its exit code and stdout.
#[cfg(feature = "native-adapters")]
fn run_in(dir: &Scratch, args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_latt"))
        .args(args)
        .current_dir(&dir.0)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

/// Run `latt <args> --format json` in `dir`; return its exit code and document.
#[cfg(feature = "native-adapters")]
fn run_json(dir: &Scratch, args: &[&str]) -> (Option<i32>, Value) {
    let (code, stdout) = run_in(dir, &[args, &["--format", "json"]].concat());
    let value = serde_json::from_str(&stdout).unwrap_or_else(|error| panic!("{error}: {stdout}"));
    (code, value)
}

/// Write a manifest fusing `sources` (each written by `toml_source`) under
/// `fuse_profile`.
#[cfg(feature = "native-adapters")]
fn write_fuse(dir: &Scratch, sources: &[&str], fuse_profile: &str) {
    dir.write("fuse-profile.yaml", fuse_profile);
    let entries: String = sources
        .iter()
        .map(|name| {
            format!("  - {{name: {name}, profile: {name}.yaml, adapter: toml, target: {name}}}\n")
        })
        .collect();
    dir.write(
        "manifest.yaml",
        &format!("manifest_version: '1.0.0'\nname: t\nversion: '1.0.0'\nfuse_profile: fuse-profile.yaml\nsources:\n{entries}"),
    );
}

#[cfg(feature = "native-adapters")]
fn toml_fuse(dir: &Scratch, sources: &[&str], fuse_profile: &str) -> (Option<i32>, Value) {
    write_fuse(dir, sources, fuse_profile);
    run_json(dir, &["fuse", "--manifest", "manifest.yaml"])
}

#[cfg(feature = "native-adapters")]
fn with_code<'a>(document: &'a Value, code: &str) -> Vec<&'a Value> {
    document["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["code"] == code)
        .collect()
}

#[cfg(feature = "native-adapters")]
fn at<'a>(document: &'a Value, code: &str, node: &str) -> Vec<&'a Value> {
    with_code(document, code)
        .into_iter()
        .filter(|finding| finding["node_id"] == node)
        .collect()
}

/// Return the one finding with `code` on `node`, failing on none or several.
#[cfg(feature = "native-adapters")]
fn one<'a>(document: &'a Value, code: &str, node: &str) -> &'a Value {
    let found = at(document, code, node);
    assert_eq!(found.len(), 1, "{code} on {node}: {document}");
    found[0]
}

/// Return the `EMPTY_KIND` messages, sorted, after checking each is a nodeless
/// profile finding at the default severity.
#[cfg(feature = "native-adapters")]
fn empty_kind_messages(document: &Value) -> Vec<&str> {
    let mut messages: Vec<&str> = with_code(document, "EMPTY_KIND")
        .iter()
        .map(|finding| {
            assert_eq!(finding["node_id"], Value::Null, "{finding}");
            assert_eq!(finding["severity"], "info", "{finding}");
            assert_eq!(finding["file"], "<profile>", "{finding}");
            assert_eq!(finding["line"], 0, "{finding}");
            finding["message"].as_str().unwrap()
        })
        .collect();
    messages.sort_unstable();
    messages
}

#[cfg(feature = "native-adapters")]
const REQ_HAZ: &str = "node_kinds:\n  req: {id_pattern: '^R-[0-9]+$'}\n  haz: {id_pattern: '^H-[0-9]+$'}\nedge_kinds:\n  controls: {allowed: [[req, haz]]}\n";
#[cfg(feature = "native-adapters")]
const DECLARES_CONTROLS: &str = "edge_kinds:\n  controls: {allowed: [[a:req, a:haz]]}\n";

// Requirement: validation/EMPTY_KIND built-in code
// Requirement: program-composition/Standard validators on the composed graph

/// Edges whose targets no source declares are still edges of their kind: the
/// fuse lists them, so it must not also call their kind empty.
#[cfg(feature = "native-adapters")]
#[test]
fn a_kind_whose_every_edge_dangles_is_not_empty() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\ncontrols = ['H-1']\n[[req]]\nid = 'R-2'\ncontrols = ['H-2']\n",
        "node_kinds:\n  req: {id_pattern: '^R-[0-9]+$', orphan_ok: true}\nedge_kinds:\n  controls: {cross_source: true, allowed: []}\n",
        "",
    );
    let (code, fused) = toml_fuse(
        &dir,
        &["a"],
        "edge_kinds: {}\nvalidations:\n  - VACANCY: {}\n",
    );
    assert_eq!(code, Some(0), "{fused}");
    let edges = fused["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2, "{fused}");
    assert!(
        edges
            .iter()
            .all(|edge| edge["kind"] == "controls" && edge["target_kind"].is_null()),
        "{fused}"
    );
    let vacancies = with_code(&fused, "VACANCY");
    assert_eq!(vacancies.len(), 2, "{fused}");
    assert!(
        vacancies
            .iter()
            .all(|finding| finding["severity"] == "hint")
    );
    assert_eq!(empty_kind_messages(&fused), Vec::<&str>::new(), "{fused}");
}

// Requirement: validation/EMPTY_KIND built-in code

/// Resolved and dangling edges both populate their kind; a kind with neither is
/// still reported.
#[cfg(feature = "native-adapters")]
#[test]
fn only_kinds_with_no_instance_are_empty_beside_resolved_and_dangling_edges() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\ncontrols = ['H-1']\n[[req]]\nid = 'R-2'\ncontrols = ['H-404']\n[[haz]]\nid = 'H-1'\n",
        "node_kinds:\n  req: {id_pattern: '^R-[0-9]+$'}\n  haz: {id_pattern: '^H-[0-9]+$'}\n  doc: {id_pattern: '^D-[0-9]+$'}\nedge_kinds:\n  controls: {allowed: [[req, haz]]}\n  cites: {allowed: [[req, doc]]}\n",
        "",
    );
    let (_, fused) = toml_fuse(&dir, &["a"], DECLARES_CONTROLS);
    let resolved: Vec<bool> = fused["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| !edge["target_kind"].is_null())
        .collect();
    assert_eq!(resolved.len(), 2, "{fused}");
    assert!(
        resolved.contains(&true) && resolved.contains(&false),
        "{fused}"
    );
    assert_eq!(
        empty_kind_messages(&fused),
        [
            "edge kind 'cites' has no edge",
            "node kind 'a:doc' has no node",
        ],
        "{fused}"
    );
}

// Requirement: validation/EMPTY_KIND built-in code
// Requirement: program-composition/Source-qualified merge with colon separator

/// A source's empty kinds and the composed run's are one population: each
/// empty kind is reported once, a node kind under its qualified name, and a kind
/// no edge pair mentions is still among them.
#[cfg(feature = "native-adapters")]
#[test]
fn each_empty_kind_is_reported_once_under_its_composed_name() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n",
        &REQ_HAZ.replace(
            "edge_kinds:",
            "  doc: {id_pattern: '^D-[0-9]+$'}\nedge_kinds:",
        ),
        "",
    );
    let (code, fused) = toml_fuse(&dir, &["a"], "edge_kinds: {}\n");
    assert_eq!(code, Some(0), "{fused}");
    assert_eq!(
        empty_kind_messages(&fused),
        [
            "edge kind 'controls' has no edge",
            "node kind 'a:doc' has no node",
            "node kind 'a:haz' has no node",
        ],
        "{fused}"
    );
}

// Requirement: validation/EMPTY_KIND built-in code

/// An edge kind is shared by name across sources: one source with no edge of it
/// does not make it empty when another source has one.
#[cfg(feature = "native-adapters")]
#[test]
fn an_edge_kind_populated_in_one_source_is_not_empty_in_the_fuse() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\ncontrols = ['H-1']\n[[haz]]\nid = 'H-1'\n",
        REQ_HAZ,
        "",
    );
    toml_source(
        &dir,
        "b",
        "[[req]]\nid = 'R-2'\n[[haz]]\nid = 'H-2'\n",
        REQ_HAZ,
        "",
    );
    let (code, fused) = toml_fuse(&dir, &["a", "b"], "edge_kinds: {}\n");
    assert_eq!(code, Some(0), "{fused}");
    assert_eq!(fused["edges"].as_array().unwrap().len(), 1, "{fused}");
    assert!(
        with_code(&fused, "CROSS_SOURCE_DUPLICATE_ID").is_empty(),
        "{fused}"
    );
    assert_eq!(empty_kind_messages(&fused), Vec::<&str>::new(), "{fused}");
}

// Requirement: validation/EMPTY_KIND built-in code

/// An edge kind no source has an edge of is reported once, however many sources
/// declare it.
#[cfg(feature = "native-adapters")]
#[test]
fn an_edge_kind_empty_in_every_source_is_reported_once() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n[[haz]]\nid = 'H-1'\n",
        REQ_HAZ,
        "",
    );
    toml_source(
        &dir,
        "b",
        "[[req]]\nid = 'R-2'\n[[haz]]\nid = 'H-2'\n",
        REQ_HAZ,
        "",
    );
    let (code, fused) = toml_fuse(&dir, &["a", "b"], "edge_kinds: {}\n");
    assert_eq!(code, Some(0), "{fused}");
    assert_eq!(
        empty_kind_messages(&fused),
        ["edge kind 'controls' has no edge"],
        "{fused}"
    );
}

// Requirement: program-composition/Suppression in program composition

/// The composed `EMPTY_KIND` takes the fuse profile's severity override, which
/// `--strict` then promotes, and its `SUPPRESS`; each setting is used.
#[cfg(feature = "native-adapters")]
#[test]
fn a_fuse_profile_overrides_and_suppresses_the_composed_empty_kind() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n",
        "node_kinds:\n  req: {id_pattern: '^R-[0-9]+$', orphan_ok: true}\n  haz: {id_pattern: '^H-[0-9]+$', orphan_ok: true}\nedge_kinds:\n  controls: {allowed: [[req, haz]]}\n",
        "",
    );
    let severities = |profile: &str, strict: bool| {
        write_fuse(&dir, &["a"], profile);
        let mut args = vec!["fuse", "--manifest", "manifest.yaml"];
        if strict {
            args.push("--strict");
        }
        let (code, fused) = run_json(&dir, &args);
        for unused in ["OVERRIDE_UNUSED", "SUPPRESS_UNUSED"] {
            assert!(with_code(&fused, unused).is_empty(), "{fused}");
        }
        let mut found: Vec<(String, bool)> = with_code(&fused, "EMPTY_KIND")
            .iter()
            .map(|finding| {
                (
                    finding["severity"].as_str().unwrap().to_string(),
                    finding["suppressed"] == true,
                )
            })
            .collect();
        found.sort();
        (code, found)
    };
    let warning = "edge_kinds: {}\nvalidations:\n  - EMPTY_KIND: {severity: warning}\n";
    let two = |severity: &str, suppressed: bool| vec![(severity.to_string(), suppressed); 2];
    assert_eq!(severities(warning, false), (Some(0), two("warning", false)));
    assert_eq!(severities(warning, true), (Some(1), two("error", false)));
    let suppress = format!("{warning}  - SUPPRESS: {{code: EMPTY_KIND}}\n");
    assert_eq!(severities(&suppress, true), (Some(0), two("error", true)));
}

/// Write source `a` with three annotated findings fuse withdraws and recomputes:
/// `UNTRACED` on R-1, `UNREFERENCED` on H-9, and `VACANCY` on R-2, whose edge
/// kind the fuse profile declares.
#[cfg(feature = "native-adapters")]
fn noted_source(dir: &Scratch) {
    toml_source(
        dir,
        "a",
        "[[req]]\nid = 'R-1'\n[[req]]\nid = 'R-2'\ncontrols = ['H-404']\n[[haz]]\nid = 'H-9'\n",
        REQ_HAZ,
        "- finding: {code: UNTRACED, node: R-1}\n  note: R-1 controls nothing yet.\n\
         - finding: {code: UNREFERENCED, node: H-9}\n  note: incoming pending\n\
         - finding: {code: VACANCY, node: R-2}\n  note: target pending\n",
    );
}

/// Return the source's own finding for `code` on raw `node`, and the fuse's.
#[cfg(feature = "native-adapters")]
fn noted_pair(code: &str, node: &str) -> (Value, Value) {
    let dir = Scratch::new();
    noted_source(&dir);
    let (_, alone) = run_json(&dir, &["validate", "--profile", "a.yaml", "--target", "a"]);
    let (_, fused) = toml_fuse(&dir, &["a"], DECLARES_CONTROLS);
    let source = one(&alone, code, node).clone();
    assert!(!source["note"].is_null(), "{alone}");
    (source, one(&fused, code, &format!("a:{node}")).clone())
}

// Requirement: program-composition/Member findings keep their notes
// Requirement: finding-notes/Notes attach to findings

#[cfg(feature = "native-adapters")]
#[test]
fn a_source_note_reaches_the_untraced_finding_fuse_recomputes() {
    let (source, fused) = noted_pair("UNTRACED", "R-1");
    assert_eq!(fused["note"], source["note"], "{fused}");
}

// Requirement: program-composition/Member findings keep their notes

#[cfg(feature = "native-adapters")]
#[test]
fn a_source_note_reaches_the_unreferenced_finding_fuse_recomputes() {
    let (source, fused) = noted_pair("UNREFERENCED", "H-9");
    assert_eq!(fused["note"], source["note"], "{fused}");
}

// Requirement: program-composition/Member findings keep their notes

/// The vacancy fuse judges replaces the source's, with fuse's own message; the
/// note moves onto the replacement.
#[cfg(feature = "native-adapters")]
#[test]
fn a_source_note_reaches_the_vacancy_fuse_replaces() {
    let (source, fused) = noted_pair("VACANCY", "R-2");
    assert!(
        fused["message"]
            .as_str()
            .unwrap()
            .starts_with("edge 'a:R-2'->'H-404'"),
        "{fused}"
    );
    assert_eq!(fused["note"], source["note"], "{fused}");
}

// Requirement: finding-notes/Notes attach to findings

/// One note attaches to every finding with its code and node: each vacancy fuse
/// judges on the node carries it.
#[cfg(feature = "native-adapters")]
#[test]
fn a_source_note_reaches_every_replacement_vacancy_on_its_node() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-2'\ncontrols = ['H-404', 'H-405']\n",
        REQ_HAZ,
        "- finding: {code: VACANCY, node: R-2}\n  note: targets pending\n",
    );
    let (_, fused) = toml_fuse(&dir, &["a"], DECLARES_CONTROLS);
    let vacancies = at(&fused, "VACANCY", "a:R-2");
    assert_eq!(vacancies.len(), 2, "{fused}");
    assert!(
        vacancies
            .iter()
            .all(|finding| finding["note"]["text"] == "targets pending"),
        "{fused}"
    );
}

// Requirement: program-composition/Member findings keep their notes

/// A note belongs to its source: another source declaring the same raw ID does
/// not receive it.
#[cfg(feature = "native-adapters")]
#[test]
fn a_source_note_does_not_reach_another_source_with_the_same_raw_id() {
    let dir = Scratch::new();
    let note = "- finding: {code: UNTRACED, node: R-1}\n  note: a's own.\n";
    toml_source(&dir, "a", "[[req]]\nid = 'R-1'\n", REQ_HAZ, note);
    toml_source(&dir, "b", "[[req]]\nid = 'R-1'\n", REQ_HAZ, "");
    let (_, fused) = toml_fuse(&dir, &["a", "b"], "edge_kinds: {}\n");
    assert_eq!(
        one(&fused, "UNTRACED", "a:R-1")["note"]["text"],
        "a's own.",
        "{fused}"
    );
    one(&fused, "UNTRACED", "b:R-1");
    let on_b: Vec<&Value> = fused["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["node_id"] == "b:R-1")
        .collect();
    assert!(
        on_b.iter().all(|finding| finding["note"].is_null()),
        "{fused}"
    );
}

// Requirement: program-composition/Member findings keep their notes

/// A raw ID may hold the qualifier's separator; its note still finds it.
#[cfg(feature = "native-adapters")]
#[test]
fn a_source_note_reaches_a_recomputed_finding_on_a_raw_id_with_a_colon() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R:1'\n",
        &REQ_HAZ.replace("^R-[0-9]+$", "^R:[0-9]+$"),
        "- finding: {code: UNTRACED, node: 'R:1'}\n  note: colon kept\n",
    );
    let (_, fused) = toml_fuse(&dir, &["a"], "edge_kinds: {}\n");
    assert_eq!(
        one(&fused, "UNTRACED", "a:R:1")["note"]["text"],
        "colon kept",
        "{fused}"
    );
}

// Requirement: program-composition/Member findings keep their notes
// Requirement: program-composition/Suppression in program composition

/// A recomputed finding keeps the source's suppression and the whole expired
/// note, and the source's `NOTE_EXPIRED` is neither lost nor repeated.
#[cfg(feature = "native-adapters")]
#[test]
fn a_suppressed_finding_keeps_its_expired_note_through_recomputation() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n",
        &format!("{REQ_HAZ}validations:\n  - SUPPRESS: {{code: UNTRACED, node_ids: [R-1]}}\n"),
        "- finding: {code: UNTRACED, node: R-1}\n  note: was due\n  until: 2020-01-01\n  by: jone\n  at: 2019-12-01\n",
    );
    let (_, alone) = run_json(&dir, &["validate", "--profile", "a.yaml", "--target", "a"]);
    let source = one(&alone, "UNTRACED", "R-1");
    assert_eq!(source["note"]["expired"], true, "{alone}");
    assert_eq!(source["suppressed"], true, "{alone}");
    let (_, fused) = toml_fuse(&dir, &["a"], "edge_kinds: {}\n");
    let recomputed = one(&fused, "UNTRACED", "a:R-1");
    assert_eq!(recomputed["suppressed"], true, "{fused}");
    assert_eq!(recomputed["note"], source["note"], "{fused}");
    assert_eq!(with_code(&alone, "NOTE_EXPIRED").len(), 1, "{alone}");
    assert_eq!(with_code(&fused, "NOTE_EXPIRED").len(), 1, "{fused}");
}

// Requirement: program-composition/Member findings keep their notes

/// Fuse copies notes a source attached; it does not read annotations against the
/// composed graph, so a note that matched nothing in its source stays unattached.
#[cfg(feature = "native-adapters")]
#[test]
fn a_source_note_that_matched_nothing_does_not_attach_to_a_fuse_finding() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n",
        "node_kinds:\n  req: {id_pattern: '^R-[0-9]+$', orphan_ok: true}\nedge_kinds: {}\n",
        "- finding: {code: CONSTRAINT, node: R-1}\n  note: owner pending\n",
    );
    let (_, alone) = run_json(&dir, &["validate", "--profile", "a.yaml", "--target", "a"]);
    assert_eq!(with_code(&alone, "NOTE_UNUSED").len(), 1, "{alone}");
    let (_, fused) = toml_fuse(
        &dir,
        &["a"],
        "edge_kinds: {}\nvalidations:\n  - CONSTRAINT: {kind: 'a:req', expect: {owner: {present: true}}}\n",
    );
    assert_eq!(
        one(&fused, "CONSTRAINT", "a:R-1")["note"],
        Value::Null,
        "{fused}"
    );
}

// Requirement: program-composition/Tri-format output

/// A copied note renders in `plain` and `rich` as a source's own note does.
#[cfg(feature = "native-adapters")]
#[test]
fn a_copied_note_renders_in_plain_and_rich() {
    let dir = Scratch::new();
    noted_source(&dir);
    write_fuse(&dir, &["a"], DECLARES_CONTROLS);
    for format in ["plain", "rich"] {
        let (_, text) = run_in(
            &dir,
            &["fuse", "--manifest", "manifest.yaml", "--format", format],
        );
        for note in [
            "R-1 controls nothing yet.",
            "incoming pending",
            "target pending",
        ] {
            assert!(text.contains(note), "{format} lacks '{note}':\n{text}");
        }
    }
}

// Requirement: program-composition/A source profile that cannot load is a source failure

/// When assembly fails, the healthy source's findings arrive as it reported
/// them: its own empty kinds, unqualified, and its notes.
#[cfg(feature = "native-adapters")]
#[test]
fn a_failed_assembly_keeps_the_healthy_source_empty_kinds_and_notes() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n",
        REQ_HAZ,
        "- finding: {code: UNTRACED, node: R-1}\n  note: kept\n",
    );
    fs::create_dir(dir.0.join("b")).unwrap();
    dir.write("b.yaml", "name: b\nnode_kinds: [unloadable\n");
    let (code, fused) = toml_fuse(&dir, &["a", "b"], "edge_kinds: {}\n");
    assert_eq!(code, Some(2), "{fused}");
    assert_eq!(
        one(&fused, "UNTRACED", "a:R-1")["note"]["text"],
        "kept",
        "{fused}"
    );
    let mut empty: Vec<(&str, &str)> = with_code(&fused, "EMPTY_KIND")
        .iter()
        .map(|finding| {
            (
                finding["source"].as_str().unwrap(),
                finding["message"].as_str().unwrap(),
            )
        })
        .collect();
    empty.sort_unstable();
    assert_eq!(
        empty,
        [
            ("a", "edge kind 'controls' has no edge"),
            ("a", "node kind 'haz' has no node"),
        ],
        "{fused}"
    );
}

// Requirement: program-composition/Standard validators on the composed graph
// Requirement: program-composition/Member findings keep their notes

/// A malformed ID the source already reported is reported once: the source's
/// finding, unchanged but for qualification, with its note.
#[cfg(feature = "native-adapters")]
#[test]
fn a_source_id_format_finding_is_reported_once_as_the_source_reported_it() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'BAD'\n",
        REQ_HAZ,
        "- finding: {code: ID_FORMAT, node: BAD}\n  note: legacy id\n",
    );
    let (alone_code, alone) = run_json(&dir, &["validate", "--profile", "a.yaml", "--target", "a"]);
    let (fused_code, fused) = toml_fuse(&dir, &["a"], "edge_kinds: {}\n");
    assert_eq!((alone_code, fused_code), (Some(1), Some(1)), "{fused}");
    let source = one(&alone, "ID_FORMAT", "BAD");
    let carried = one(&fused, "ID_FORMAT", "a:BAD");
    for key in ["message", "file", "line", "severity", "suppressed", "note"] {
        assert_eq!(carried[key], source[key], "{key}: {fused}");
    }
    assert_eq!(carried["note"]["text"], "legacy id");
    assert_eq!(carried["source"], "a");
}

// Requirement: program-composition/Standard validators on the composed graph

/// Fuse skips its own ID check only for the node a source already reported:
/// another source's node with the same raw ID is still checked.
#[test]
fn the_composed_id_check_still_covers_a_node_its_source_did_not_report() {
    let dir = Scratch::new();
    let source_profile = "name: local\nprofile_version: '1.0.0'\nnode_kinds:\n  item: {id_pattern: '^[A-Z]+-[0-9]+$', orphan_ok: true}\nedge_kinds: {}\n";
    dir.write("z.yaml", source_profile);
    dir.write("a.yaml", source_profile);
    let mut z = trace(json!([entry("BAD", json!([]))]));
    z["entries"][0]["findings"] = json!([{"code": "ID_FORMAT", "severity": "error", "message": "z's own", "file": "x", "line": 1}]);
    let report = report(
        &dir,
        z,
        trace(json!([entry("BAD", json!([]))])),
        "edge_kinds: {}",
        false,
    );
    let mut nodes: Vec<(&str, &str)> = report
        .findings
        .iter()
        .filter(|finding| finding.issue.code == "ID_FORMAT")
        .map(|finding| {
            (
                finding.issue.node_id.as_deref().unwrap(),
                finding.issue.message.as_str(),
            )
        })
        .collect();
    nodes.sort_unstable();
    assert_eq!(nodes.len(), 2, "{nodes:?}");
    assert_eq!(nodes[0].0, "a:BAD");
    assert_eq!(nodes[1], ("z:BAD", "z's own"));
}

// Requirement: program-composition/Standard validators on the composed graph
// Requirement: program-composition/Suppression in program composition

/// A node kind belongs to one source, so its `EMPTY_KIND` follows the orphan
/// rule: the fuse profile's override, else the source's, and the source's
/// `SUPPRESS`. A shared edge kind's takes neither from a source.
#[cfg(feature = "native-adapters")]
#[test]
fn a_node_kind_empty_kind_follows_its_source_profile_unless_the_fuse_overrides() {
    let settings = |source: &str, fuse: &str| {
        let dir = Scratch::new();
        toml_source(
            &dir,
            "a",
            "[[req]]\nid = 'R-1'\n",
            &format!(
                "node_kinds:\n  req: {{id_pattern: '^R-[0-9]+$', orphan_ok: true}}\n  haz: {{id_pattern: '^H-[0-9]+$'}}\nedge_kinds:\n  controls: {{allowed: [[req, haz]]}}\nvalidations:\n{source}"
            ),
            "",
        );
        let (_, fused) = toml_fuse(&dir, &["a"], &format!("edge_kinds: {{}}\n{fuse}"));
        let of = |message: &str| {
            let found = with_code(&fused, "EMPTY_KIND");
            let finding = found
                .iter()
                .find(|finding| finding["message"] == message)
                .unwrap_or_else(|| panic!("{message}: {fused}"));
            (
                finding["severity"].as_str().unwrap().to_string(),
                finding["suppressed"] == true,
            )
        };
        (
            of("node kind 'a:haz' has no node"),
            of("edge kind 'controls' has no edge"),
        )
    };
    let at = |severity: &str, suppressed: bool| (severity.to_string(), suppressed);
    assert_eq!(
        settings("  - EMPTY_KIND: {severity: warning}\n", ""),
        (at("warning", false), at("info", false))
    );
    assert_eq!(
        settings(
            "  - EMPTY_KIND: {severity: warning}\n",
            "validations:\n  - EMPTY_KIND: {severity: hint}\n"
        ),
        (at("hint", false), at("hint", false))
    );
    assert_eq!(
        settings("  - SUPPRESS: {code: EMPTY_KIND}\n", ""),
        (at("info", true), at("info", false))
    );
}

// Requirement: program-composition/Suppression in program composition

/// A fuse `SUPPRESS` adds suppression and never clears it: an entry whose
/// `node_ids` cannot match a node-less `EMPTY_KIND` leaves the source's
/// suppression standing, and is itself reported unused.
#[cfg(feature = "native-adapters")]
#[test]
fn a_fuse_suppress_that_matches_nothing_keeps_the_source_suppression() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n",
        "node_kinds:\n  req: {id_pattern: '^R-[0-9]+$', orphan_ok: true}\n  haz: {id_pattern: '^H-[0-9]+$'}\nedge_kinds: {}\nvalidations:\n  - EMPTY_KIND: {severity: error}\n  - SUPPRESS: {code: EMPTY_KIND}\n",
        "",
    );
    let (code, fused) = toml_fuse(
        &dir,
        &["a"],
        "edge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: EMPTY_KIND, node_ids: ['a:R-1']}\n",
    );
    let found = with_code(&fused, "EMPTY_KIND");
    assert_eq!(found.len(), 1, "{fused}");
    assert_eq!(found[0]["message"], "node kind 'a:haz' has no node");
    assert_eq!(found[0]["severity"], "error");
    assert_eq!(found[0]["suppressed"], true, "{fused}");
    assert_eq!(with_code(&fused, "SUPPRESS_UNUSED").len(), 1, "{fused}");
    assert_eq!(code, Some(0), "{fused}");
}

// Requirement: program-composition/Member findings keep their notes

/// Fuse's own judgements carry no source's note: `AMBIGUOUS_CROSS_REF` on a node
/// whose source noted that code, and `CROSS_SOURCE_DUPLICATE_ID`, whose node is a
/// raw ID that may spell another source's qualified one.
#[test]
fn a_source_note_does_not_reach_a_finding_only_fuse_makes() {
    let dir = Scratch::new();
    for name in ["a.yaml", "b.yaml", "c.yaml"] {
        dir.write(name, SOURCE_PROFILE);
    }
    let manifest = load_manifest(&dir.write("fuse.yaml", "manifest_version: '1.0.0'\nname: sample\nversion: '2.0.0'\nfuse_profile: profile.yaml\nsources:\n  - {name: a, profile: a.yaml, adapter: a, target: .}\n  - {name: b, profile: b.yaml, adapter: b, target: .}\n  - {name: c, profile: c.yaml, adapter: c, target: .}\n")).unwrap();
    let profile = load_fuse_profile(&dir.write(
        "profile.yaml",
        "edge_kinds:\n  links:\n    allowed: [[a:item, b:item], [a:item, c:item]]\n",
    ))
    .unwrap();
    let noted = |code: &str| json!({"code": code, "severity": "error", "message": "m", "file": "x", "line": 1, "note": {"text": "a's own"}});
    let mut a = trace(json!([entry("X", json!([edge("a:X")]))]));
    a["entries"][0]["findings"] = json!([
        noted("CROSS_SOURCE_DUPLICATE_ID"),
        noted("AMBIGUOUS_CROSS_REF")
    ]);
    let b = trace(json!([entry("a:X", json!([]))]));
    let c = trace(json!([entry("a:X", json!([]))]));
    let report = assemble(
        &manifest,
        &profile,
        vec![
            parse_trace(&a.to_string(), "a").unwrap(),
            parse_trace(&b.to_string(), "b").unwrap(),
            parse_trace(&c.to_string(), "c").unwrap(),
        ],
        false,
    )
    .unwrap();
    for code in ["CROSS_SOURCE_DUPLICATE_ID", "AMBIGUOUS_CROSS_REF"] {
        let own: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.issue.code == code && f.issue.message != "m")
            .collect();
        assert_eq!(own.len(), 1, "{code}: {:?}", report.findings);
        assert_eq!(own[0].issue.node_id.as_deref(), Some("a:X"), "{code}");
        assert_eq!(own[0].issue.note, None, "{code}");
    }
}

// Requirement: finding-notes/Notes attach to findings

/// A note reaches every finding with its code and node: the source's own
/// `CONSTRAINT` and each fuse rule's, which stay distinct.
#[cfg(feature = "native-adapters")]
#[test]
fn a_source_note_reaches_each_repeated_fuse_rule_on_its_node() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n",
        "node_kinds:\n  req: {id_pattern: '^R-[0-9]+$', orphan_ok: true}\nedge_kinds: {}\nvalidations:\n  - CONSTRAINT: {kind: req, expect: {owner: {present: true}}}\n",
        "- finding: {code: CONSTRAINT, node: R-1}\n  note: owner pending\n",
    );
    let (_, fused) = toml_fuse(
        &dir,
        &["a"],
        "edge_kinds: {}\nvalidations:\n  - CONSTRAINT: {kind: 'a:req', expect: {owner: {present: true}}}\n  - CONSTRAINT: {kind: 'a:req', expect: {title: {present: true}}}\n",
    );
    let found = at(&fused, "CONSTRAINT", "a:R-1");
    assert_eq!(found.len(), 3, "{fused}");
    assert!(
        found
            .iter()
            .all(|finding| finding["note"]["text"] == "owner pending"),
        "{fused}"
    );
}

// Requirement: program-composition/A source profile that cannot load is a source failure

/// A failed assembly recomputes nothing, but `--strict` still promotes the
/// healthy source's warnings.
#[cfg(feature = "native-adapters")]
#[test]
fn a_failed_assembly_still_promotes_healthy_warnings_under_strict() {
    let dir = Scratch::new();
    toml_source(
        &dir,
        "a",
        "[[req]]\nid = 'R-1'\n",
        &format!("{REQ_HAZ}validations:\n  - EMPTY_KIND: {{severity: warning}}\n"),
        "",
    );
    fs::create_dir(dir.0.join("b")).unwrap();
    dir.write("b.yaml", "name: b\nnode_kinds: [unloadable\n");
    write_fuse(&dir, &["a", "b"], "edge_kinds: {}\n");
    for (strict, severity) in [(false, "warning"), (true, "error")] {
        let mut args = vec!["fuse", "--manifest", "manifest.yaml"];
        if strict {
            args.push("--strict");
        }
        let (code, fused) = run_json(&dir, &args);
        assert_eq!(code, Some(2), "{fused}");
        let found = with_code(&fused, "EMPTY_KIND");
        assert_eq!(found.len(), 2, "{fused}");
        assert!(
            found.iter().all(|finding| finding["severity"] == severity),
            "{fused}"
        );
    }
}

// Requirement: cli/CLI entry point

/// A source whose profile suppresses its parse finding still leaves the fused
/// run's plain output saying that input went unread.
#[cfg(unix)]
#[test]
fn a_source_suppressed_parse_error_leaves_a_partial_input_notice() {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    let source_profile = "name: local\nprofile_version: '1.0.0'\nnode_kinds: {item: {id_pattern: '.*', orphan_ok: true}}\nedge_kinds: {}\nvalidations:\n  - SUPPRESS: {code: PARSE_ERROR}\n";
    dir.write("z.yaml", source_profile);
    dir.write("a.yaml", source_profile);
    for (name, findings) in [
        (
            "z",
            json!([{"code": "PARSE_ERROR", "severity": "error", "message": "m",
                    "provenance": {"file": "x", "line": 4}}]),
        ),
        ("a", json!([])),
    ] {
        let document = json!({"interface_version": "1.2", "findings": findings, "nodes": [
            {"id": name.to_uppercase(), "kind": "item", "attrs": {}, "provenance": {"file": "x", "line": 1}}
        ]});
        executable(
            &dir,
            name,
            &format!("#!/bin/sh\ncat <<'PAYLOAD'\n{document}\nPAYLOAD\n"),
        );
    }
    let run = |format: &str| {
        Command::new(env!("CARGO_BIN_EXE_latt"))
            .args(["fuse", "--manifest"])
            .arg(&manifest)
            .args(["--format", format])
            .output()
            .unwrap()
    };
    let plain = run("plain");
    assert_eq!(plain.status.code(), Some(0), "{plain:?}");
    assert_eq!(
        String::from_utf8_lossy(&plain.stderr),
        "Partial input: the profile suppresses 1 PARSE_ERROR; --format json lists it\n"
    );
    let json = run("json");
    assert_eq!(String::from_utf8_lossy(&json.stderr), "");
}

// Requirement: program-composition/Suppression in program composition

/// Fuse two sources through real `latt trace` runs: `z` declares `ids` with a
/// self-edge each, `a` the same ids with no edges, and the fuse profile
/// redirects `z`'s edges into `a`. Each source profile gives `SUPPRESS_UNUSED`
/// an error severity and a message template, as a source run renders it.
#[cfg(unix)]
fn templated_unused_report(
    ids: &[&str],
    z_suppress: &str,
    a_suppress: &str,
) -> latt_core::types::FuseReport {
    let dir = Scratch::new();
    let manifest = manifest(&dir);
    dir.write(
        "profile.yaml",
        "edge_kinds: {links: {allowed: [[z:item, a:item]]}}\nvalidations:\n  - SUPPRESS: {code: CROSS_SOURCE_DUPLICATE_ID}\n",
    );
    for (name, suppress, edges) in [("z", z_suppress, true), ("a", a_suppress, false)] {
        dir.write(
            &format!("{name}.yaml"),
            &format!(
                "name: local\nprofile_version: '1.0.0'\nnode_kinds: {{item: {{id_pattern: '.*'}}}}\n\
                 edge_kinds: {{links: {{allowed: [[item, item]]}}}}\nvalidations:\n\
                 \x20 - SUPPRESS: {suppress}\n\
                 \x20 - SUPPRESS_UNUSED: {{severity: error, message: 'review suppression: {{message}}'}}\n"
            ),
        );
        let nodes: Vec<Value> = ids
            .iter()
            .map(|id| json!({"id": id, "kind": "item", "attrs": {}, "provenance": {"file": "x", "line": 1}}))
            .collect();
        let links: Vec<Value> = if edges {
            ids.iter()
                .map(|id| json!({"src": id, "tgt": id, "kind": "links", "provenance": {"file": "x", "line": 2}}))
                .collect()
        } else {
            Vec::new()
        };
        let document = json!({"interface_version": "1.2", "nodes": nodes, "edges": links});
        executable(
            &dir,
            name,
            &format!("#!/bin/sh\ncat <<'PAYLOAD'\n{document}\nPAYLOAD\n"),
        );
    }
    latt_core::fuse::fuse(&manifest, Path::new(env!("CARGO_BIN_EXE_latt")), false).unwrap()
}

/// A source's templated `SUPPRESS_UNUSED` is reconciled against the composed
/// graph as its default text is: withdrawn when the composed graph uses the
/// entry, rewritten keeping the template when it uses part of it, and added
/// with the template when the source run used an entry the composed graph does not.
#[cfg(unix)]
#[test]
fn suppress_a_source_message_template_survives_reconciliation() {
    let errors = |report: &latt_core::types::FuseReport| -> Vec<(Option<String>, String)> {
        report
            .findings
            .iter()
            .filter(|f| f.issue.code == "SUPPRESS_UNUSED")
            .inspect(|f| assert_eq!(f.issue.severity, latt_core::types::Severity::Error))
            .map(|f| (f.source.clone(), f.issue.message.clone()))
            .collect()
    };
    let none = "{code: UNTRACED, node_ids: [NONE]}";

    // `z`'s run saw no orphan; the composed graph orphans z:X, which the entry suppresses.
    let used = templated_unused_report(&["X"], "{code: UNREFERENCED}", none);
    assert_eq!(
        errors(&used)
            .iter()
            .filter(|(source, _)| source.as_deref() == Some("z"))
            .count(),
        0,
        "{:?}",
        errors(&used)
    );

    // The composed graph uses X of [X, W]: the rewrite names W and keeps the template.
    let partial = templated_unused_report(&["X"], "{code: UNREFERENCED, node_ids: [X, W]}", none);
    assert!(
        errors(&partial).contains(&(
            Some("z".to_string()),
            "review suppression: SUPPRESS entry for 'UNREFERENCED' matched no finding for \
             node_ids [\"W\"]: 'UNREFERENCED' is a built-in code; findings carried it this \
             run, but none for those node_ids"
                .to_string()
        )),
        "{:?}",
        errors(&partial)
    );

    // `a`'s run used its entry on A; the composed graph gives a:A an incoming edge.
    let added = templated_unused_report(&["A"], none, "{code: UNREFERENCED, node_ids: [A]}");
    assert!(
        errors(&added).contains(&(
            Some("a".to_string()),
            "review suppression: SUPPRESS entry for 'UNREFERENCED' matched no finding for \
             node_ids [\"A\"]: 'UNREFERENCED' is a built-in code; no finding carried it this run"
                .to_string()
        )),
        "{:?}",
        errors(&added)
    );
}

// Requirement: program-composition/Read a fuse manifest
#[test]
fn cli_unreadable_manifest_names_its_cause_once_on_one_line() {
    let dir = Scratch::new();
    let malformed = dir.write("bad.yaml", "manifest_version: [\n");
    let directory = dir.0.join("manifest-dir");
    fs::create_dir(&directory).unwrap();
    for (path, cause, line) in [
        (&malformed, "did not find expected node content", 2),
        (&directory, "Is a directory", 0),
    ] {
        let plain = cli(path, "plain");
        assert_eq!(plain.status.code(), Some(2));
        let text = String::from_utf8_lossy(&plain.stdout);
        assert_eq!(text.matches(cause).count(), 1, "{text}");
        let json: Value = serde_json::from_slice(&cli(path, "json").stdout).unwrap();
        let findings = json["findings"].as_array().unwrap();
        // The header line, then one line per finding.
        assert_eq!(text.lines().count(), 1 + findings.len(), "{text}");
        let [finding] = findings.as_slice() else {
            panic!("one finding: {findings:?}");
        };
        assert_eq!(finding["code"], "FUSE_ERROR");
        assert_eq!(finding["file"], path.display().to_string());
        assert_eq!(finding["line"], line, "{finding}");
        let message = finding["message"].as_str().unwrap();
        assert!(!message.contains('\n'), "{message}");
        assert!(message.contains(&path.display().to_string()), "{message}");
    }
}
// Requirement: program-composition/Run each source through latt trace
#[cfg(unix)]
#[test]
fn failed_trace_stderr_is_one_line_naming_the_exit_code_once() {
    let dir = Scratch::new();
    let path = manifest(&dir);
    dir.write("profile.yaml", "edge_kinds: {}");
    for name in ["z", "a"] {
        dir.write(
            &format!("{name}.yaml"),
            "name: s\nprofile_version: '1.0.0'\nnode_kinds: {}\nedge_kinds: {}\n",
        );
        stub::write_stub(
            &dir.0.join(name),
            "#!/bin/sh\necho 'first line' >&2\necho 'second line' >&2\nexit 2\n",
        );
    }
    let plain = cli(&path, "plain");
    assert_eq!(plain.status.code(), Some(2));
    let text = String::from_utf8_lossy(&plain.stdout);
    let json: Value = serde_json::from_slice(&cli(&path, "json").stdout).unwrap();
    let findings = json["findings"].as_array().unwrap();
    assert_eq!(text.lines().count(), 1 + findings.len(), "{text}");
    for finding in findings {
        assert_eq!(finding["code"], "SOURCE_FAILURE");
        let message = finding["message"].as_str().unwrap();
        assert!(!message.contains('\n'), "{message}");
        assert!(!message.contains("exit status"), "{message}");
        assert!(message.starts_with("trace exited 2: "), "{message}");
        assert!(message.contains("first line; second line"), "{message}");
    }
}
