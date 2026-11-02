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
    assert_eq!(report.findings.len(), 2);
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
            "message": message, "node_id": null, "severity": "error"}]})
    );
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
